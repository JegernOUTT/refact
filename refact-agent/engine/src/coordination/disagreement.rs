//! Disagreement records: dissent kept as signal rather than resolved by whoever
//! wrote last.
//!
//! A filed disagreement carries the claim in dispute and every position argued
//! on it, each with its author, run generation, evidence and the revision it was
//! argued against. Adjudication settles it **on the evidence presented** and
//! never deletes the rejected side.
//!
//! **Not automatically a blocker.** Whether an open disagreement stops work from
//! being reported complete is a policy decision read from settings, defaulting
//! to off. Nothing here cancels, reprioritises or admits anything.

use std::sync::Arc;

use refact_work_core::store::records::{now_ms, DisagreementRecord, DisagreementStatus, EvidenceRef};
use refact_work_core::{
    default_store_path, CoordinationStore, DisagreementFiling, ProjectAccess, ProjectKey,
    StoreError,
};

use crate::app_state::AppState;

pub const NO_STORE: &str =
    "no coordination store is available, so disagreements cannot be filed or read";

/// This project's disagreements, in the coordination store.
pub struct DisagreementLedger {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
}

impl DisagreementLedger {
    pub fn new(store: Arc<CoordinationStore>, project: ProjectKey) -> Self {
        let access = store.authorize(project);
        DisagreementLedger {
            store,
            access,
        }
    }

    pub fn project(&self) -> &ProjectKey {
        self.access.project()
    }

    pub fn store_path(&self) -> &std::path::Path {
        self.store.path()
    }

    pub async fn file(
        &self,
        record: &DisagreementRecord,
    ) -> Result<DisagreementFiling, StoreError> {
        self.store.file_disagreement(&self.access, record).await
    }

    pub async fn get(&self, record_id: &str) -> Result<Option<DisagreementRecord>, StoreError> {
        self.store.get_disagreement(&self.access, record_id).await
    }

    pub async fn list(
        &self,
        status: Option<DisagreementStatus>,
    ) -> Result<Vec<DisagreementRecord>, StoreError> {
        self.store.list_disagreements(&self.access, status).await
    }

    pub async fn put_resolution(&self, record: &DisagreementRecord) -> Result<bool, StoreError> {
        self.store
            .put_disagreement_resolution(&self.access, record)
            .await
    }

    /// Drop settled records older than the retention window. Open records are
    /// untouched however old they are.
    pub async fn expire(&self, retention_ms: u64, at_ms: i64) -> Result<usize, StoreError> {
        let cutoff = at_ms.saturating_sub(retention_ms as i64);
        self.store.expire_disagreements(&self.access, cutoff).await
    }
}

/// Open this project's ledger. Coordination being off, or a store that cannot be
/// opened, leaves it absent and the surface reports itself unavailable.
pub async fn ledger_for(app: &AppState) -> Option<DisagreementLedger> {
    let settings = crate::file_activity::settings();
    if !settings.enabled {
        return None;
    }
    let path = default_store_path(&app.paths.cache_dir);
    let project = super::outbox::project_key_for_root(app, None).await;
    match super::outbox::store_for_path(&path, &settings).await {
        Ok(store) => Some(DisagreementLedger::new(store, project)),
        Err(error) => {
            super::outbox::note_durability_unavailable(&path, error.to_string());
            None
        }
    }
}

pub fn retention_ms() -> u64 {
    crate::runtime_settings::current()
        .coordination
        .disagreement_retention_ms
}

pub fn blocks_completion() -> bool {
    crate::runtime_settings::current()
        .coordination
        .disagreement_blocks_completion
}

/// Whether open dissent stops completion, and which records it names.
///
/// `open` is reported either way: the same records are visible whether or not
/// the policy makes them a blocker.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CompletionGate {
    pub blocked: bool,
    pub open: Vec<String>,
}

