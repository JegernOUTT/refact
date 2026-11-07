//! Decomposition advisor: what fanning work out actually cost.
//!
//! Every finished run records what it used — wall-clock, tokens, turns, tool
//! calls, whether it succeeded, whether the work had to be run again — together
//! with whether it ran alone or as one of several overlapping siblings. The
//! advisor compares those two cohorts for comparable work.
//!
//! **Advisory only.** Nothing here reserves, admits, cancels or reprioritises
//! anything. It answers a question a planner or a human asked; scheduling reads
//! it never.
//!
//! **Honest about confidence.** A cohort below the configured sample size gets
//! a refusal with its sample counts, not a mean over three runs presented as a
//! law. With no usable store the feature reports itself unavailable and the
//! engine is otherwise unaffected.

use std::path::PathBuf;
use std::sync::Arc;

use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use refact_work_core::store::records::DecompositionOutcomeRecord;
use refact_work_core::{
    default_store_path, CoordinationStore, ProjectAccess, ProjectKey, StoreError, WorkKey,
};

use crate::agents::types::{BackgroundAgent, BgAgentStatus};
use crate::app_state::AppState;
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;

use super::error::WorkError;

/// Paths kept per outcome. They exist only so the model-facing surface can be
/// filtered against the destination policy, so a bounded sample is enough.
const MAX_RECORDED_PATHS: usize = 64;

pub const NO_STORE: &str = "no coordination store is available, so no run costs were recorded";

/// One finished run as the recorder saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    pub record_id: String,
    pub work_id: String,
    pub work_class: String,
    pub fan_out_group: Option<String>,
    pub sibling_count: i64,
    pub wall_clock_ms: i64,
    pub tokens: i64,
    pub turns: i64,
    pub tool_calls: i64,
    pub succeeded: bool,
    pub reworked: bool,
    pub recorded_at_ms: i64,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CohortStat {
    pub samples: usize,
    pub median_wall_clock_ms: i64,
    pub mean_wall_clock_ms: i64,
    pub median_tokens: i64,
    pub mean_tokens: i64,
    pub succeeded: usize,
    pub reworked: usize,
}

