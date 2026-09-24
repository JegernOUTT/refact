use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

use chrono::Utc;
use refact_buddy_core::operator_types::{
    ActionExecution, ArtifactRevision, EvidenceRef, ProjectIdentity, ReadState, RunOutcome,
    RunStatus, WorkItem, WorkItemSeverity, WorkSnooze, WorkerIdentity, OPERATOR_SCHEMA_VERSION_V1,
};
use refact_buddy_core::workflows::{
    housekeeping_blocked_report, workflow_visibility, WorkflowFailureCategory,
    HOUSEKEEPING_BLOCK_THRESHOLD,
};

use super::operator_store::{OperatorStore, OperatorStoreError, Page, Stored};
use super::artifacts::{ArtifactDraft, ArtifactMaterialization};
use super::work_items::{problem_summary_from_title, SubjectKey, WorkItemDraft};

pub const MAX_OUTCOME_EVIDENCE: usize = 12;

/// The one coverage string a housekeeping subject ever uses. Housekeeping does
/// not report per-run coverage because it does not report per-run at all.
pub const HOUSEKEEPING_COVERAGE: &str =
    "background maintenance health only; individual runs are not reported";

/// Subject for the single work item a blocked housekeeping loop is allowed to
/// raise. The `resource_kind` is deliberately *not* `"workflow"`, so this can
/// never adopt, re-key or collide with the work item a user-facing workflow
/// publishes under `SubjectKey::new(project, "workflow", ...)`.
pub(crate) fn housekeeping_subject(
    project: &ProjectIdentity,
    workflow: &str,
) -> Result<SubjectKey, OperatorStoreError> {
    SubjectKey::new(
        project,
        "workflow_housekeeping",
        workflow,
        HOUSEKEEPING_COVERAGE,
        "housekeeping-v1",
    )
    .map_err(|_| OperatorStoreError::Conflict("housekeeping subject is invalid"))
}

/// The subject a housekeeping workflow's per-run rows were published under
/// before this classification existed: the constant coverage and constant
/// fingerprint that collapsed every trajectory onto one row. Reconstructed
/// exactly so the upgrade can retire that row without deleting anything.
pub(crate) fn collapsed_workflow_subject(
    project: &ProjectIdentity,
    workflow: &str,
) -> Result<SubjectKey, OperatorStoreError> {
    SubjectKey::new(
        project,
        "workflow",
        workflow,
        "workflow invocation and terminal result",
        format!("workflow:{workflow}"),
    )
    .map_err(|_| OperatorStoreError::Conflict("collapsed workflow subject is invalid"))
}

/// Subject of R-4's aggregated provider-rate-limit item. Housekeeping consults
/// it rather than raising a second item that says the same thing.
pub(crate) fn provider_rate_limit_subject(
    project: &ProjectIdentity,
) -> Result<SubjectKey, OperatorStoreError> {
    let producer_id = format!(
        "producer:{}",
        super::work_items::stable_hash(&["provider:rate_limited:global"])
    );
    SubjectKey::new(
        project,
        "legacy_opportunity",
        producer_id,
        "legacy-import-v1",
        "1",
    )
    .map_err(|_| OperatorStoreError::Conflict("provider rate-limit subject is invalid"))
}

/// True when R-4 already has one live aggregated provider-rate-limit item on the
/// Desk. Housekeeping references that fact instead of duplicating it.
pub(crate) fn provider_rate_limit_item_is_live(
    store: &OperatorStore,
    project: &ProjectIdentity,
) -> Result<bool, OperatorStoreError> {
    let subject = provider_rate_limit_subject(project)?;
    Ok(store
        .get_work_item(&subject.work_item_id())?
        .is_some_and(|work| {
            matches!(
                work.value.state,
                refact_buddy_core::operator_types::WorkItemState::NeedsAttention
                    | refact_buddy_core::operator_types::WorkItemState::Blocked
            )
        }))
}

/// Move a work item to a terminal state without deleting it, its artifact
/// revisions or its outcomes. Evidence retention is a hard contract.
fn retire_work_item_in_store(
    store: &mut OperatorStore,
    work_item_id: &str,
) -> Result<bool, OperatorStoreError> {
    let Some(current) = store.get_work_item(work_item_id)? else {
        return Ok(false);
    };
    if !matches!(
        current.value.state,
        refact_buddy_core::operator_types::WorkItemState::NeedsAttention
            | refact_buddy_core::operator_types::WorkItemState::Blocked
            | refact_buddy_core::operator_types::WorkItemState::Watching
    ) {
        return Ok(false);
    }
    let mut retired = current.value.clone();
    retired.state = refact_buddy_core::operator_types::WorkItemState::Cancelled;
    store.upsert_work_item(&retired, Some(current.revision))?;
    Ok(true)
}

