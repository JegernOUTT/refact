//! Owner-service migration boundary. Legacy files are inputs, never a second lifecycle.
//! Import is explicit, bounded and retryable. It neither scans nor deletes user files.
use std::collections::BTreeSet;

use refact_buddy_core::operator_types::{
    ProjectIdentity, RunOutcome, RunStatus, WorkItem, WorkItemState, WorkerIdentity,
    OPERATOR_SCHEMA_VERSION_V1,
};

use super::artifacts::{ArtifactDraft, ArtifactMaterialization};
use super::operator_store::{OperatorStore, OperatorStoreError, Stored};
use super::receipts::BuddyReceipt;
use super::types::{
    BuddyAction, BuddyActivity, BuddyDraft, BuddyOpportunity, BuddyOpportunityKind,
    BuddyOpportunityLinks, BuddyPriority, OpportunityStatus,
};
use super::work_items::{stable_hash, SubjectKey, WorkItemDraft};

pub const MAX_MIGRATION_BATCH: usize = 128;
/// Namespaces for retained legacy history. These are read-only projections of
/// files the migration never edits or deletes.
pub const LEGACY_RECEIPT_NAMESPACE: &str = "legacy_receipt";
pub const LEGACY_ACTIVITY_NAMESPACE: &str = "legacy_activity";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationProgress {
    pub imported: usize,
    pub already_imported: usize,
}

impl MigrationProgress {
    fn record(&mut self, imported: bool) {
        if imported {
            self.imported += 1;
        } else {
            self.already_imported += 1;
        }
    }

    fn merge(&mut self, other: MigrationProgress) {
        self.imported += other.imported;
        self.already_imported += other.already_imported;
    }
}

/// Result of one bounded migration pass. `remaining` is what a later pass must
/// still carry; nothing is dropped and nothing is retried destructively.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChunkedProgress {
    pub progress: MigrationProgress,
    pub consumed_drafts: usize,
    pub consumed_opportunities: usize,
    pub remaining: usize,
}

/// Keep offsets tied to one source snapshot. A changing feed cannot reorder or
/// starve the tail of an import. Empty reads are retryable, not completion.
pub(crate) struct HistorySeed<T> {
    inputs: Vec<T>,
    pub(crate) cursor: usize,
}

impl<T> Default for HistorySeed<T> {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            cursor: 0,
        }
    }
}

impl<T: Clone> HistorySeed<T> {
    pub(crate) fn needs_source(&self) -> bool {
        self.inputs.is_empty()
    }

    pub(crate) fn stage(&mut self, inputs: Vec<T>) {
        if self.needs_source() {
            self.inputs = inputs;
        }
    }

    pub(crate) fn page(&self, limit: usize) -> Vec<T> {
        self.inputs
            .iter()
            .skip(self.cursor)
            .take(limit.min(MAX_MIGRATION_BATCH))
            .cloned()
            .collect()
    }

    pub(crate) fn acknowledge(&mut self, start: usize, consumed: usize) {
        if self.cursor == start {
            self.cursor = start.saturating_add(consumed).min(self.inputs.len());
        }
    }

    pub(crate) fn complete(&self) -> bool {
        !self.inputs.is_empty() && self.cursor == self.inputs.len()
    }
}

/// Envelope for retained legacy history. `imported_at` is derived from the
/// source record so a repeated import is byte-identical and does not churn
/// revisions; a materially changed source appends a new revision instead.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyDocument<T> {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub source: String,
    pub imported_at: String,
    pub value: T,
}

/// Receipt history without the pre-image. Recovery bytes keep their bounded
/// legacy retention; copying them into the durable store would silently extend
/// the lifetime of sensitive content this migration is not authorized to move.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyReceiptRecord {
    pub id: String,
    pub action_kind: String,
    pub target_path: String,
    pub created_at: String,
    pub undone: bool,
    pub undone_at: Option<String>,
    pub pre_image_digest: Option<String>,
    pub pre_image_retained_in_legacy_file: bool,
}