/// Read the policy against the records. This is the only place that decides
/// whether dissent blocks, and it decides it from settings, never from the
/// records alone.
pub fn completion_gate(records: &[DisagreementRecord], blocks: bool) -> CompletionGate {
    let open: Vec<String> = records
        .iter()
        .filter(|record| record.status.is_open())
        .map(|record| record.record_id.clone())
        .collect();
    CompletionGate {
        blocked: blocks && !open.is_empty(),
        open,
    }
}

/// Open dissent as completion blocks, when the project's policy makes it a blocker.
pub async fn completion_gate_blocks(app: &AppState) -> Vec<crate::chat::evidence::GateBlock> {
    use crate::chat::evidence::{EvidenceVerdict, GateBlock};

    if !blocks_completion() {
        return Vec::new();
    }
    let settings = crate::file_activity::settings();
    if !settings.enabled {
        return Vec::new();
    }
    let unavailable = || {
        vec![GateBlock {
            spec_id: "disagreement:unavailable".to_string(),
            command: String::new(),
            verdict: EvidenceVerdict::Unavailable,
            exit_code: None,
            reason: "disagreement ledger unavailable; cannot confirm there is no open dissent"
                .to_string(),
            from_cache: false,
        }]
    };
    let Some(ledger) = ledger_for(app).await else {
        return unavailable();
    };
    let records = match ledger.list(Some(DisagreementStatus::Open)).await {
        Ok(records) => records,
        Err(_) => return unavailable(),
    };
    let gate = completion_gate(&records, true);
    gate.open
        .into_iter()
        .filter_map(|record_id| {
            let record = records
                .iter()
                .find(|record| record.record_id == record_id)?;
            let reason = format!(
                "open disagreement {} on {}: {}",
                record.record_id, record.subject, record.claim
            )
            .chars()
            .take(500)
            .collect();
            Some(GateBlock {
                spec_id: format!("disagreement:{record_id}"),
                command: String::new(),
                // An open dispute blocks completion; it does not prove either position failed.
                verdict: EvidenceVerdict::Blocked,
                exit_code: None,
                reason,
                from_cache: false,
            })
        })
        .collect()
}