/// One-shot upgrade step: retire the collapsed per-run row each housekeeping
/// workflow published before it was classified as housekeeping. This is a state
/// transition only — the row, its 1,698 artifact revisions and every outcome
/// linked to it stay readable.
pub(crate) fn retire_collapsed_housekeeping_rows(
    store: &mut OperatorStore,
    project: &ProjectIdentity,
) -> Result<usize, OperatorStoreError> {
    let mut retired = 0;
    for workflow in refact_buddy_core::workflows::HOUSEKEEPING_WORKFLOWS {
        let subject = collapsed_workflow_subject(project, workflow)?;
        if retire_work_item_in_store(store, &subject.work_item_id())? {
            retired += 1;
        }
    }
    Ok(retired)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowOutcomeStart {
    pub run_id: String,
    pub work_id: String,
    pub parent_run_id: Option<String>,
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub origin: String,
    pub workflow: String,
    pub input_fingerprint: String,
    pub coverage: String,
    pub started_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WorkflowTerminal {
    NoChange,
    Partial { failure_class: Option<String> },
    Blocked { failure_class: Option<String> },
    Cancelled { failure_class: Option<String> },
    Failed { failure_class: Option<String> },
    Completed,
    Inconclusive,
}

pub(crate) fn workflow_display_title(workflow: &str) -> String {
    refact_buddy_core::autonomous_workflows::autonomous_workflow_meta(workflow)
        .map(|meta| meta.title.to_string())
        .unwrap_or_else(|| {
            workflow
                .split('_')
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let mut chars = part.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
}

pub(crate) fn workflow_result_title(title: &str, terminal: &WorkflowTerminal) -> String {
    match terminal {
        WorkflowTerminal::NoChange => format!("{title} — no change"),
        WorkflowTerminal::Partial { .. } => format!("{title} is incomplete"),
        WorkflowTerminal::Blocked { .. } => format!("{title} is blocked"),
        WorkflowTerminal::Cancelled { .. } => format!("{title} was cancelled"),
        WorkflowTerminal::Failed { .. } => format!("{title} didn't finish"),
        WorkflowTerminal::Completed => format!("{title} finished"),
        WorkflowTerminal::Inconclusive => format!("{title} assessment"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowOutcomeCheckpoint {
    pub evidence: Vec<EvidenceRef>,
}

impl WorkflowOutcomeCheckpoint {
    pub fn from_evidence(evidence: impl IntoIterator<Item = EvidenceRef>) -> Self {
        let mut evidence: Vec<_> = evidence.into_iter().collect();
        evidence.sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
        evidence.dedup_by(|left, right| left.evidence_id == right.evidence_id);
        evidence.truncate(MAX_OUTCOME_EVIDENCE);
        Self { evidence }
    }
}

pub(crate) fn merge_checkpoint_evidence(
    retained: Vec<EvidenceRef>,
    incoming: Vec<EvidenceRef>,
) -> Result<Vec<EvidenceRef>, OperatorStoreError> {
    let mut merged = retained;
    for evidence in incoming {
        if let Some(existing) = merged
            .iter()
            .find(|existing| existing.evidence_id == evidence.evidence_id)
        {
            if *existing != evidence {
                return Err(OperatorStoreError::Conflict(
                    "checkpoint evidence identity cannot change",
                ));
            }
        } else if merged.len() < MAX_OUTCOME_EVIDENCE {
            merged.push(evidence);
        }
    }
    merged.sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
    Ok(merged)
}

pub fn parent_run_id(parent: Option<&str>) -> Option<String> {
    parent.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

pub fn build_workflow_outcome(
    start: WorkflowOutcomeStart,
    checkpoint: WorkflowOutcomeCheckpoint,
    terminal: WorkflowTerminal,
    ended_at: String,
) -> RunOutcome {
    let (status, failure_class) = match terminal {
        WorkflowTerminal::NoChange => (RunStatus::NoChange, None),
        WorkflowTerminal::Partial { failure_class } => (RunStatus::Partial, failure_class),
        WorkflowTerminal::Blocked { failure_class } => (RunStatus::Blocked, failure_class),
        WorkflowTerminal::Cancelled { failure_class } => (RunStatus::Cancelled, failure_class),
        WorkflowTerminal::Failed { failure_class } => (RunStatus::Failed, failure_class),
        WorkflowTerminal::Completed => (RunStatus::Completed, None),
        WorkflowTerminal::Inconclusive => (RunStatus::Inconclusive, None),
    };
    RunOutcome {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        run_id: start.run_id,
        work_id: start.work_id,
        parent_run_id: start.parent_run_id,
        project: start.project,
        worker: start.worker,
        origin: start.origin,
        workflow: Some(start.workflow),
        input_fingerprint: start.input_fingerprint,
        coverage: start.coverage,
        started_at: start.started_at,
        ended_at: Some(ended_at),
        status,
        evidence: checkpoint.evidence,
        effects: Vec::new(),
        failure_class,
    }
}

pub fn terminal_from_workflow_result(result: &Result<(), String>) -> WorkflowTerminal {
    match result {
        Ok(()) => WorkflowTerminal::Inconclusive,
        Err(error) => {
            let category = refact_buddy_core::workflows::WorkflowFailureCategory::classify(error);
            let failure_class = Some(category.as_str().to_string());
            if category == refact_buddy_core::workflows::WorkflowFailureCategory::Cancelled {
                WorkflowTerminal::Cancelled { failure_class }
            } else {
                WorkflowTerminal::Failed { failure_class }
            }
        }
    }
}

#[derive(Clone)]
pub struct OutcomeRuntime {
    store: Arc<Mutex<Option<OperatorStore>>>,
    project: ProjectIdentity,
    worker: WorkerIdentity,
    closed: Arc<AtomicBool>,
    /// Consecutive failed housekeeping runs per workflow id. In-process only:
    /// a restart starts the count again, but an already published blocked item
    /// survives in the store and still clears on the next success.
    housekeeping_failures: Arc<Mutex<HashMap<String, u32>>>,
}

impl OutcomeRuntime {
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.store, &other.store) && Arc::ptr_eq(&self.closed, &other.closed)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) async fn briefing_inputs(
        &self,
    ) -> Result<
        (
            Vec<super::types::BuddyOpportunity>,
            Vec<super::receipts::BuddyReceipt>,
        ),
        OperatorStoreError,
    > {
        self.read(|store| {
            let (opportunities, _) = super::legacy_work::projection_page(store, None, 200)?;
            let receipts = super::legacy_work::latest_receipts(store, 10)?;
            Ok((opportunities, receipts))
        })
        .await
    }

    pub fn project(&self) -> ProjectIdentity {
        self.project.clone()
    }

    pub fn worker(&self) -> WorkerIdentity {
        self.worker.clone()
    }

    pub async fn with_store<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut OperatorStore) -> Result<T, OperatorStoreError> + Send + 'static,
    ) -> Result<T, OperatorStoreError> {
        let store = self.store.clone();
        let closed = self.closed.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            operation(
                guard
                    .as_mut()
                    .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?,
            )
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("operator repository task failed"))?
    }

    async fn read<T: Send + 'static>(
        &self,
        read: impl FnOnce(&OperatorStore) -> Result<T, OperatorStoreError> + Send + 'static,
    ) -> Result<T, OperatorStoreError> {
        let store = self.store.clone();
        let closed = self.closed.clone();
        tokio::task::spawn_blocking(move || {
            let store = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            read(
                store
                    .as_ref()
                    .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?,
            )
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("outcome reader task failed"))?
    }

    pub async fn list_work_items(
        &self,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<WorkItem>>, OperatorStoreError> {
        self.read(move |store| store.list_work_items(after, limit))
            .await
    }

    pub async fn clear_provider_rate_limit_work(&self) -> Result<(), OperatorStoreError> {
        let project = self.project.clone();
        self.with_store(move |store| {
            let subject = provider_rate_limit_subject(&project)?;
            let Some(mut work) = store.get_work_item(&subject.work_item_id())? else {
                return Ok(());
            };
            if matches!(
                work.value.state,
                refact_buddy_core::operator_types::WorkItemState::NeedsAttention
                    | refact_buddy_core::operator_types::WorkItemState::Blocked
            ) {
                work.value.state = refact_buddy_core::operator_types::WorkItemState::Cancelled;
                store.upsert_work_item(&work.value, Some(work.revision))?;
            }
            Ok(())
        })
        .await
    }

    pub async fn search_work_items(
        &self,
        after: Option<u64>,
        limit: usize,
        view: Option<String>,
        search: Option<String>,
    ) -> Result<Page<Stored<WorkItem>>, OperatorStoreError> {
        self.read(move |store| {
            store.search_work_items(after, limit, view.as_deref(), search.as_deref())
        })
        .await
    }

    pub async fn get_work_item(
        &self,
        id: String,
    ) -> Result<Option<Stored<WorkItem>>, OperatorStoreError> {
        self.read(move |store| store.get_work_item(&id)).await
    }

    pub async fn search_work_with_attention(
        &self,
        after: Option<u64>,
        limit: usize,
        view: Option<String>,
        search: Option<String>,
        attention: Option<String>,
    ) -> Result<Page<Stored<WorkItem>>, OperatorStoreError> {
        self.read(move |store| {
            store.search_work_items_with_attention(
                after,
                limit,
                view.as_deref(),
                search.as_deref(),
                attention.as_deref(),
            )
        })
        .await
    }

    pub async fn get_work_snooze(
        &self,
        id: String,
    ) -> Result<Option<WorkSnooze>, OperatorStoreError> {
        self.read(move |store| store.get_work_snooze(&id)).await
    }

    pub async fn snooze_work(
        &self,
        id: String,
        artifact_id: String,
        revision: u32,
        minutes: Option<u32>,
    ) -> Result<Option<WorkSnooze>, OperatorStoreError> {
        let store = self.store.clone();
        let closed = self.closed.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            guard
                .as_mut()
                .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?
                .snooze_work(&id, &artifact_id, revision, minutes)
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("snooze task failed"))?
    }

    pub async fn get_artifact_revision(
        &self,
        id: String,
        revision: Option<u32>,
    ) -> Result<Option<Stored<ArtifactRevision>>, OperatorStoreError> {
        self.read(move |store| store.get_artifact_revision(&id, revision))
            .await
    }

    pub async fn get_read_state(
        &self,
        id: String,
    ) -> Result<Option<ReadState>, OperatorStoreError> {
        self.read(move |store| store.get_read_state(&id)).await
    }

    pub async fn get_run_outcome(
        &self,
        id: String,
    ) -> Result<Option<Stored<RunOutcome>>, OperatorStoreError> {
        self.read(move |store| store.get_run_outcome(&id)).await
    }

    pub async fn list_work_outcomes(
        &self,
        id: String,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<RunOutcome>>, OperatorStoreError> {
        self.read(move |store| store.list_work_outcomes(&id, after, limit))
            .await
    }

    pub async fn list_artifact_revisions(
        &self,
        id: String,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<ArtifactRevision>>, OperatorStoreError> {
        self.read(move |store| store.list_artifact_revisions(&id, after, limit))
            .await
    }

    pub async fn mark_work_read(
        &self,
        id: String,
        artifact_id: String,
        revision: u32,
    ) -> Result<ReadState, OperatorStoreError> {
        let store = self.store.clone();
        let closed = self.closed.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            guard
                .as_mut()
                .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?
                .mark_work_read(&id, &artifact_id, revision)
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("read-state task failed"))?
    }

    pub async fn get_execution(
        &self,
        id: String,
    ) -> Result<Option<Stored<ActionExecution>>, OperatorStoreError> {
        self.read(move |store| store.get_execution(&id)).await
    }

    pub async fn desk_actions(
        &self,
        work_id: String,
    ) -> Result<Vec<super::desk_actions::DeskAction>, OperatorStoreError> {
        self.read(move |store| super::desk_actions::actions_for_work(store, &work_id))
            .await
    }

    pub async fn execute_desk_action(
        &self,
        work_id: String,
        action_id: String,
        request: super::desk_actions::DeskActionRequest,
    ) -> Result<Stored<ActionExecution>, OperatorStoreError> {
        let store = self.store.clone();
        let closed = self.closed.clone();
        let worker = self.worker.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            let store = guard
                .as_mut()
                .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?;
            super::desk_actions::execute_action(store, worker, &work_id, &action_id, request)
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("action task failed"))?
    }

    pub fn open(
        project_root: &Path,
        project: ProjectIdentity,
        worker: WorkerIdentity,
    ) -> Result<Self, OperatorStoreError> {
        let mut store = OperatorStore::open(project_root, &project)?;
        // Upgrade step, idempotent and non-destructive: the per-run row each
        // housekeeping workflow published before it was classified is moved to a
        // terminal state. Nothing is deleted, so its artifact revisions and all
        // of its outcomes stay readable. A failure here must not keep the user
        // out of their own store.
        match retire_collapsed_housekeeping_rows(&mut store, &project) {
            Ok(0) => {}
            Ok(retired) => tracing::info!(
                retired,
                "buddy: retired collapsed per-run housekeeping rows; their artifacts and outcomes are retained"
            ),
            Err(error) => tracing::warn!(
                %error,
                "buddy: collapsed housekeeping row was not retired; it stays readable and will be retried on the next open"
            ),
        }
        Ok(Self::from_store(store, project, worker))
    }

    pub fn from_store(
        store: OperatorStore,
        project: ProjectIdentity,
        worker: WorkerIdentity,
    ) -> Self {
        Self {
            store: Arc::new(Mutex::new(Some(store))),
            project,
            worker,
            closed: Arc::new(AtomicBool::new(false)),
            housekeeping_failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_workflow(
        &self,
        workflow: impl Into<String>,
        started_at: String,
    ) -> WorkflowOutcomeStart {
        let workflow = workflow.into();
        if workflow_visibility(&workflow).is_housekeeping() {
            // One subject, one coverage, one fingerprint for the whole loop. The
            // per-run identity is `run_id`, which is what keeps every run
            // queryable without giving every run a Desk row.
            let work_id = housekeeping_subject(&self.project, &workflow)
                .map(|subject| subject.work_item_id())
                .unwrap_or_else(|_| format!("workflow_housekeeping:{workflow}"));
            return WorkflowOutcomeStart {
                run_id: uuid::Uuid::new_v4().to_string(),
                work_id,
                parent_run_id: None,
                project: self.project.clone(),
                worker: self.worker.clone(),
                origin: "buddy_workflow".to_string(),
                input_fingerprint: "housekeeping-v1".to_string(),
                coverage: HOUSEKEEPING_COVERAGE.to_string(),
                workflow,
                started_at,
            };
        }
        WorkflowOutcomeStart {
            run_id: uuid::Uuid::new_v4().to_string(),
            work_id: format!("workflow:{workflow}"),
            parent_run_id: None,
            project: self.project.clone(),
            worker: self.worker.clone(),
            origin: "buddy_workflow".to_string(),
            input_fingerprint: format!("workflow:{workflow}"),
            coverage: "workflow invocation and terminal result".to_string(),
            workflow,
            started_at,
        }
    }

    pub async fn begin_workflow(
        &self,
        start: WorkflowOutcomeStart,
    ) -> Result<(), OperatorStoreError> {
        // Housekeeping never persists an active run. An active run exists so
        // crash recovery can publish an "interrupted run" work item, and a
        // housekeeping loop that publishes a Desk row on every engine restart
        // is precisely the noise this classification removes. Its terminal
        // outcome is still recorded on the normal path.
        if workflow_visibility(&start.workflow).is_housekeeping() {
            return Ok(());
        }
        self.write_active(start, None).await
    }

    pub async fn checkpoint_workflow(
        &self,
        start: WorkflowOutcomeStart,
        checkpoint: WorkflowOutcomeCheckpoint,
    ) -> Result<(), OperatorStoreError> {
        self.write_active(start, Some(checkpoint)).await
    }

    async fn write_active(
        &self,
        start: WorkflowOutcomeStart,
        checkpoint: Option<WorkflowOutcomeCheckpoint>,
    ) -> Result<(), OperatorStoreError> {
        let store = self.store.clone();
        let closed = self.closed.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            let store = guard
                .as_mut()
                .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?;
            match checkpoint {
                Some(checkpoint) => store.checkpoint_workflow(&start, checkpoint),
                None => store.begin_workflow(&start),
            }
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("active run writer task failed"))?
    }

    pub async fn append(
        &self,
        outcome: RunOutcome,
    ) -> Result<Stored<RunOutcome>, OperatorStoreError> {
        self.append_finalization(outcome, false).await
    }

    async fn append_finalization(
        &self,
        mut outcome: RunOutcome,
        reuse_recorded_completion_time: bool,
    ) -> Result<Stored<RunOutcome>, OperatorStoreError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
        }
        let store = Arc::clone(&self.store);
        let closed = Arc::clone(&self.closed);
        tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            if closed.load(Ordering::Acquire) {
                return Err(OperatorStoreError::Conflict("outcome runtime is shut down"));
            }
            let store = store
                .as_mut()
                .ok_or(OperatorStoreError::Conflict("outcome runtime is shut down"))?;
            store.retain_run_checkpoint(&mut outcome)?;
            if reuse_recorded_completion_time {
                if let Some(recorded) = store.get_run_outcome(&outcome.run_id)? {
                    outcome.evidence =
                        merge_checkpoint_evidence(recorded.value.evidence, outcome.evidence)?;
                    outcome.ended_at = recorded.value.ended_at;
                }
            }
            store.append_run_outcome(&outcome)
        })
        .await
        .map_err(|_| OperatorStoreError::Conflict("outcome writer task failed"))?
    }

    pub async fn checkpoint_and_finish(
        &self,
        start: WorkflowOutcomeStart,
        checkpoint: WorkflowOutcomeCheckpoint,
        terminal: WorkflowTerminal,
    ) -> Result<Stored<RunOutcome>, OperatorStoreError> {
        let run_id = start.run_id.clone();
        let title = workflow_result_title(&workflow_display_title(&start.workflow), &terminal);
        self.publish_report(
            start,
            title,
            "The run ended with the recorded status and evidence.".into(),
            checkpoint.evidence,
            terminal,
        )
        .await
        .map_err(|_| OperatorStoreError::Conflict("terminal report publication failed"))?;
        self.get_run_outcome(run_id)
            .await?
            .ok_or(OperatorStoreError::Conflict("published run is unavailable"))
    }

    pub async fn publish_assessment(
        &self,
        start: WorkflowOutcomeStart,
        title: String,
        report: String,
        evidence: Vec<EvidenceRef>,
    ) -> Result<Stored<WorkItem>, String> {
        self.publish_report(
            start,
            title,
            report,
            evidence,
            WorkflowTerminal::Inconclusive,
        )
        .await
    }

    pub async fn publish_workflow_result(
        &self,
        start: WorkflowOutcomeStart,
        summary: String,
        terminal: WorkflowTerminal,
    ) -> Result<Stored<WorkItem>, String> {
        let title = workflow_result_title(&workflow_display_title(&start.workflow), &terminal);
        let report = if summary.trim().is_empty() {
            "The workflow returned no readable summary.".to_string()
        } else {
            summary
        };
        self.publish_report(start, title, report, Vec::new(), terminal)
            .await
    }

    /// Terminal path for a workflow classified as `Housekeeping`.
    ///
    /// The run's `RunOutcome` is always recorded, so accountability is intact
    /// and every run stays queryable by `run_id`. No work item and no artifact
    /// revision are created while the loop is merely working or merely
    /// unlucky. Only once it is durably blocked does exactly one work item
    /// appear, and it is refreshed rather than duplicated on every repeat.
    pub(crate) async fn publish_housekeeping_result(
        &self,
        mut start: WorkflowOutcomeStart,
        terminal: WorkflowTerminal,
        failure: Option<WorkflowFailureCategory>,
    ) -> Result<(), String> {
        let subject = housekeeping_subject(&self.project, &start.workflow)
            .map_err(|error| error.to_string())?;
        start.work_id = subject.work_item_id();
        // Every housekeeping run shares one subject, coverage and fingerprint:
        // the thing being reported on is the health of the loop, not the
        // individual run. The per-run identity lives in `run_id`.
        start.coverage = HOUSEKEEPING_COVERAGE.to_string();
        start.input_fingerprint = "housekeeping-v1".to_string();
        let workflow = start.workflow.clone();
        let blocked = self.record_housekeeping_terminal(&workflow, &terminal);
        let outcome = build_workflow_outcome(
            start,
            WorkflowOutcomeCheckpoint::from_evidence([]),
            terminal,
            Utc::now().to_rfc3339(),
        );
        let (title, body) = housekeeping_blocked_report(&workflow, failure.as_ref());
        let title = super::jobs::autonomous_chats::redact_and_cap_text(&title, 240);
        let body = super::jobs::autonomous_chats::redact_and_cap_text(&body, 8_000);
        let project = self.project.clone();
        self.with_store(move |store| {
            // R-4 already raises ONE aggregated provider-rate-limit item across
            // every workflow. When that item is live it is the user's single
            // report of this outage, so housekeeping references it instead of
            // raising a second row saying the same thing.
            let aggregate_is_live = provider_rate_limit_item_is_live(store, &project)?;
            if blocked && !aggregate_is_live {
                // One item, refreshed not duplicated: the subject is constant and
                // the report carries no counters, so a repeat folds into the
                // existing artifact revision instead of appending a new one.
                publish_report_in_store(store, outcome, subject, title, body).map_err(|_| {
                    OperatorStoreError::Conflict("housekeeping report was not published")
                })?;
                return Ok(());
            }
            // Healthy, transiently unlucky, or already covered by R-4's item:
            // the outcome is still recorded, but nothing reaches the Desk.
            store.append_run_outcome(&outcome)?;
            retire_work_item_in_store(store, &subject.work_item_id())?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())
    }

    /// Count consecutive failed housekeeping runs and report whether the loop is
    /// now durably blocked. Anything that is not an outright failure resets the
    /// count, which is what makes the blocked item clear on recovery.
    fn record_housekeeping_terminal(&self, workflow: &str, terminal: &WorkflowTerminal) -> bool {
        let failed = matches!(
            terminal,
            WorkflowTerminal::Failed { .. } | WorkflowTerminal::Blocked { .. }
        );
        let Ok(mut failures) = self.housekeeping_failures.lock() else {
            return false;
        };
        if !failed {
            failures.remove(workflow);
            return false;
        }
        let count = failures
            .entry(workflow.to_string())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        *count >= HOUSEKEEPING_BLOCK_THRESHOLD
    }

    pub(crate) async fn publish_report(
        &self,
        mut start: WorkflowOutcomeStart,
        title: String,
        report: String,
        evidence: Vec<EvidenceRef>,
        terminal: WorkflowTerminal,
    ) -> Result<Stored<WorkItem>, String> {
        if report.trim().is_empty() {
            return Err("assessment has no readable report".into());
        }
        let subject = SubjectKey::new(
            &start.project,
            "workflow",
            &start.workflow,
            &start.coverage,
            &start.input_fingerprint,
        )
        .map_err(|error| error.to_string())?;
        let uncertainty = Some(
            "Buddy's assessment; its findings and any effects have not been independently verified."
                .to_string(),
        );
        start.work_id = subject.work_item_id();
        let outcome = build_workflow_outcome(
            start,
            WorkflowOutcomeCheckpoint::from_evidence(evidence),
            terminal,
            Utc::now().to_rfc3339(),
        );
        let title = super::jobs::autonomous_chats::redact_and_cap_text(&title, 240);
        let report = super::jobs::autonomous_chats::redact_and_cap_text(&report, 8_000);
        let store = self.store.clone();
        let closed = self.closed.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = store
                .lock()
                .map_err(|_| "outcome store lock poisoned".to_string())?;
            if closed.load(Ordering::Acquire) {
                return Err("outcome runtime is shut down".into());
            }
            let store = guard
                .as_mut()
                .ok_or_else(|| "outcome runtime is shut down".to_string())?;
            publish_report_in_store_with_uncertainty(
                store,
                outcome,
                subject,
                title,
                report,
                uncertainty,
            )
        })
        .await
        .map_err(|_| "assessment writer task failed".to_string())?
    }

    pub(crate) async fn publish_supervision_report(
        &self,
        link: super::supervision::SupervisionLink,
    ) -> Result<String, String> {
        self.with_store(move |store| store.publish_supervision_report(&link))
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        let store = Arc::clone(&self.store);
        let _ = tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .map_err(|_| OperatorStoreError::Conflict("outcome store lock poisoned"))?;
            store.take();
            Ok::<(), OperatorStoreError>(())
        })
        .await;
    }
}