/// What the numbers say, on both dimensions at once. Wall-clock alone is
/// misleading when a build dominates a run, so a faster cohort that burns more
/// tokens is reported as exactly that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    FanOutCheaper,
    SingleCheaper,
    FanOutFasterButCostlier,
    FanOutSlowerButCheaper,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::FanOutCheaper => "fanning out was both faster and no more expensive in tokens",
            Verdict::SingleCheaper => "one agent was both faster and cheaper in tokens",
            Verdict::FanOutFasterButCostlier => "fanning out was faster but spent more tokens",
            Verdict::FanOutSlowerButCheaper => "fanning out was slower but spent fewer tokens",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Comparison {
    pub work_class: String,
    pub single: CohortStat,
    pub fanned_out: CohortStat,
    /// Fanned-out median minus single median; positive means fan-out was slower.
    pub wall_clock_delta_ms: i64,
    pub tokens_delta: i64,
    pub verdict: Verdict,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Advice {
    Unavailable {
        reason: String,
    },
    InsufficientData {
        work_class: String,
        single_samples: usize,
        fanned_out_samples: usize,
        required: usize,
    },
    Comparison(Comparison),
}

fn median(values: &mut [i64]) -> i64 {
    values.sort_unstable();
    match values.len() {
        0 => 0,
        len if len % 2 == 1 => values[len / 2],
        len => (values[len / 2 - 1] + values[len / 2]) / 2,
    }
}

fn mean(values: &[i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values
        .iter()
        .fold(0i64, |sum, value| sum.saturating_add(*value))
        / values.len() as i64
}

fn cohort(records: &[&DecompositionOutcomeRecord]) -> CohortStat {
    let mut wall: Vec<i64> = records.iter().map(|record| record.wall_clock_ms).collect();
    let mut tokens: Vec<i64> = records.iter().map(|record| record.tokens).collect();
    CohortStat {
        samples: records.len(),
        median_wall_clock_ms: median(&mut wall),
        mean_wall_clock_ms: mean(&wall),
        median_tokens: median(&mut tokens),
        mean_tokens: mean(&tokens),
        succeeded: records.iter().filter(|record| record.succeeded).count(),
        reworked: records.iter().filter(|record| record.reworked).count(),
    }
}

/// Compare the two cohorts of one work class. Either cohort below
/// `min_samples` refuses with both counts instead of reporting a figure.
pub fn aggregate(
    records: &[DecompositionOutcomeRecord],
    work_class: &str,
    min_samples: usize,
) -> Advice {
    let required = min_samples.max(1);
    let (fanned_out, single): (
        Vec<&DecompositionOutcomeRecord>,
        Vec<&DecompositionOutcomeRecord>,
    ) = records
        .iter()
        .filter(|record| record.work_class == work_class)
        .partition(|record| record.fanned_out());
    if single.len() < required || fanned_out.len() < required {
        return Advice::InsufficientData {
            work_class: work_class.to_string(),
            single_samples: single.len(),
            fanned_out_samples: fanned_out.len(),
            required,
        };
    }
    let single = cohort(&single);
    let fanned_out = cohort(&fanned_out);
    let wall_clock_delta_ms = fanned_out.median_wall_clock_ms - single.median_wall_clock_ms;
    let tokens_delta = fanned_out.median_tokens - single.median_tokens;
    let verdict = match (wall_clock_delta_ms < 0, tokens_delta <= 0) {
        (true, true) => Verdict::FanOutCheaper,
        (false, false) => Verdict::SingleCheaper,
        (true, false) => Verdict::FanOutFasterButCostlier,
        (false, true) => Verdict::FanOutSlowerButCheaper,
    };
    Advice::Comparison(Comparison {
        work_class: work_class.to_string(),
        single,
        fanned_out,
        wall_clock_delta_ms,
        tokens_delta,
        verdict,
    })
}

/// Model-facing text. A refusal reads as a refusal, never as a number.
pub fn render(advice: &Advice) -> String {
    match advice {
        Advice::Unavailable {
            reason,
        } => {
            format!("Decomposition advisor unavailable: {reason}.")
        }
        Advice::InsufficientData {
            work_class,
            single_samples,
            fanned_out_samples,
            required,
        } => format!(
            "Decomposition advisor: insufficient data for `{work_class}` — \
             {single_samples} single-agent and {fanned_out_samples} fanned-out run(s) recorded, \
             {required} of each required. No comparison is offered."
        ),
        Advice::Comparison(comparison) => {
            let single = &comparison.single;
            let fanned_out = &comparison.fanned_out;
            format!(
                "Decomposition advisor for `{}` ({} single-agent run(s), {} fanned-out run(s)):\n\
                 - single: median {} ms, median {} tokens, {}/{} succeeded, {} reworked\n\
                 - fanned out: median {} ms, median {} tokens, {}/{} succeeded, {} reworked\n\
                 - difference: {} ms and {} tokens; {}. Advisory only.",
                comparison.work_class,
                single.samples,
                fanned_out.samples,
                single.median_wall_clock_ms,
                single.median_tokens,
                single.succeeded,
                single.samples,
                single.reworked,
                fanned_out.median_wall_clock_ms,
                fanned_out.median_tokens,
                fanned_out.succeeded,
                fanned_out.samples,
                fanned_out.reworked,
                comparison.wall_clock_delta_ms,
                comparison.tokens_delta,
                comparison.verdict.as_str()
            )
        }
    }
}

/// The recorded outcomes of one project, in the coordination store.
pub struct OutcomeLedger {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
}

impl OutcomeLedger {
    pub fn new(store: Arc<CoordinationStore>, project: ProjectKey) -> Self {
        let access = store.authorize(project);
        OutcomeLedger {
            store,
            access,
        }
    }

    pub fn store_path(&self) -> &std::path::Path {
        self.store.path()
    }

    /// Persist one outcome. Replaying the same run overwrites its row, so a
    /// repeated terminal notification can never inflate a cohort.
    pub async fn record(&self, outcome: &RunOutcome) -> Result<(), StoreError> {
        let record = DecompositionOutcomeRecord {
            project: self.access.project().clone(),
            record_id: outcome.record_id.clone(),
            work: WorkKey::background_agent(outcome.work_id.clone()),
            work_class: outcome.work_class.clone(),
            fan_out_group: outcome.fan_out_group.clone(),
            sibling_count: outcome.sibling_count,
            wall_clock_ms: outcome.wall_clock_ms,
            tokens: outcome.tokens,
            turns: outcome.turns,
            tool_calls: outcome.tool_calls,
            succeeded: outcome.succeeded,
            reworked: outcome.reworked,
            recorded_at_ms: outcome.recorded_at_ms,
            paths: outcome.paths.clone(),
        };
        self.store
            .put_decomposition_outcome(&self.access, &record)
            .await
    }

    pub async fn outcomes(
        &self,
        work_class: Option<&str>,
    ) -> Result<Vec<DecompositionOutcomeRecord>, StoreError> {
        self.store
            .list_decomposition_outcomes(&self.access, work_class)
            .await
    }

    pub async fn retain(&self, max_records: usize) -> Result<usize, StoreError> {
        self.store
            .prune_decomposition_outcomes(&self.access, max_records.max(1))
            .await
    }
}

/// Open this project's ledger. Coordination being off, or a store that cannot
/// be opened, leaves it absent: the advisor is unavailable and nothing else
/// changes.
pub async fn ledger_for(app: &AppState) -> Option<OutcomeLedger> {
    let settings = crate::file_activity::settings();
    if !settings.enabled {
        return None;
    }
    let path = default_store_path(&app.paths.cache_dir);
    let project = super::outbox::project_key_for_root(app, None).await;
    match super::outbox::store_for_path(&path, &settings).await {
        Ok(store) => Some(OutcomeLedger::new(store, project)),
        Err(error) => {
            super::outbox::note_durability_unavailable(&path, error.to_string());
            None
        }
    }
}

fn min_samples() -> usize {
    crate::runtime_settings::current()
        .coordination
        .decomposition_min_samples
}

fn retention_cap() -> usize {
    crate::runtime_settings::current()
        .coordination
        .store_max_decomposition_outcomes
}

/// Read-only projection: what the recorded runs of one work class say. Reading
/// it mutates no scheduling state.
pub async fn advise_with(
    ledger: Option<&OutcomeLedger>,
    work_class: &str,
    min_samples: usize,
) -> Advice {
    let Some(ledger) = ledger else {
        return Advice::Unavailable {
            reason: NO_STORE.to_string(),
        };
    };
    match ledger.outcomes(Some(work_class)).await {
        Ok(records) => aggregate(&records, work_class, min_samples),
        Err(error) => {
            super::outbox::note_durability_unavailable(ledger.store_path(), error.to_string());
            Advice::Unavailable {
                reason: NO_STORE.to_string(),
            }
        }
    }
}

pub async fn advise(app: &AppState, work_class: &str) -> Advice {
    let ledger = ledger_for(app).await;
    advise_with(ledger.as_ref(), work_class, min_samples()).await
}

/// The same projection, filtered for the model that is about to read it.
pub async fn advise_for_model(
    app: &AppState,
    model: Option<&BaseModelRecord>,
    work_class: &str,
) -> Advice {
    let Some(ledger) = ledger_for(app).await else {
        return Advice::Unavailable {
            reason: NO_STORE.to_string(),
        };
    };
    let records = match ledger.outcomes(Some(work_class)).await {
        Ok(records) => records,
        Err(error) => {
            super::outbox::note_durability_unavailable(ledger.store_path(), error.to_string());
            return Advice::Unavailable {
                reason: NO_STORE.to_string(),
            };
        }
    };
    match clear_for_model(&app.gcx, model, records) {
        Ok(visible) => aggregate(&visible, work_class, min_samples()),
        Err(error) => Advice::Unavailable {
            reason: error.message,
        },
    }
}

struct OutcomeAudit {
    records: Vec<(usize, FileRecord)>,
}

impl PrivacyAudited for OutcomeAudit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(self.records.clone())
    }
}