fn render_evidence(evidence: &[EvidenceRef]) -> String {
    if evidence.is_empty() {
        return "none presented".to_string();
    }
    evidence
        .iter()
        .map(|reference| reference.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Model-facing text for one record. Both sides are always printed, and an
/// adjudicated record still prints the argument it rejected.
pub fn render(record: &DisagreementRecord) -> String {
    let mut text = format!(
        "Disagreement `{}` [{}] on `{}`\n- claim: {}\n",
        record.record_id,
        record.status.as_str(),
        record.subject,
        record.claim
    );
    for position in &record.positions {
        text.push_str(&format!(
            "- position `{}` by {} (run {}, revision `{}`): {}\n  evidence: {}\n",
            position.position_id,
            position.author,
            position.run_generation,
            position.subject_revision,
            position.statement,
            render_evidence(&position.evidence)
        ));
    }
    match (&record.adjudication, &record.open_reason) {
        (Some(adjudication), _) => {
            text.push_str(&format!(
                "- adjudicated by {} on {}: position `{}` stands, because {}\n",
                adjudication.adjudicator,
                adjudication.decisive_evidence,
                adjudication.resolved_position,
                adjudication.rationale
            ));
            let rejected: Vec<&str> = record
                .rejected_positions()
                .iter()
                .map(|position| position.position_id.as_str())
                .collect();
            if !rejected.is_empty() {
                text.push_str(&format!(
                    "- rejected position(s) `{}` are kept above and stay readable\n",
                    rejected.join("`, `")
                ));
            }
        }
        (None, Some(reason)) => {
            text.push_str(&format!(
                "- still open: no decisive evidence was presented — {reason}\n"
            ));
        }
        (None, None) => {
            text.push_str("- not adjudicated\n");
        }
    }
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

pub fn render_all(records: &[DisagreementRecord], gate: &CompletionGate) -> String {
    if records.is_empty() {
        return "No disagreements are on record for this project.".to_string();
    }
    let mut text = records.iter().map(render).collect::<Vec<_>>().join("\n\n");
    text.push_str(&format!(
        "\n\n{} open disagreement(s); completion is {}.",
        gate.open.len(),
        if gate.blocked {
            "blocked by policy"
        } else {
            "not blocked by them"
        }
    ));
    text
}

/// Persist a resolution and sweep settled records past the retention window.
pub async fn store_resolution(
    ledger: &DisagreementLedger,
    record: &DisagreementRecord,
) -> Result<bool, StoreError> {
    let written = ledger.put_resolution(record).await?;
    if let Err(error) = ledger.expire(retention_ms(), now_ms()).await {
        tracing::warn!("disagreement retention failed: {error}");
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::store::records::{
        AdjudicationRefusal, AdjudicationVerdict, DisagreementPosition, EvidenceKind,
    };
    use refact_work_core::{CoordinationSettings, WorkKey};

    const REVISION: &str = "sha-abc";

    async fn ledger() -> DisagreementLedger {
        let store = Arc::new(
            CoordinationStore::open_in_memory(&CoordinationSettings::default())
                .await
                .unwrap(),
        );
        DisagreementLedger::new(store, ProjectKey::new("alpha"))
    }

    fn position(
        position_id: &str,
        author: &str,
        evidence: Vec<EvidenceRef>,
    ) -> DisagreementPosition {
        DisagreementPosition {
            position_id: position_id.to_string(),
            author: WorkKey::background_agent(author),
            run_generation: 2,
            statement: format!("{position_id} is right"),
            evidence,
            asserted_at_ms: now_ms(),
            subject_revision: REVISION.to_string(),
        }
    }

    fn record(record_id: &str) -> DisagreementRecord {
        DisagreementRecord {
            project: ProjectKey::new("alpha"),
            record_id: record_id.to_string(),
            claim: "the lock is held across the await".to_string(),
            subject: "src/chat/session.rs".to_string(),
            positions: vec![
                position(
                    "held",
                    "agent-a",
                    vec![EvidenceRef::new(
                        EvidenceKind::File,
                        "src/chat/session.rs:210",
                    )],
                ),
                position("dropped", "agent-b", vec![]),
            ],
            status: DisagreementStatus::Open,
            adjudication: None,
            open_reason: None,
            filed_at_ms: now_ms(),
            updated_at_ms: now_ms(),
        }
    }

    #[tokio::test]
    async fn an_unresolvable_disagreement_stays_open_and_a_moved_revision_goes_stale() {
        let ledger = ledger().await;
        let mut unresolved = record("d-open");
        let mut moved = record("d-stale");
        ledger.file(&unresolved).await.unwrap();
        ledger.file(&moved).await.unwrap();

        unresolved
            .resolve(
                "reviewer",
                AdjudicationVerdict::Unresolvable {
                    reason: "neither side ran the case".to_string(),
                },
                REVISION,
                now_ms(),
            )
            .unwrap();
        store_resolution(&ledger, &unresolved).await.unwrap();
        let refusal = moved
            .resolve(
                "reviewer",
                AdjudicationVerdict::Decided {
                    position_id: "held".to_string(),
                    decisive_evidence: EvidenceRef::new(
                        EvidenceKind::File,
                        "src/chat/session.rs:210",
                    ),
                    rationale: "still looks right".to_string(),
                },
                "sha-moved",
                now_ms(),
            )
            .unwrap_err();
        store_resolution(&ledger, &moved).await.unwrap();

        assert!(matches!(refusal, AdjudicationRefusal::Stale { .. }));
        let open = ledger.get("d-open").await.unwrap().unwrap();
        let stale = ledger.get("d-stale").await.unwrap().unwrap();
        assert_eq!(open.status, DisagreementStatus::Open);
        assert!(open.adjudication.is_none());
        assert_eq!(stale.status, DisagreementStatus::Stale);
        assert!(stale.adjudication.is_none());
        assert_ne!(stale.status, DisagreementStatus::Adjudicated);
        assert!(render(&open).contains("no decisive evidence"));
    }
}