impl LegacyReceiptRecord {
    pub fn from_receipt(receipt: &BuddyReceipt) -> Self {
        Self {
            id: receipt.id.clone(),
            action_kind: super::actor::redact_sensitive(&receipt.action_kind),
            target_path: super::actor::redact_sensitive(&receipt.target_path),
            created_at: receipt.created_at.to_rfc3339(),
            undone: receipt.undone,
            undone_at: receipt.undone_at.map(|time| time.to_rfc3339()),
            pre_image_digest: receipt
                .pre_image
                .as_deref()
                .map(|pre_image| stable_hash(&[pre_image])),
            pre_image_retained_in_legacy_file: receipt.pre_image.is_some(),
        }
    }

    /// Compatibility projection. `pre_image` is intentionally absent: recovery
    /// runs against the legacy receipt file, not against this history record.
    pub fn project_receipt(&self) -> Option<BuddyReceipt> {
        Some(BuddyReceipt {
            id: self.id.clone(),
            action_kind: self.action_kind.clone(),
            target_path: self.target_path.clone(),
            pre_image: None,
            created_at: chrono::DateTime::parse_from_rfc3339(&self.created_at)
                .ok()?
                .with_timezone(&chrono::Utc),
            undone: self.undone,
            undone_at: self
                .undone_at
                .as_deref()
                .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                .map(|time| time.with_timezone(&chrono::Utc)),
        })
    }
}

/// Run only from the owning service after checking OperatorFeatures.storage.
/// An unchanged retry never rewrites work/read/snooze. Material source changes append revisions.
/// Each record commits atomically; failure leaves the old source and prior commits intact.
pub fn migrate_batch(
    store: &mut OperatorStore,
    worker: &WorkerIdentity,
    opportunities: &[BuddyOpportunity],
    drafts: &[BuddyDraft],
) -> Result<MigrationProgress, OperatorStoreError> {
    if opportunities.len().saturating_add(drafts.len()) > MAX_MIGRATION_BATCH {
        return Err(OperatorStoreError::Conflict(
            "legacy migration batch exceeds 128 records",
        ));
    }
    let mut progress = MigrationProgress::default();
    // Draft first: a card pointing at that editor shares this durable proposal,
    // rather than creating a second visible piece of work.
    for draft in drafts {
        let body = format!("{}\n\n{}", draft.explanation, draft.yaml_or_json);
        let imported = import_record(
            store,
            worker,
            "legacy_draft",
            &draft.id,
            &draft.title,
            &body,
            &draft.created_at.to_rfc3339(),
            WorkItemState::NeedsAttention,
        )?;
        progress.record(imported);
    }
    for opportunity in opportunities {
        // A legacy Completed flag is not independently verified completion evidence.
        let state = match opportunity.status {
            // Hiding or expiring a card never proves the underlying work cancelled.
            OpportunityStatus::Dismissed
            | OpportunityStatus::Expired
            | OpportunityStatus::Accepted => WorkItemState::Watching,
            OpportunityStatus::Completed => WorkItemState::Delivered,
            OpportunityStatus::New | OpportunityStatus::Shown => WorkItemState::NeedsAttention,
        };
        let actions = serde_json::to_string_pretty(&opportunity.proposed_actions)?;
        let body = format!(
            "{}\n\nLegacy proposed actions (reference only; not execution authority):\n{}",
            opportunity.summary, actions
        );
        let linked_draft = opportunity
            .proposed_actions
            .iter()
            .find_map(action_draft_id);
        // The editor and its opportunity name the same proposal, not two work items.
        // Import draft content below. A dangling editor link still retains its proposal.
        if linked_draft.is_some_and(|id| drafts.iter().any(|draft| draft.id == id)) {
            continue;
        }
        let producer_id = if opportunity.cooldown_key.trim().is_empty() {
            opportunity.id.clone()
        } else {
            format!(
                "producer:{}",
                super::work_items::stable_hash(&[&opportunity.cooldown_key])
            )
        };
        let (kind, id) = linked_draft
            .map(|id| ("legacy_draft", id))
            .unwrap_or(("legacy_opportunity", producer_id.as_str()));
        if linked_draft.is_some() {
            let subject = SubjectKey::new(store.identity(), kind, id, "legacy-import-v1", "1")
                .map_err(|_| OperatorStoreError::Conflict("legacy source identity missing"))?;
            if store.get_work_item(&subject.work_item_id())?.is_some() {
                progress.already_imported += 1;
                continue;
            }
        }
        let imported = import_record(
            store,
            worker,
            kind,
            id,
            &opportunity.summary,
            &body,
            &opportunity.created_at.to_rfc3339(),
            state,
        )?;
        progress.record(imported);
    }
    Ok(progress)
}