/// Destination-model filtering, applied before any of this reaches a model. An
/// outcome naming a path the policy will not send to this model is dropped
/// whole — its numbers describe that path's work — and an unusable policy or an
/// unknown model clears nothing.
pub fn clear_for_model(
    gcx: &Arc<GlobalContext>,
    model: Option<&BaseModelRecord>,
    records: Vec<DecompositionOutcomeRecord>,
) -> Result<Vec<DecompositionOutcomeRecord>, WorkError> {
    let Some(model) = model.filter(|model| !model.id.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let before = super::privacy_guard::policy_key(gcx, model)?;
    let visible: Vec<DecompositionOutcomeRecord> = records
        .into_iter()
        .filter(|record| {
            let paths = record.paths.iter().map(PathBuf::from);
            let Ok(file_records) = crate::privacy::records::declared_file_records(gcx, paths)
            else {
                return false;
            };
            crate::privacy::destinations::clear_for_model(
                gcx,
                OutcomeAudit {
                    records: file_records.into_iter().enumerate().collect(),
                },
                model,
            )
            .is_ok()
        })
        .collect();
    if super::privacy_guard::policy_key(gcx, model)? != before {
        return Err(WorkError::policy_unavailable());
    }
    Ok(visible)
}

fn run_window(record: &BackgroundAgent) -> (i64, i64) {
    let start = record
        .started_at
        .unwrap_or(record.created_at)
        .timestamp_millis();
    let end = record
        .finished_at
        .unwrap_or(record.last_update_at)
        .timestamp_millis();
    (start, end.max(start))
}

fn fan_out_group(record: &BackgroundAgent) -> String {
    record
        .parent_root_chat_id
        .clone()
        .unwrap_or_else(|| record.parent_chat_id.clone())
}

/// Facts from one finished child, with its fan-out context read off the runs
/// that actually overlapped it under the same parent.
pub fn outcome_from_agent(
    record: &BackgroundAgent,
    siblings: &[BackgroundAgent],
    tool_calls: i64,
) -> RunOutcome {
    let (start, end) = run_window(record);
    let group = fan_out_group(record);
    let overlapping = siblings
        .iter()
        .filter(|sibling| sibling.agent_id != record.agent_id)
        .filter(|sibling| fan_out_group(sibling) == group)
        .filter(|sibling| {
            let (sibling_start, sibling_end) = run_window(sibling);
            sibling_start <= end && start <= sibling_end
        })
        .count();
    let mut paths: Vec<String> = record
        .target_files
        .iter()
        .chain(record.edited_files.iter())
        .cloned()
        .collect();
    paths.sort();
    paths.dedup();
    paths.truncate(MAX_RECORDED_PATHS);
    RunOutcome {
        record_id: format!("{}:{}", record.agent_id, record.run_generation),
        work_id: record.agent_id.clone(),
        work_class: crate::agents::handoff::record_key(record),
        fan_out_group: Some(group),
        sibling_count: 1 + overlapping as i64,
        wall_clock_ms: end - start,
        tokens: record.tokens_used as i64,
        turns: record.step_count as i64,
        tool_calls,
        succeeded: record.status == BgAgentStatus::Completed,
        reworked: record.run_generation > 1 || !record.previous_runs.is_empty(),
        recorded_at_ms: record
            .finished_at
            .unwrap_or(record.last_update_at)
            .timestamp_millis(),
        paths,
    }
}

/// Tool calls the child's live session shows, read without waiting for it. A
/// session another task still holds contributes nothing rather than blocking a
/// terminal notification.
async fn tool_calls_for(app: &AppState, child_chat_id: Option<&str>) -> i64 {
    let Some(chat_id) = child_chat_id else {
        return 0;
    };
    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return 0;
    };
    let Ok(session) = session_arc.try_lock() else {
        return 0;
    };
    session
        .messages
        .iter()
        .filter_map(|message| message.tool_calls.as_ref())
        .map(|calls| calls.len() as i64)
        .sum()
}