pub(crate) fn publish_report_in_store(
    store: &mut OperatorStore,
    outcome: RunOutcome,
    subject: SubjectKey,
    title: String,
    report: String,
) -> Result<Stored<WorkItem>, String> {
    publish_report_in_store_with_uncertainty(
        store,
        outcome,
        subject,
        title,
        report,
        Some("Buddy's assessment; its findings and any effects have not been independently verified.".into()),
    )
}

fn publish_report_in_store_with_uncertainty(
    store: &mut OperatorStore,
    outcome: RunOutcome,
    subject: SubjectKey,
    title: String,
    report: String,
    uncertainty: Option<String>,
) -> Result<Stored<WorkItem>, String> {
    publish_report_in_store_owned_with_uncertainty(
        store,
        outcome,
        subject,
        title,
        report,
        None,
        uncertainty,
    )
}

pub(crate) fn publish_report_in_store_owned(
    store: &mut OperatorStore,
    outcome: RunOutcome,
    subject: SubjectKey,
    title: String,
    report: String,
    completion: Option<&super::operator_store::SupervisionCompletion>,
) -> Result<Stored<WorkItem>, String> {
    publish_report_in_store_owned_with_uncertainty(
        store,
        outcome,
        subject,
        title,
        report,
        completion,
        Some("Buddy's assessment; its findings and any effects have not been independently verified.".into()),
    )
}