/// Migrate an arbitrarily long legacy history in bounded passes. A historical
/// backlog larger than one batch is a reason to paginate, not to refuse the
/// user's data. Drafts drain first so a card can never import a placeholder
/// before its proposal exists; the caller retries with the remainder.
pub fn migrate_chunk(
    store: &mut OperatorStore,
    worker: &WorkerIdentity,
    opportunities: &[BuddyOpportunity],
    drafts: &[BuddyDraft],
) -> Result<ChunkedProgress, OperatorStoreError> {
    let (chunk, error) = migrate_chunk_partial(store, worker, opportunities, drafts);
    match error {
        Some(error) => Err(error),
        None => Ok(chunk),
    }
}

/// Return the exact committed prefix even when a later record fails. A retry
/// can start at that prefix; a process restart can safely replay from zero.
pub fn migrate_chunk_partial(
    store: &mut OperatorStore,
    worker: &WorkerIdentity,
    opportunities: &[BuddyOpportunity],
    drafts: &[BuddyDraft],
) -> (ChunkedProgress, Option<OperatorStoreError>) {
    let mut chunk = ChunkedProgress::default();
    let mut failure = None;
    for draft in drafts.iter().take(MAX_MIGRATION_BATCH) {
        match migrate_batch(store, worker, &[], std::slice::from_ref(draft)) {
            Ok(progress) => {
                chunk.progress.merge(progress);
                chunk.consumed_drafts += 1;
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    if failure.is_none() && chunk.consumed_drafts == drafts.len() {
        for opportunity in opportunities
            .iter()
            .take(MAX_MIGRATION_BATCH - chunk.consumed_drafts)
        {
            match migrate_batch(store, worker, std::slice::from_ref(opportunity), &[]) {
                Ok(progress) => {
                    chunk.progress.merge(progress);
                    chunk.consumed_opportunities += 1;
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
    }
    chunk.remaining =
        drafts.len() - chunk.consumed_drafts + opportunities.len() - chunk.consumed_opportunities;
    (chunk, failure)
}

/// Read retained history without the legacy UI loader's error-to-empty and
/// retention filtering. Missing is not proof that migration finished.
pub async fn load_receipt_history(root: &std::path::Path) -> Result<Vec<BuddyReceipt>, String> {
    use tokio::io::AsyncReadExt;
    let file = match tokio::fs::File::open(super::receipts::receipts_path(root)).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err("legacy receipt history could not be read; import remains pending".into())
        }
    };
    const MAX_SEED_BYTES: u64 = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(MAX_SEED_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "legacy receipt history read failed; import remains pending")?;
    if bytes.len() as u64 > MAX_SEED_BYTES {
        return Err("legacy receipt history exceeds import byte limit; source retained".into());
    }
    let mut receipts: Vec<BuddyReceipt> = serde_json::from_slice(&bytes)
        .map_err(|_| "legacy receipt history is invalid; source retained")?;
    receipts.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    Ok(receipts)
}

/// A bounded history pass reports each committed prefix, including on failure.
/// One bad source must not starve the other independent observation feed.
pub(crate) fn migrate_history_chunk(
    store: &mut OperatorStore,
    receipts: &[BuddyReceipt],
    activities: &[BuddyActivity],
) -> (usize, usize, Option<String>) {
    let mut receipt_count = 0;
    let mut activity_count = 0;
    let mut error = None;
    for receipt in receipts.iter().take(MAX_MIGRATION_BATCH / 2) {
        match migrate_receipts(store, std::slice::from_ref(receipt)) {
            Ok(_) => receipt_count += 1,
            Err(err) => {
                error = Some(err.to_string());
                break;
            }
        }
    }
    for activity in activities.iter().take(MAX_MIGRATION_BATCH / 2) {
        match migrate_activities(store, std::slice::from_ref(activity)) {
            Ok(_) => activity_count += 1,
            Err(err) => {
                error = Some(err.to_string());
                break;
            }
        }
    }
    (receipt_count, activity_count, error)
}

fn legacy_document_id(namespace: &str, source_id: &str) -> String {
    format!("{namespace}:{}", stable_hash(&[source_id]))
}

/// Import retained action receipts as durable history. Receipts are evidence of
/// past effects, never a second execution authority: nothing here can be
/// replayed or undone from the Work store, and the legacy file stays intact.
pub fn migrate_receipts(
    store: &mut OperatorStore,
    receipts: &[BuddyReceipt],
) -> Result<MigrationProgress, OperatorStoreError> {
    if receipts.len() > MAX_MIGRATION_BATCH {
        return Err(OperatorStoreError::Conflict(
            "legacy receipt batch exceeds 128 records",
        ));
    }
    let project = store.identity().clone();
    let mut progress = MigrationProgress::default();
    let mut seen = BTreeSet::new();
    for receipt in receipts {
        if receipt.id.trim().is_empty() {
            return Err(OperatorStoreError::InvalidInput(
                "legacy receipt identity missing",
            ));
        }
        if !seen.insert(receipt.id.as_str()) {
            continue;
        }
        let record = LegacyReceiptRecord::from_receipt(receipt);
        let id = legacy_document_id(LEGACY_RECEIPT_NAMESPACE, &receipt.id);
        let document = LegacyDocument {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            project: project.clone(),
            source: "legacy_receipts_file".into(),
            imported_at: record.created_at.clone(),
            value: record,
        };
        // The store folds a byte-identical payload into the existing revision,
        // so a repeated import is a no-op and a material change appends history.
        progress.record(import_document(
            store,
            LEGACY_RECEIPT_NAMESPACE,
            &id,
            &document,
        )?);
    }
    Ok(progress)
}

/// Import the retained activity feed as durable history. Activity entries are
/// observations; importing one never creates work, resolves work, or claims a
/// verified effect.
pub fn migrate_activities(
    store: &mut OperatorStore,
    activities: &[BuddyActivity],
) -> Result<MigrationProgress, OperatorStoreError> {
    if activities.len() > MAX_MIGRATION_BATCH {
        return Err(OperatorStoreError::Conflict(
            "legacy activity batch exceeds 128 records",
        ));
    }
    let project = store.identity().clone();
    let mut progress = MigrationProgress::default();
    let mut seen = BTreeSet::new();
    for activity in activities {
        let mut activity = activity.clone();
        activity.title = super::actor::redact_sensitive(&activity.title);
        activity.description = super::actor::redact_sensitive(&activity.description);
        activity.failure_summary = activity
            .failure_summary
            .as_deref()
            .map(super::actor::redact_sensitive);
        if activity.timestamp.trim().is_empty() || activity.activity_type.trim().is_empty() {
            return Err(OperatorStoreError::InvalidInput(
                "legacy activity identity missing",
            ));
        }
        // Legacy activities carry no id, so identity is their exact content.
        // Re-importing the same feed is therefore a no-op, and a genuinely new
        // entry never overwrites an older one.
        let source_id = stable_hash(&[
            activity.timestamp.trim(),
            activity.activity_type.trim(),
            activity.title.trim(),
            activity.description.trim(),
            activity.chat_id.as_deref().unwrap_or(""),
        ]);
        if !seen.insert(source_id.clone()) {
            continue;
        }
        let id = legacy_document_id(LEGACY_ACTIVITY_NAMESPACE, &source_id);
        let document = LegacyDocument {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            project: project.clone(),
            source: "legacy_activity_feed".into(),
            imported_at: activity.timestamp.clone(),
            value: activity,
        };
        progress.record(import_document(
            store,
            LEGACY_ACTIVITY_NAMESPACE,
            &id,
            &document,
        )?);
    }
    Ok(progress)
}

fn import_document<T: serde::Serialize + serde::de::DeserializeOwned + Clone>(
    store: &mut OperatorStore,
    namespace: &str,
    id: &str,
    document: &T,
) -> Result<bool, OperatorStoreError> {
    let current = store.get_document::<serde_json::Value>(namespace, id)?;
    let value = serde_json::to_value(document)?;
    if current
        .as_ref()
        .is_some_and(|current| current.value == value)
    {
        return Ok(false);
    }
    store.put_document(
        namespace,
        id,
        document,
        current.map(|stored| stored.revision),
    )?;
    Ok(true)
}

/// Bounded receipt history page from the Work authority. Pre-images are not
/// returned; only the legacy receipt file can still restore them.
pub fn receipt_history_page(
    store: &OperatorStore,
    after: Option<u64>,
    limit: usize,
) -> Result<(Vec<BuddyReceipt>, Option<u64>), OperatorStoreError> {
    let page = store.list_documents::<LegacyDocument<LegacyReceiptRecord>>(
        LEGACY_RECEIPT_NAMESPACE,
        after,
        limit,
    )?;
    let mut receipts = Vec::new();
    for stored in page.entries {
        if let Some(receipt) = stored.value.value.project_receipt() {
            receipts.push(receipt);
        }
    }
    receipts.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok((receipts, page.next_after))
}

/// Bounded latest history for a briefing. Return chronological display order;
/// this is deliberately separate from the public change-sequence cursor API.
pub(crate) fn latest_receipts(
    store: &OperatorStore,
    limit: usize,
) -> Result<Vec<BuddyReceipt>, OperatorStoreError> {
    let mut receipts: Vec<_> = store
        .latest_receipt_documents::<LegacyDocument<LegacyReceiptRecord>>(limit)?
        .into_iter()
        .filter_map(|stored| stored.value.value.project_receipt())
        .collect();
    receipts.reverse();
    Ok(receipts)
}

/// Bounded activity history page from the Work authority.
pub fn activity_history_page(
    store: &OperatorStore,
    after: Option<u64>,
    limit: usize,
) -> Result<(Vec<BuddyActivity>, Option<u64>), OperatorStoreError> {
    let page = store.list_documents::<LegacyDocument<BuddyActivity>>(
        LEGACY_ACTIVITY_NAMESPACE,
        after,
        limit,
    )?;
    let mut activities: Vec<BuddyActivity> = page
        .entries
        .into_iter()
        .map(|stored| stored.value.value)
        .collect();
    activities.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok((activities, page.next_after))
}

fn action_draft_id(action: &BuddyAction) -> Option<&str> {
    match action {
        BuddyAction::DraftSkill { draft_id, .. }
        | BuddyAction::DraftCommand { draft_id, .. }
        | BuddyAction::DraftDelegate { draft_id, .. }
        | BuddyAction::DraftMode { draft_id, .. }
        | BuddyAction::ApplyConfigPatch { draft_id, .. } => {
            (!draft_id.trim().is_empty()).then_some(draft_id.as_str())
        }
        _ => None,
    }
}

fn import_record(
    store: &mut OperatorStore,
    worker: &WorkerIdentity,
    kind: &str,
    id: &str,
    title: &str,
    body: &str,
    created_at: &str,
    state: WorkItemState,
) -> Result<bool, OperatorStoreError> {
    let project = store.identity().clone();
    let subject = SubjectKey::new(&project, kind, id, "legacy-import-v1", "1")
        .map_err(|_| OperatorStoreError::Conflict("legacy source identity missing"))?;
    let current = store.get_work_item(&subject.work_item_id())?;
    let current_artifact = store.get_artifact_revision(&subject.artifact_id(), None)?;
    let title = super::actor::redact_sensitive(title);
    let body = super::actor::redact_sensitive(body);
    if current_artifact.as_ref().is_some_and(|artifact| {
        artifact.value.title == title.trim() && artifact.value.body == body.trim()
    }) {
        return Ok(false);
    }
    let revision = current_artifact
        .as_ref()
        .map_or(1, |artifact| artifact.value.revision.saturating_add(1));
    let outcome = RunOutcome {
        schema_version: 1,
        run_id: format!("legacy-import:{}:{revision}", subject.stable_hash()),
        work_id: subject.work_item_id(),
        parent_run_id: None,
        project: project.clone(),
        worker: worker.clone(),
        origin: "legacy_import".into(),
        workflow: None,
        input_fingerprint: super::work_items::stable_hash(&[&title, &body]),
        coverage: "Legacy proposal import only; effects and conclusions are unverified".into(),
        started_at: created_at.into(),
        ended_at: Some(created_at.into()),
        status: RunStatus::Inconclusive,
        evidence: vec![],
        effects: vec![],
        failure_class: None,
    };
    let mut work = WorkItemDraft {
        project,
        subject: subject.clone(),
        kind: kind.into(),
        title: title.clone(),
        state,
        native_sources: vec![],
        last_material_change_at: created_at.into(),
    }
    .build()
    .map_err(|_| OperatorStoreError::Conflict("invalid legacy work item"))?;
    let artifact = ArtifactDraft {
        subject,
        source_run_ids: vec![outcome.run_id.clone()],
        title,
        conclusion: "Imported legacy proposal; no verified effects claimed.".into(),
        delta: String::new(),
        body,
        uncertainty: Some("Legacy content is untrusted reference material, not action authorization. Editor expiry does not resolve this proposal.".into()),
        proposed_action_ids: vec![],
        evidence: vec![],
    }.materialize(std::slice::from_ref(&outcome), current_artifact.as_ref().map(|record| &record.value))
        .map_err(|_| OperatorStoreError::Conflict("legacy artifact failed bounded content validation"))?;
    let ArtifactMaterialization::Revision(mut artifact) = artifact else {
        return Err(OperatorStoreError::Conflict(
            "initial legacy artifact unexpectedly folded",
        ));
    };
    artifact.provenance.contains_untrusted_source = true;
    work.current_artifact_id = Some(artifact.artifact_id.clone());
    work.current_artifact_revision = Some(artifact.revision);
    if let Some(current) = &current {
        work.state = current.value.state.clone();
        work.native_sources = current.value.native_sources.clone();
    }
    store.append_outcome_work_item_artifact(
        &outcome,
        &work,
        current.as_ref().map(|record| record.revision),
        Some(&artifact),
    )?;
    Ok(true)
}

/// Compatibility projection; never copies legacy status/actions back over Work authority.
/// Mutation clients must use Work Desk's revision-bound commands instead.
pub fn project_opportunity(work: &Stored<WorkItem>, read: bool) -> BuddyOpportunity {
    let created_at = chrono::DateTime::parse_from_rfc3339(&work.value.last_material_change_at)
        .map(|time| time.with_timezone(&chrono::Utc))
        .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH);
    BuddyOpportunity {
        id: work.value.id.clone(),
        kind: BuddyOpportunityKind::JobFinding,
        summary: work.value.title.clone(),
        priority: BuddyPriority::Normal,
        confidence: 0.5,
        fact_keys: vec![],
        cooldown_key: work.value.id.clone(),
        cooldown_secs: 0,
        status: match work.value.state {
            WorkItemState::Cancelled => OpportunityStatus::Dismissed,
            WorkItemState::Delivered => OpportunityStatus::Completed,
            WorkItemState::Working | WorkItemState::Watching => OpportunityStatus::Accepted,
            _ if read => OpportunityStatus::Shown,
            _ => OpportunityStatus::New,
        },
        proposed_actions: vec![],
        humor: None,
        humor_allowed: false,
        related: BuddyOpportunityLinks::default(),
        created_at,
        expires_at: chrono::DateTime::<chrono::Utc>::MAX_UTC,
        resolved_at: None,
    }
}

pub fn projection_page(
    store: &OperatorStore,
    after: Option<u64>,
    limit: usize,
) -> Result<(Vec<BuddyOpportunity>, Option<u64>), OperatorStoreError> {
    let page = store.list_work_items(after, limit)?;
    let mut opportunities = Vec::new();
    for work in page.entries {
        if store.get_work_snooze(&work.value.id)?.is_some() {
            continue;
        }
        let read = store
            .get_read_state(&work.value.id)?
            .map(|read| {
                Some(read.last_read_revision) == work.value.current_artifact_revision
                    && Some(read.artifact_id.as_str()) == work.value.current_artifact_id.as_deref()
            })
            .unwrap_or(false);
        opportunities.push(project_opportunity(&work, read));
    }
    Ok((opportunities, page.next_after))
}

#[cfg(test)]
mod migration_regressions {
    use super::*;

    fn identity(root: &std::path::Path) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "migration-fixture".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        }
    }
    fn worker() -> WorkerIdentity {
        WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        }
    }
    fn opportunity(index: usize) -> BuddyOpportunity {
        serde_json::from_value(serde_json::json!({
            "id": format!("card-{index:04}"), "kind": "job_finding",
            "summary": format!("Review proposal {index}"), "priority": "normal", "confidence": 0.5,
            "fact_keys": [], "cooldown_key": format!("source-{index}"), "cooldown_secs": 0,
            "status": "new", "proposed_actions": [{"kind": "dismiss"}],
            "humor": null, "humor_allowed": false, "related": {},
            "created_at": "2026-01-01T00:00:00Z", "expires_at": "2026-01-02T00:00:00Z", "resolved_at": null
        })).unwrap()
    }
    fn receipt(index: usize) -> BuddyReceipt {
        BuddyReceipt {
            id: format!("receipt-{index}"),
            action_kind: "config_patch".into(),
            target_path: ".refact/example.yaml".into(),
            pre_image: Some("retained-private-content".into()),
            created_at: chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            undone: false,
            undone_at: None,
        }
    }
    fn activity(index: usize) -> BuddyActivity {
        BuddyActivity {
            icon: "history".into(),
            title: format!("Action {index}"),
            description: "Observed only".into(),
            timestamp: "2020-01-01T00:00:00Z".into(),
            activity_type: "action".into(),
            chat_id: None,
            failure_category: None,
            failure_summary: None,
        }
    }

    #[test]
    fn backlog_over_128_pages_exactly_and_restarts_without_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let project = identity(root.path());
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        let inputs: Vec<_> = (0..273).map(opportunity).collect();
        let mut cursor = 0;
        while cursor < inputs.len() {
            let chunk = migrate_chunk(&mut store, &worker(), &inputs[cursor..], &[]).unwrap();
            assert!(chunk.consumed_opportunities > 0 && chunk.consumed_opportunities <= 128);
            cursor += chunk.consumed_opportunities;
        }
        drop(store);
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        for page in inputs.chunks(128) {
            assert_eq!(
                migrate_batch(&mut store, &worker(), page, &[])
                    .unwrap()
                    .imported,
                0
            );
        }
        let mut after = None;
        let mut ids = BTreeSet::new();
        loop {
            let page = store.list_work_items(after, 31).unwrap();
            for entry in page.entries {
                assert!(ids.insert(entry.value.id));
            }
            after = page.next_after;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 273);
    }

    #[test]
    fn retained_actions_remain_reference_only_and_do_not_duplicate() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::open(root.path(), &identity(root.path())).unwrap();
        let input = opportunity(1);
        migrate_batch(&mut store, &worker(), &[input.clone()], &[]).unwrap();
        let work = store.list_work_items(None, 10).unwrap().entries.remove(0);
        let artifact = store
            .get_artifact_revision(work.value.current_artifact_id.as_ref().unwrap(), None)
            .unwrap()
            .unwrap();
        assert!(artifact.value.body.contains("dismiss"));
        assert!(artifact.value.proposed_action_ids.is_empty());
        assert!(project_opportunity(&work, false)
            .proposed_actions
            .is_empty());
        assert_eq!(
            migrate_batch(&mut store, &worker(), &[input], &[])
                .unwrap()
                .already_imported,
            1
        );
        assert_eq!(store.list_work_items(None, 10).unwrap().entries.len(), 1);
    }

    #[test]
    fn briefing_latest_receipts_exceed_first_page_and_repeats_fold() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::open(root.path(), &identity(root.path())).unwrap();
        let receipts: Vec<_> = (0..259)
            .map(|index| {
                let mut input = receipt(index);
                input.created_at += chrono::Duration::seconds(index as i64);
                input
            })
            .collect();
        // Import newest first to distinguish source recency from change sequence.
        for input in receipts.iter().rev() {
            migrate_receipts(&mut store, &[input.clone()]).unwrap();
        }
        let latest = latest_receipts(&store, 10).unwrap();
        assert_eq!(
            latest.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            receipts[249..]
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        );
        let mut old = receipts[0].clone();
        old.undone = true;
        migrate_receipts(&mut store, &[old.clone()]).unwrap();
        assert_eq!(
            migrate_receipts(&mut store, &[old])
                .unwrap()
                .already_imported,
            1
        );
        assert_eq!(
            latest_receipts(&store, 10).unwrap().last().unwrap().id,
            "receipt-258"
        );
        assert!(latest_receipts(&store, 0).is_err());
        assert!(latest_receipts(&store, 201).is_err());
        let mut after = None;
        let mut ids = BTreeSet::new();
        loop {
            let (page, next) = receipt_history_page(&store, after, 37).unwrap();
            for receipt in page {
                assert!(ids.insert(receipt.id));
            }
            after = next;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 259);
    }

    #[test]
    fn repeated_history_import_is_exact_and_has_no_recovery_bytes() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::open(root.path(), &identity(root.path())).unwrap();
        let receipts: Vec<_> = (0..259).map(receipt).collect();
        let activities: Vec<_> = (0..259).map(activity).collect();
        for repeat in 0..2 {
            for page in receipts.chunks(128) {
                assert_eq!(
                    migrate_receipts(&mut store, page).unwrap().imported,
                    if repeat == 0 { page.len() } else { 0 }
                );
            }
            for page in activities.chunks(128) {
                assert_eq!(
                    migrate_activities(&mut store, page).unwrap().imported,
                    if repeat == 0 { page.len() } else { 0 }
                );
            }
        }
        let mut after = None;
        let mut ids = BTreeSet::new();
        loop {
            let (page, next) = receipt_history_page(&store, after, 29).unwrap();
            for receipt in page {
                assert!(receipt.pre_image.is_none());
                assert!(ids.insert(receipt.id));
            }
            after = next;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 259);
        let mut after = None;
        let mut count = 0;
        loop {
            let (page, next) = activity_history_page(&store, after, 29).unwrap();
            count += page.len();
            after = next;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(count, 259);
        let doc = store
            .get_document::<serde_json::Value>(
                LEGACY_RECEIPT_NAMESPACE,
                &legacy_document_id(LEGACY_RECEIPT_NAMESPACE, "receipt-0"),
            )
            .unwrap()
            .unwrap();
        assert_eq!(doc.revision, 1);
        assert!(!doc.value.to_string().contains("retained-private-content"));
    }
}