/// Record what a finished child cost. Called once per terminal notification and
/// idempotent per run, so repeats overwrite rather than accumulate.
pub async fn observe_agent_outcome(app: &AppState, record: &BackgroundAgent) {
    if !record.status.is_terminal() {
        return;
    }
    let Some(ledger) = ledger_for(app).await else {
        return;
    };
    let siblings = app.agents.list_all().await;
    let tool_calls = tool_calls_for(app, record.child_chat_id.as_deref()).await;
    let outcome = outcome_from_agent(record, &siblings, tool_calls);
    if let Err(error) = ledger.record(&outcome).await {
        super::outbox::note_durability_unavailable(ledger.store_path(), error.to_string());
        return;
    }
    if let Err(error) = ledger.retain(retention_cap()).await {
        tracing::warn!("decomposition outcome retention failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::CoordinationSettings;

    async fn ledger() -> OutcomeLedger {
        let store = Arc::new(
            CoordinationStore::open_in_memory(&CoordinationSettings::default())
                .await
                .unwrap(),
        );
        OutcomeLedger::new(store, ProjectKey::new("alpha"))
    }

    fn agent(agent_id: &str, started_ms: i64, finished_ms: i64) -> BackgroundAgent {
        let started = chrono::DateTime::from_timestamp_millis(started_ms).unwrap();
        let finished = chrono::DateTime::from_timestamp_millis(finished_ms).unwrap();
        let mut record: BackgroundAgent = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "agent_id": agent_id,
            "parent_chat_id": "parent",
            "parent_root_chat_id": "root",
            "parent_tool_call_id": null,
            "child_chat_id": null,
            "kind": "subagent",
            "config_name": "subagent",
            "title": agent_id,
            "prompt": "prompt",
            "target_files": ["src/a.rs"],
            "status": "completed",
            "progress": null,
            "step_count": 7,
            "last_activity": null,
            "result_summary": null,
            "result_payload_path": null,
            "error": null,
            "edited_files": [],
            "diff_summary": null,
            "conflict_summary": null,
            "completion_message_id": null,
            "completion_pushed_at": null,
            "model": "model",
            "created_at": started.to_rfc3339(),
            "started_at": started.to_rfc3339(),
            "finished_at": finished.to_rfc3339(),
            "last_update_at": finished.to_rfc3339(),
            "change_seq": 1
        }))
        .unwrap();
        record.tokens_used = 1_000;
        record
    }

    fn outcome(record_id: &str, sibling_count: i64, wall_clock_ms: i64, tokens: i64) -> RunOutcome {
        RunOutcome {
            record_id: record_id.to_string(),
            work_id: record_id.to_string(),
            work_class: "config:subagent".to_string(),
            fan_out_group: Some("root".to_string()),
            sibling_count,
            wall_clock_ms,
            tokens,
            turns: 5,
            tool_calls: 9,
            succeeded: true,
            reworked: false,
            recorded_at_ms: wall_clock_ms,
            paths: vec!["src/a.rs".to_string()],
        }
    }

    #[tokio::test]
    async fn outcomes_are_filtered_for_the_destination_model_before_a_model_sees_them() {
        let dir = tempfile::tempdir().unwrap();
        let guarded = dir.path().join("guarded.rs");
        std::fs::write(&guarded, "secret").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec![guarded.to_string_lossy().into_owned()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let ledger = ledger().await;
        let mut guarded_run = outcome("guarded-run", 3, 10_000, 9_000);
        guarded_run.paths = vec![guarded.to_string_lossy().into_owned()];
        ledger.record(&guarded_run).await.unwrap();
        ledger
            .record(&outcome("open-run", 1, 30_000, 4_000))
            .await
            .unwrap();
        let records = ledger.outcomes(None).await.unwrap();

        let trusted = clear_for_model(
            &gcx,
            Some(&BaseModelRecord {
                id: "trusted/model".into(),
                ..Default::default()
            }),
            records.clone(),
        )
        .unwrap();
        let other = clear_for_model(
            &gcx,
            Some(&BaseModelRecord {
                id: "other/model".into(),
                ..Default::default()
            }),
            records.clone(),
        )
        .unwrap();
        let unknown_model = clear_for_model(&gcx, None, records.clone()).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(trusted.len(), 2);
        assert_eq!(
            other.len(),
            1,
            "a model the zone does not send to must not see that run at all"
        );
        assert_eq!(other[0].record_id, "open-run");
        assert!(unknown_model.is_empty());
        assert!(matches!(
            aggregate(&other, "config:subagent", 1),
            Advice::InsufficientData { .. }
        ));
    }
}