fn publish_report_in_store_owned_with_uncertainty(
    store: &mut OperatorStore,
    mut outcome: RunOutcome,
    subject: SubjectKey,
    title: String,
    report: String,
    completion: Option<&super::operator_store::SupervisionCompletion>,
    uncertainty: Option<String>,
) -> Result<Stored<WorkItem>, String> {
    store
        .retain_run_checkpoint(&mut outcome)
        .map_err(|error| error.to_string())?;
    if let Some(recorded) = store
        .get_run_outcome(&outcome.run_id)
        .map_err(|error| error.to_string())?
    {
        outcome.evidence =
            merge_checkpoint_evidence(recorded.value.evidence.clone(), outcome.evidence)
                .map_err(|error| error.to_string())?;
        outcome.ended_at = recorded.value.ended_at.clone();
        if recorded.value != outcome {
            return Err("run already exists with different evidence".into());
        }
        let work = store
            .get_work_item(&outcome.work_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "recorded run has no work item".to_string())?;
        let artifact = store
            .artifact_for_run(&subject.artifact_id(), &recorded)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "recorded run has no assessment".to_string())?;
        if artifact.value.body != report || artifact.value.title != title {
            return Err("run already exists with a different assessment".into());
        }
        return Ok(work);
    }
    let current = store
        .get_work_item(&subject.work_item_id())
        .map_err(|error| error.to_string())?;
    let current_artifact = store
        .get_artifact_revision(&subject.artifact_id(), None)
        .map_err(|error| error.to_string())?;
    let (conclusion, state) = match outcome.status {
        RunStatus::Failed => (
            "Run failed.",
            refact_buddy_core::operator_types::WorkItemState::Blocked,
        ),
        RunStatus::Blocked => (
            "Run blocked. The stated scope was not successfully checked.",
            refact_buddy_core::operator_types::WorkItemState::Blocked,
        ),
        RunStatus::Cancelled => (
            "Run cancelled.",
            refact_buddy_core::operator_types::WorkItemState::Cancelled,
        ),
        RunStatus::Partial => (
            "Run incomplete. The retained evidence covers only part of the scope.",
            refact_buddy_core::operator_types::WorkItemState::NeedsAttention,
        ),
        RunStatus::NoChange => (
            "No change reported within the recorded observation scope.",
            refact_buddy_core::operator_types::WorkItemState::Watching,
        ),
        RunStatus::Completed => (
            "Run completed.",
            refact_buddy_core::operator_types::WorkItemState::Delivered,
        ),
        _ => (
            "Assessment ready.",
            refact_buddy_core::operator_types::WorkItemState::NeedsAttention,
        ),
    };
    let artifact = ArtifactDraft {
        subject: subject.clone(),
        source_run_ids: vec![outcome.run_id.clone()],
        title: title.clone(),
        conclusion: conclusion.into(),
        delta: String::new(),
        body: report,
        uncertainty: uncertainty.clone(),
        proposed_action_ids: Vec::new(),
        evidence: outcome.evidence.clone(),
    }
    .materialize(
        std::slice::from_ref(&outcome),
        current_artifact.as_ref().map(|record| &record.value),
    )
    .map_err(|error| error.to_string())?;
    let mut item = WorkItemDraft {
        project: outcome.project.clone(),
        subject,
        kind: "assessment".into(),
        title: title.clone(),
        state,
        native_sources: outcome
            .evidence
            .iter()
            .map(|evidence| evidence.source.clone())
            .collect(),
        last_material_change_at: outcome
            .ended_at
            .clone()
            .unwrap_or_else(|| outcome.started_at.clone()),
        severity: Some(WorkItemSeverity::Unknown),
        problem_summary: problem_summary_from_title(&title),
        scope: Some(outcome.coverage.clone()),
    }
    .build()
    .map_err(|error| error.to_string())?;
    let new_artifact = match artifact {
        ArtifactMaterialization::Revision(mut artifact) => {
            artifact.provenance.contains_untrusted_source = true;
            item.current_artifact_id = Some(artifact.artifact_id.clone());
            item.current_artifact_revision = Some(artifact.revision);
            Some(artifact)
        }
        ArtifactMaterialization::Folded {
            artifact_id,
            revision,
        } => {
            item.current_artifact_id = Some(artifact_id);
            item.current_artifact_revision = Some(revision);
            if let Some(current) = &current {
                item.last_material_change_at = current.value.last_material_change_at.clone();
            }
            None
        }
    };
    let (_, work, _) = store
        .append_outcome_work_item_artifact_owned(
            &outcome,
            &item,
            current.as_ref().map(|record| record.revision),
            new_artifact.as_ref(),
            completion,
        )
        .map_err(|error| error.to_string())?;
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::NativeSourceIdentity;
    use tempfile::TempDir;

    #[tokio::test]
    async fn durable_begin_survives_failed_publication_and_restart() {
        let dir = TempDir::new().unwrap();
        let begun = start(dir.path(), "durable");
        let runtime =
            OutcomeRuntime::open(dir.path(), begun.project.clone(), begun.worker.clone()).unwrap();
        runtime.begin_workflow(begun.clone()).await.unwrap();
        runtime
            .checkpoint_workflow(begun.clone(), WorkflowOutcomeCheckpoint::from_evidence([]))
            .await
            .unwrap();
        assert!(runtime
            .publish_assessment(begun.clone(), "Report".into(), "".into(), vec![])
            .await
            .is_err());
        assert!(runtime
            .get_run_outcome(begun.run_id.clone())
            .await
            .unwrap()
            .is_none());
        runtime.shutdown().await;
        let reopened =
            OutcomeRuntime::open(dir.path(), begun.project.clone(), begun.worker.clone()).unwrap();
        let outcome = reopened
            .get_run_outcome(begun.run_id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.value.status, RunStatus::Blocked);
        let history = reopened
            .list_work_outcomes(outcome.value.work_id.clone(), None, 10)
            .await
            .unwrap();
        assert_eq!(history.entries, vec![outcome.clone()]);
        assert!(reopened
            .publish_workflow_result(
                begun.clone(),
                "Completed".into(),
                WorkflowTerminal::Completed
            )
            .await
            .is_err());
        assert_eq!(
            reopened
                .get_run_outcome(begun.run_id)
                .await
                .unwrap()
                .unwrap(),
            outcome
        );
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn failed_and_cancelled_runs_publish_truthful_readable_deliverables() {
        let root = TempDir::new().unwrap();
        let project = identity(root.path());
        let runtime = OutcomeRuntime::open(
            root.path(),
            project,
            WorkerIdentity {
                worker_id: "test-worker".into(),
                engine_instance_id: "test-engine".into(),
                epoch: 1,
            },
        )
        .unwrap();
        for (id, terminal, expected_state, expected_title) in [
            (
                "failed",
                WorkflowTerminal::Failed {
                    failure_class: Some("auth".into()),
                },
                refact_buddy_core::operator_types::WorkItemState::Blocked,
                "Dependency Radar didn't finish",
            ),
            (
                "cancelled",
                WorkflowTerminal::Cancelled {
                    failure_class: None,
                },
                refact_buddy_core::operator_types::WorkItemState::Cancelled,
                "Dependency Radar was cancelled",
            ),
        ] {
            let mut start = runtime.start_workflow(
                refact_buddy_core::autonomous_workflows::DEPENDENCY_RADAR_WORKFLOW_ID,
                "2026-09-05T00:00:00Z".into(),
            );
            start.run_id = id.into();
            start.input_fingerprint = id.into();
            let work = runtime
                .publish_workflow_result(
                    start,
                    "Run did not finish the requested check.".into(),
                    terminal,
                )
                .await
                .unwrap();
            assert_eq!(work.value.state, expected_state);
            assert_eq!(work.value.title, expected_title);
            assert!(!work.value.title.contains("assessment"));
            let artifact = runtime
                .get_artifact_revision(work.value.current_artifact_id.unwrap(), Some(1))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(artifact.value.title, expected_title);
            assert!(!artifact.value.title.contains("assessment"));
            assert!(artifact.value.body.contains("did not finish"));
            assert!(
                artifact.value.conclusion.contains("Run failed.")
                    || artifact.value.conclusion.contains("Run cancelled.")
            );
            assert!(artifact.value.proposed_action_ids.is_empty());
        }
        runtime.shutdown().await;
    }

    #[test]
    fn workflow_result_titles_are_terminal_specific() {
        let cases = [
            (WorkflowTerminal::NoChange, "Dependency Radar — no change"),
            (
                WorkflowTerminal::Partial {
                    failure_class: Some("partial".into()),
                },
                "Dependency Radar is incomplete",
            ),
            (
                WorkflowTerminal::Blocked {
                    failure_class: Some("blocked".into()),
                },
                "Dependency Radar is blocked",
            ),
            (
                WorkflowTerminal::Cancelled {
                    failure_class: Some("cancelled".into()),
                },
                "Dependency Radar was cancelled",
            ),
            (
                WorkflowTerminal::Failed {
                    failure_class: Some("failed".into()),
                },
                "Dependency Radar didn't finish",
            ),
            (WorkflowTerminal::Completed, "Dependency Radar finished"),
            (
                WorkflowTerminal::Inconclusive,
                "Dependency Radar assessment",
            ),
        ];

        for (terminal, expected) in cases {
            assert_eq!(
                workflow_result_title("Dependency Radar", &terminal),
                expected
            );
        }
    }

    #[tokio::test]
    async fn one_caveat_per_published_report() {
        let dir = TempDir::new().unwrap();
        let runtime = OutcomeRuntime::open(
            dir.path(),
            identity(dir.path()),
            WorkerIdentity {
                worker_id: "worker".into(),
                engine_instance_id: "engine".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let terminals = [
            WorkflowTerminal::Completed,
            WorkflowTerminal::Inconclusive,
            WorkflowTerminal::Partial {
                failure_class: Some("partial".into()),
            },
            WorkflowTerminal::NoChange,
            WorkflowTerminal::Failed {
                failure_class: Some("failed".into()),
            },
            WorkflowTerminal::Blocked {
                failure_class: Some("blocked".into()),
            },
            WorkflowTerminal::Cancelled {
                failure_class: Some("cancelled".into()),
            },
        ];
        let caveat = regex::Regex::new(
            r"(?i)(not (been )?independently verif|no fix was|unverified|not proof|remain(s)? inconclusive)",
        )
        .unwrap();
        for (index, terminal) in terminals.into_iter().enumerate() {
            let mut start = runtime.start_workflow("commit_msg", "2026-09-05T00:00:00Z".into());
            start.run_id = format!("caveat-run-{index}");
            start.input_fingerprint = format!("caveat-{index}");
            let work = runtime
                .publish_workflow_result(start, "A readable result.".into(), terminal)
                .await
                .unwrap();
            let artifact = runtime
                .get_artifact_revision(
                    work.value.current_artifact_id.unwrap(),
                    work.value.current_artifact_revision,
                )
                .await
                .unwrap()
                .unwrap();
            let rendered = format!(
                "{}\n{}\n{}\n{}",
                artifact.value.conclusion,
                artifact.value.delta,
                artifact.value.body,
                artifact.value.uncertainty.as_deref().unwrap_or_default()
            );
            assert_eq!(caveat.find_iter(&rendered).count(), 1, "{rendered}");
            assert!(artifact.value.uncertainty.is_some(), "{rendered}");
            assert!(artifact
                .value
                .uncertainty
                .as_deref()
                .unwrap()
                .contains("independently verified"));
        }
        let check = crate::buddy::workflow_deliverables::SourceCheck {
            scope: "the complete scratch scope".into(),
            input_fingerprint: "source-check-fingerprint".into(),
            evidence: vec![refact_buddy_core::operator_types::EvidenceRef {
                evidence_id: "source-check-evidence".into(),
                source: NativeSourceIdentity {
                    source_kind: refact_buddy_core::operator_types::NativeSourceKind::Check,
                    source_id: "source-check".into(),
                    source_version: "1".into(),
                },
                locator: "refact://check/source-check".into(),
                observed_at: "2026-09-05T00:00:00Z".into(),
                digest: None,
            }],
            completion: crate::buddy::workflow_deliverables::SourceCompletion::Complete,
            findings: vec![],
        };
        let report = check.into_report("commit_msg");
        let mut start = runtime.start_workflow("commit_msg", "2026-09-05T00:00:00Z".into());
        start.run_id = "source-check-report".into();
        start.input_fingerprint = "source-check-report".into();
        let work = runtime
            .publish_report(
                start,
                report.title,
                report.body,
                report.evidence,
                report.terminal,
            )
            .await
            .unwrap();
        let artifact = runtime
            .get_artifact_revision(
                work.value.current_artifact_id.unwrap(),
                work.value.current_artifact_revision,
            )
            .await
            .unwrap()
            .unwrap();
        let rendered = format!(
            "{}\n{}\n{}\n{}",
            artifact.value.conclusion,
            artifact.value.delta,
            artifact.value.body,
            artifact.value.uncertainty.as_deref().unwrap_or_default()
        );
        assert_eq!(caveat.find_iter(&rendered).count(), 1, "{rendered}");
        assert!(artifact.value.uncertainty.is_some(), "{rendered}");
        assert!(artifact
            .value
            .uncertainty
            .as_deref()
            .unwrap()
            .contains("independently verified"));
        assert!(artifact
            .value
            .uncertainty
            .as_deref()
            .unwrap()
            .contains("independently verified"));
        let assessment = crate::buddy::workflow_deliverables::assessment_body(
            refact_buddy_core::autonomous_workflows::DEPENDENCY_RADAR_WORKFLOW_ID,
            "Cargo.toml modified",
            "A follow-up is available.",
            None,
        );
        let assessment_caveat_count = caveat.find_iter(&assessment).count();
        assert_eq!(assessment_caveat_count, 0, "{assessment}");
        let mut start = runtime.start_workflow(
            refact_buddy_core::autonomous_workflows::DEPENDENCY_RADAR_WORKFLOW_ID,
            "2026-09-05T00:00:00Z".into(),
        );
        start.run_id = "autonomous-assessment-caveat".into();
        start.input_fingerprint = "autonomous-assessment-caveat".into();
        let work = runtime
            .publish_assessment(
                start,
                "Dependency Radar assessment".into(),
                assessment,
                vec![],
            )
            .await
            .unwrap();
        let artifact = runtime
            .get_artifact_revision(
                work.value.current_artifact_id.unwrap(),
                work.value.current_artifact_revision,
            )
            .await
            .unwrap()
            .unwrap();
        let rendered = format!(
            "{}\n{}\n{}\n{}",
            artifact.value.conclusion,
            artifact.value.delta,
            artifact.value.body,
            artifact.value.uncertainty.as_deref().unwrap_or_default()
        );
        assert_eq!(caveat.find_iter(&rendered).count(), 1, "{rendered}");
        assert!(artifact.value.uncertainty.is_some(), "{rendered}");
        assert!(artifact
            .value
            .uncertainty
            .as_deref()
            .unwrap()
            .contains("independently verified"));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn titles_never_contain_raw_workflow_ids() {
        for workflow in refact_buddy_core::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS {
            let dir = TempDir::new().unwrap();
            let runtime = OutcomeRuntime::open(
                dir.path(),
                identity(dir.path()),
                WorkerIdentity {
                    worker_id: "worker".into(),
                    engine_instance_id: "engine".into(),
                    epoch: 1,
                },
            )
            .unwrap();
            let start = runtime.start_workflow(workflow.id, "2026-09-05T00:00:00Z".into());
            let work = runtime
                .publish_workflow_result(
                    start,
                    "Completed summary.".into(),
                    WorkflowTerminal::Completed,
                )
                .await
                .unwrap();
            let title = work.value.title;
            assert!(title.starts_with(workflow.title), "{title}");
            assert!(!title.contains("run outcome"), "{title}");
            assert!(
                !title.split_whitespace().any(|token| token.contains('_')),
                "{title}"
            );
            runtime.shutdown().await;
        }
    }

    fn identity(root: &Path) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "project".to_string(),
            canonical_root: std::fs::canonicalize(root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    fn start(root: &Path, run_id: &str) -> WorkflowOutcomeStart {
        WorkflowOutcomeStart {
            run_id: run_id.to_string(),
            work_id: "workflow:commit_msg".to_string(),
            parent_run_id: parent_run_id(Some(" parent-run ")),
            project: identity(root),
            worker: WorkerIdentity {
                worker_id: "worker".to_string(),
                engine_instance_id: "engine".to_string(),
                epoch: 1,
            },
            origin: "buddy_workflow".to_string(),
            workflow: "commit_msg".to_string(),
            input_fingerprint: "native:workflow:commit_msg".to_string(),
            coverage: "workflow execution result only".to_string(),
            started_at: "2026-09-05T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn checkpoint_merge_retains_existing_evidence_and_rejects_identity_rewrites() {
        let evidence = |index: usize| EvidenceRef {
            evidence_id: format!("evidence-{index:02}"),
            source: NativeSourceIdentity {
                source_kind: refact_buddy_core::operator_types::NativeSourceKind::Tool,
                source_id: "owned-tool".into(),
                source_version: "1".into(),
            },
            locator: "tool://owned".into(),
            observed_at: "2026-09-05T00:00:00Z".into(),
            digest: None,
        };
        let retained: Vec<_> = (10..10 + MAX_OUTCOME_EVIDENCE).map(evidence).collect();
        assert_eq!(
            merge_checkpoint_evidence(retained.clone(), vec![evidence(0)]).unwrap(),
            retained
        );
        let mut changed = evidence(10);
        changed.source.source_id = "foreign-tool".into();
        assert!(merge_checkpoint_evidence(retained, vec![changed]).is_err());
    }

    #[test]
    fn parent_run_id_omits_blank_values() {
        assert_eq!(parent_run_id(None), None);
        assert_eq!(parent_run_id(Some("  ")), None);
        assert_eq!(parent_run_id(Some(" parent ")).as_deref(), Some("parent"));
    }

    #[test]
    fn no_change_is_explicit_not_inferred_from_empty_evidence() {
        let dir = TempDir::new().unwrap();
        let outcome = build_workflow_outcome(
            start(dir.path(), "run"),
            WorkflowOutcomeCheckpoint::from_evidence([]),
            WorkflowTerminal::Inconclusive,
            "2026-09-05T00:01:00Z".to_string(),
        );
        assert_eq!(outcome.status, RunStatus::Inconclusive);
    }

    #[tokio::test]
    async fn failure_class_preserves_provider_rate_limit() {
        let dir = TempDir::new().unwrap();
        let start = start(dir.path(), "provider-rate-limit");
        let runtime =
            OutcomeRuntime::open(dir.path(), start.project.clone(), start.worker.clone()).unwrap();
        let outcome = build_workflow_outcome(
            start,
            WorkflowOutcomeCheckpoint::from_evidence([]),
            terminal_from_workflow_result(&Err(
                "OpenAI API error (429): rate_limit_exceeded".to_string()
            )),
            "2026-09-05T00:01:00Z".to_string(),
        );
        let stored = runtime.append(outcome).await.unwrap();
        assert_eq!(
            stored.value.failure_class.as_deref(),
            Some("provider_rate_limit")
        );
        runtime.shutdown().await;
    }

    #[test]
    fn failure_class_preserves_provider_transient() {
        assert_eq!(
            terminal_from_workflow_result(&Err(
                "OpenAI API error (503 Service Unavailable): overloaded".to_string()
            )),
            WorkflowTerminal::Failed {
                failure_class: Some("provider_transient".to_string())
            }
        );
    }

    #[test]
    fn legacy_workflow_error_rows_still_deserialise() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))
        .unwrap();
        let legacy: RunOutcome = serde_json::from_value(fixture["run_outcome"].clone()).unwrap();
        assert_eq!(legacy.failure_class.as_deref(), Some("workflow_error"));
    }

    #[tokio::test]
    async fn duplicate_completion_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let runtime = OutcomeRuntime::open(
            dir.path(),
            identity(dir.path()),
            WorkerIdentity {
                worker_id: "worker".to_string(),
                engine_instance_id: "engine".to_string(),
                epoch: 1,
            },
        )
        .unwrap();
        let start = start(dir.path(), "run");
        let first = runtime
            .checkpoint_and_finish(
                start.clone(),
                WorkflowOutcomeCheckpoint::from_evidence([]),
                WorkflowTerminal::NoChange,
            )
            .await
            .unwrap();
        let second = runtime
            .checkpoint_and_finish(
                start,
                WorkflowOutcomeCheckpoint::from_evidence([]),
                WorkflowTerminal::NoChange,
            )
            .await
            .unwrap();
        assert_eq!(first.change_seq, second.change_seq);
    }

    #[tokio::test]
    async fn cancelled_run_preserves_bounded_evidence() {
        let dir = TempDir::new().unwrap();
        let runtime = OutcomeRuntime::open(
            dir.path(),
            identity(dir.path()),
            WorkerIdentity {
                worker_id: "worker".to_string(),
                engine_instance_id: "engine".to_string(),
                epoch: 1,
            },
        )
        .unwrap();
        let evidence = (0..20).map(|index| EvidenceRef {
            evidence_id: format!("evidence-{index:02}"),
            source: NativeSourceIdentity {
                source_kind: refact_buddy_core::operator_types::NativeSourceKind::Tool,
                source_id: "tool".to_string(),
                source_version: "1".to_string(),
            },
            locator: format!("tool://{index}"),
            observed_at: "2026-09-05T00:00:00Z".to_string(),
            digest: None,
        });
        let stored = runtime
            .checkpoint_and_finish(
                start(dir.path(), "cancelled"),
                WorkflowOutcomeCheckpoint::from_evidence(evidence),
                WorkflowTerminal::Cancelled {
                    failure_class: Some("cancelled".to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(stored.value.status, RunStatus::Cancelled);
        assert_eq!(stored.value.evidence.len(), MAX_OUTCOME_EVIDENCE);
    }

    #[tokio::test]
    async fn folded_runs_remain_discoverable_and_old_retries_match_their_original_report() {
        let dir = TempDir::new().unwrap();
        let runtime = OutcomeRuntime::open(
            dir.path(),
            identity(dir.path()),
            start(dir.path(), "one").worker,
        )
        .unwrap();
        let first = runtime
            .publish_assessment(
                start(dir.path(), "one"),
                "Assessment".into(),
                "Original report".into(),
                vec![],
            )
            .await
            .unwrap();
        runtime
            .publish_assessment(
                start(dir.path(), "two"),
                "Assessment".into(),
                "Original report".into(),
                vec![],
            )
            .await
            .unwrap();
        let newer = runtime
            .publish_assessment(
                start(dir.path(), "three"),
                "Assessment".into(),
                "Changed report".into(),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(newer.value.current_artifact_revision, Some(2));
        let retry = runtime
            .publish_assessment(
                start(dir.path(), "one"),
                "Assessment".into(),
                "Original report".into(),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(retry, newer);
        let folded_retry = runtime
            .publish_assessment(
                start(dir.path(), "two"),
                "Assessment".into(),
                "Original report".into(),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(folded_retry, newer);
        assert!(runtime
            .publish_assessment(
                start(dir.path(), "two"),
                "Assessment".into(),
                "Changed report".into(),
                vec![]
            )
            .await
            .is_err());
        let page = runtime
            .list_work_outcomes(first.value.id.clone(), None, 2)
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[1].value.run_id, "two");
        let next = runtime
            .list_work_outcomes(first.value.id, page.next_after, 2)
            .await
            .unwrap();
        assert_eq!(next.entries.len(), 1);
        assert_eq!(next.entries[0].value.run_id, "three");
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn assessment_publication_is_readable_atomic_and_reuses_material_revision() {
        let dir = TempDir::new().unwrap();
        let runtime = OutcomeRuntime::open(
            dir.path(),
            identity(dir.path()),
            start(dir.path(), "one").worker,
        )
        .unwrap();
        let first_start = start(dir.path(), "one");
        let first = runtime
            .publish_assessment(
                first_start.clone(),
                "Assessment".into(),
                "A readable result".into(),
                vec![],
            )
            .await
            .unwrap();
        let duplicate = runtime
            .publish_assessment(
                first_start.clone(),
                "Assessment".into(),
                "A readable result".into(),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(first, duplicate);
        assert!(runtime
            .publish_assessment(
                first_start,
                "Assessment".into(),
                "Different result".into(),
                vec![]
            )
            .await
            .is_err());
        let repeated = runtime
            .publish_assessment(
                start(dir.path(), "two"),
                "Assessment".into(),
                "A readable result".into(),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(repeated.value.current_artifact_revision, Some(1));
        assert_eq!(
            repeated.value.last_material_change_at,
            first.value.last_material_change_at
        );
        assert_eq!(
            runtime
                .list_work_items(None, 50)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        let artifact = runtime
            .get_artifact_revision(first.value.current_artifact_id.unwrap(), Some(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(artifact.value.body, "A readable result");
        assert!(artifact.value.provenance.contains_untrusted_source);
        assert!(artifact
            .value
            .uncertainty
            .as_deref()
            .unwrap()
            .contains("not been independently verified"));
        let failed = runtime
            .publish_assessment(
                start(dir.path(), "empty"),
                "Assessment".into(),
                "".into(),
                vec![],
            )
            .await;
        assert!(failed.is_err());
        runtime.shutdown().await;
        assert!(runtime
            .publish_assessment(
                start(dir.path(), "closed"),
                "Assessment".into(),
                "Result".into(),
                vec![]
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn write_failure_has_no_report_saved_signal() {
        let dir = TempDir::new().unwrap();
        let runtime = OutcomeRuntime::open(
            dir.path(),
            identity(dir.path()),
            WorkerIdentity {
                worker_id: "worker".to_string(),
                engine_instance_id: "engine".to_string(),
                epoch: 1,
            },
        )
        .unwrap();
        runtime.shutdown().await;
        assert!(runtime
            .checkpoint_and_finish(
                start(dir.path(), "closed"),
                WorkflowOutcomeCheckpoint::from_evidence([]),
                WorkflowTerminal::NoChange
            )
            .await
            .is_err());
    }
}
