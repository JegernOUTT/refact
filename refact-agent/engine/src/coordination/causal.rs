//! Causal recording (Addition F, part 1): why something happened, not just when.
//!
//! Every boundary that already exists — a delivery landing, an agent spawning
//! or finishing, a question being asked, an evidence record being written —
//! records one node of a causal DAG: a stable event id, the events that caused
//! it, the work it belongs to and a monotonic sequence. Reading the DAG back
//! answers "B did X **because of** message M from A, which came from finding F".
//!
//! **Cheap by construction.** Recording is an O(1) push into a bounded
//! in-memory ring; durability is an additive best-effort write to the M3
//! coordination store. A recorder that cannot persist never fails, delays or
//! changes the operation it is recording — the operation proceeds and the
//! trace reports itself incomplete.
//!
//! **Honest gaps.** An event whose cause the producer does not know records
//! `unknown`. A parent is never guessed, and a trace that lost events to
//! retention, or that walks into an unknown or missing cause, is labelled
//! incomplete rather than presented as the whole story.
//!
//! **Privacy applies.** Trace rows are metadata about work and pass the same
//! destination filtering as the work projection before reaching a model.
//!
//! Evidence records (V-1) are the seam, not a dependency: an evidence producer
//! records through [`record`] with [`CausalKind::Evidence`] and whatever causes
//! it knows. Nothing here requires evidence to exist.
//!
//! The viewer and replay are deliberately not here (card O-3b); this layer only
//! records and reconstructs.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use refact_work_core::{
    default_store_path, CausalCause, CausalEventRecord, CausalKind, CoordinationStore,
    ProjectAccess, ProjectKey, StoreError, WorkKey,
};

use crate::app_state::AppState;
use crate::caps::BaseModelRecord;
use crate::chat::types::PendingDelivery;
use crate::global_context::GlobalContext;

use super::error::WorkError;

/// Causes retained per event. Causality is a DAG, but an event with hundreds of
/// parents is a bug, not a finding.
pub const MAX_CAUSES: usize = 32;

/// Paths retained per event, kept only for destination filtering.
pub const MAX_PATHS: usize = 64;

/// What a boundary hands the recorder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CausalEvent {
    pub event_id: String,
    pub kind: CausalKind,
    /// Chat or agent this happened in.
    pub scope: String,
    pub work: WorkKey,
    pub causes: Vec<CausalCause>,
    pub paths: Vec<String>,
}

impl CausalEvent {
    pub fn new(event_id: impl Into<String>, kind: CausalKind, scope: impl Into<String>) -> Self {
        let scope = scope.into();
        Self {
            event_id: event_id.into(),
            kind,
            work: WorkKey::trajectory(scope.clone()),
            scope,
            causes: vec![CausalCause::Unknown],
            paths: Vec::new(),
        }
    }

    /// Causes as the producer knows them. An empty list is recorded as one
    /// `unknown` cause, never as an event that was caused by nothing.
    pub fn caused_by(mut self, causes: Vec<CausalCause>) -> Self {
        self.causes = if causes.is_empty() {
            vec![CausalCause::Unknown]
        } else {
            causes.into_iter().take(MAX_CAUSES).collect()
        };
        self
    }

    pub fn with_work(mut self, work: WorkKey) -> Self {
        self.work = work;
        self
    }

    pub fn with_paths(mut self, paths: Vec<String>) -> Self {
        self.paths = paths.into_iter().take(MAX_PATHS).collect();
        self
    }

    fn into_record(self, project: ProjectKey, seq: i64, recorded_at_ms: i64) -> CausalEventRecord {
        CausalEventRecord {
            project,
            event_id: self.event_id,
            seq,
            kind: self.kind,
            work: self.work,
            scope: self.scope,
            causes: self.causes,
            recorded_at_ms,
            paths: self.paths,
        }
    }
}

/// What a reconstruction found, and what it could not find.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CausalTrace {
    pub root: String,
    /// The root and every recorded ancestor, newest sequence first.
    pub events: Vec<CausalEventRecord>,
    /// False when any cause on the walk is unknown or missing. A trace with
    /// gaps is never presented as the whole story.
    pub complete: bool,
    pub gaps: Vec<String>,
}

/// A bounded, process-local ring of causal events.
///
/// Pushing is O(1) and never fails. Overflow evicts the oldest event and is
/// counted, so a trace that needed an evicted event reports itself incomplete
/// instead of silently shortening.
pub struct CausalRecorder {
    events: Mutex<VecDeque<CausalEventRecord>>,
    dropped: AtomicUsize,
    seq: AtomicI64,
}

impl Default for CausalRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl CausalRecorder {
    pub fn new() -> Self {
        CausalRecorder {
            events: Mutex::new(VecDeque::new()),
            dropped: AtomicUsize::new(0),
            seq: AtomicI64::new(1),
        }
    }

    fn next_seq(&self) -> i64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Push one event and enforce the retention bound. Returns the stored
    /// record so a caller can persist exactly what was recorded.
    pub fn push(&self, record: CausalEventRecord, capacity: usize) -> CausalEventRecord {
        let capacity = capacity.max(1);
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(position) = events
            .iter()
            .position(|held| held.event_id == record.event_id)
        {
            events.remove(position);
        }
        events.push_back(record.clone());
        while events.len() > capacity {
            events.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        record
    }

    /// The last event recorded in `scope`. This is a recorded fact, not an
    /// inference: it is the event this scope is currently acting on.
    pub fn active_cause(&self, scope: &str) -> Option<String> {
        let events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        events
            .iter()
            .rev()
            .find(|event| event.scope == scope)
            .map(|event| event.event_id.clone())
    }

    /// Causes for an event produced by `scope`: the event this scope is acting
    /// on, or `unknown` when nothing was recorded for it.
    pub fn causes_from(&self, scope: &str) -> Vec<CausalCause> {
        match self.active_cause(scope) {
            Some(event_id) => vec![CausalCause::Event(event_id)],
            None => vec![CausalCause::Unknown],
        }
    }

    pub fn events(&self) -> Vec<CausalEventRecord> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn events_in_scope(&self, scope: &str) -> Vec<CausalEventRecord> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|event| event.scope == scope)
            .cloned()
            .collect()
    }

    /// Events evicted by retention. Any trace reaching back past them is
    /// incomplete.
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn trace(&self, root: &str) -> CausalTrace {
        let mut trace = reconstruct(&self.events(), root);
        if self.dropped() > 0 && !trace.gaps.is_empty() {
            trace.complete = false;
        }
        trace
    }
}

/// The process-wide recorder every boundary writes through.
pub fn recorder() -> &'static CausalRecorder {
    static RECORDER: OnceLock<CausalRecorder> = OnceLock::new();
    RECORDER.get_or_init(CausalRecorder::new)
}

pub fn retention_cap() -> usize {
    crate::runtime_settings::current()
        .coordination
        .store_max_causal_events
}

/// Walk the DAG from `root` back through every recorded cause.
///
/// A cause that is `unknown`, or that names an event this set does not hold, is
/// reported as a gap and makes the trace incomplete. Cycles cannot expand the
/// walk twice: a visited event is never revisited.
pub fn reconstruct(events: &[CausalEventRecord], root: &str) -> CausalTrace {
    let mut trace = CausalTrace {
        root: root.to_string(),
        events: Vec::new(),
        complete: true,
        gaps: Vec::new(),
    };
    let Some(start) = events.iter().find(|event| event.event_id == root) else {
        trace.complete = false;
        trace.gaps.push(root.to_string());
        super::metrics::record_trace_completeness(false);
        return trace;
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut frontier = vec![start.clone()];
    seen.insert(start.event_id.clone());
    while let Some(event) = frontier.pop() {
        for cause in event.causes_or_unknown() {
            match cause.event_id() {
                None => {
                    trace.complete = false;
                    trace.gaps.push(format!("{}:unknown", event.event_id));
                }
                Some(id) if seen.contains(id) => {}
                Some(id) => match events.iter().find(|held| held.event_id == id) {
                    Some(parent) => {
                        seen.insert(parent.event_id.clone());
                        frontier.push(parent.clone());
                    }
                    None => {
                        trace.complete = false;
                        trace.gaps.push(id.to_string());
                    }
                },
            }
        }
        trace.events.push(event);
    }
    trace.events.sort_by(|left, right| {
        right
            .seq
            .cmp(&left.seq)
            .then(right.event_id.cmp(&left.event_id))
    });
    super::metrics::record_trace_completeness(trace.complete);
    trace
}

/// The recorded causal events of one project, in the coordination store.
pub struct CausalLedger {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
}

impl CausalLedger {
    pub fn new(store: Arc<CoordinationStore>, project: ProjectKey) -> Self {
        let access = store.authorize(project);
        CausalLedger {
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

    pub async fn record(&self, record: &CausalEventRecord) -> Result<(), StoreError> {
        self.store.put_causal_event(&self.access, record).await
    }

    pub async fn events(&self) -> Result<Vec<CausalEventRecord>, StoreError> {
        self.store.list_causal_events(&self.access).await
    }

    pub async fn retain(&self, max_records: usize) -> Result<usize, StoreError> {
        self.store
            .prune_causal_events(&self.access, max_records.max(1))
            .await
    }
}

/// Open this project's causal ledger. Coordination being off, or a store that
/// cannot be opened, leaves it absent: recording stays in memory and says so.
pub async fn ledger_for(app: &AppState) -> Option<CausalLedger> {
    let settings = crate::file_activity::settings();
    if !settings.enabled {
        return None;
    }
    let path = default_store_path(&app.paths.cache_dir);
    let project = super::outbox::project_key_for_root(app, None).await;
    match super::outbox::store_for_path(&path, &settings).await {
        Ok(store) => Some(CausalLedger::new(store, project)),
        Err(error) => {
            super::outbox::note_durability_unavailable(&path, error.to_string());
            None
        }
    }
}

/// Record one event. Always succeeds: the in-memory ring takes it first, then
/// durability is attempted. `false` means the event was recorded but not
/// persisted, so a trace rebuilt after a restart will be incomplete.
pub async fn record(app: &AppState, event: CausalEvent) -> bool {
    let project = super::outbox::project_key_for_root(app, None).await;
    let seq = recorder().next_seq();
    let stored = recorder().push(
        event.into_record(project, seq, refact_work_core::now_ms()),
        retention_cap(),
    );
    let Some(ledger) = ledger_for(app).await else {
        return false;
    };
    if let Err(error) = ledger.record(&stored).await {
        super::outbox::note_durability_unavailable(ledger.store_path(), error.to_string());
        return false;
    }
    if let Err(error) = ledger.retain(retention_cap()).await {
        tracing::warn!("causal event retention failed: {error}");
    }
    true
}

/// Causal identity of a delivery. An envelope with no stamp, and a stamp with
/// no `caused_by`, both read as an unknown cause.
pub fn delivery_event(chat_id: &str, delivery: &PendingDelivery) -> CausalEvent {
    let (event_id, causes) = match &delivery.causal {
        Some(stamp) if !stamp.event_id.trim().is_empty() => (
            stamp.event_id.clone(),
            stamp
                .caused_by
                .iter()
                .filter(|id| !id.trim().is_empty())
                .map(|id| CausalCause::Event(id.clone()))
                .collect::<Vec<_>>(),
        ),
        _ => (format!("delivery:{}", delivery.id), Vec::new()),
    };
    CausalEvent::new(event_id, CausalKind::Delivery, chat_id).caused_by(causes)
}

/// A child was spawned by `parent_chat_id`, caused by whatever that chat was
/// acting on. The event is scoped to the child, so everything the child does
/// afterwards chains back through it.
pub async fn record_spawn(app: &AppState, parent_chat_id: &str, child_chat_id: &str) {
    let causes = recorder().causes_from(parent_chat_id);
    let event = CausalEvent::new(
        format!("spawn:{child_chat_id}"),
        CausalKind::Spawn,
        child_chat_id,
    )
    .caused_by(causes);
    record(app, event).await;
}

/// A child reached a terminal state, caused by whatever it was last acting on.
pub async fn record_completion(app: &AppState, agent_id: &str, child_chat_id: Option<&str>) {
    let scope = child_chat_id.unwrap_or(agent_id);
    let causes = recorder().causes_from(scope);
    let event = CausalEvent::new(
        format!("completion:{agent_id}"),
        CausalKind::Completion,
        scope,
    )
    .with_work(WorkKey::background_agent(agent_id.to_string()))
    .caused_by(causes);
    record(app, event).await;
}

/// A question was asked, caused by whatever the asker was acting on.
pub async fn record_question(app: &AppState, asker_chat_id: &str, question_id: &str) {
    let causes = recorder().causes_from(asker_chat_id);
    let event = CausalEvent::new(
        format!("question:{question_id}"),
        CausalKind::Question,
        asker_chat_id,
    )
    .caused_by(causes);
    record(app, event).await;
}

struct CausalAudit {
    records: Vec<(usize, FileRecord)>,
}

impl PrivacyAudited for CausalAudit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(self.records.clone())
    }
}

/// Destination-model filtering, applied before any trace row reaches a model.
/// An event naming a path the policy will not send to this model is dropped
/// whole, and an unusable policy or an unknown model clears nothing.
pub fn clear_for_model(
    gcx: &Arc<GlobalContext>,
    model: Option<&BaseModelRecord>,
    events: Vec<CausalEventRecord>,
) -> Result<Vec<CausalEventRecord>, WorkError> {
    let Some(model) = model.filter(|model| !model.id.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let before = super::privacy_guard::policy_key(gcx, model)?;
    let visible: Vec<CausalEventRecord> = events
        .into_iter()
        .filter(|event| {
            let paths = event.paths.iter().map(PathBuf::from);
            let Ok(file_records) = crate::privacy::records::declared_file_records(gcx, paths)
            else {
                return false;
            };
            crate::privacy::destinations::clear_for_model(
                gcx,
                CausalAudit {
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

/// The trace of one event, filtered for the model that is about to read it.
pub fn trace_for_model(
    gcx: &Arc<GlobalContext>,
    model: Option<&BaseModelRecord>,
    events: Vec<CausalEventRecord>,
    root: &str,
) -> Result<CausalTrace, WorkError> {
    let visible = clear_for_model(gcx, model, events)?;
    Ok(reconstruct(&visible, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    use refact_privacy::{PrivacyPolicy, ShellBehavior, Zone};

    fn project() -> ProjectKey {
        ProjectKey::new("project")
    }

    fn event(
        id: &str,
        kind: CausalKind,
        scope: &str,
        causes: Vec<CausalCause>,
        seq: i64,
    ) -> CausalEventRecord {
        CausalEvent::new(id, kind, scope)
            .caused_by(causes)
            .into_record(project(), seq, 1_000 + seq)
    }

    fn cause(id: &str) -> CausalCause {
        CausalCause::Event(id.to_string())
    }

    fn chain() -> Vec<CausalEventRecord> {
        vec![
            event("delivery:m", CausalKind::Delivery, "parent", vec![], 1),
            event(
                "spawn:child",
                CausalKind::Spawn,
                "child",
                vec![cause("delivery:m")],
                2,
            ),
            event(
                "evidence:f",
                CausalKind::Evidence,
                "child",
                vec![cause("spawn:child")],
                3,
            ),
            event(
                "decision:d",
                CausalKind::Decision,
                "parent",
                vec![cause("evidence:f")],
                4,
            ),
        ]
    }

    #[test]
    fn a_message_spawn_finding_decision_chain_is_reconstructible_end_to_end() {
        let trace = reconstruct(&chain(), "decision:d");
        let ids: Vec<&str> = trace
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["decision:d", "evidence:f", "spawn:child", "delivery:m"],
            "the decision walks back to the finding, the spawn and the message that caused it"
        );
        assert!(
            !trace.complete,
            "the first delivery has an unknown cause, so the trace is honest about its gap"
        );
        assert_eq!(trace.gaps, vec!["delivery:m:unknown".to_string()]);
    }

    #[test]
    fn an_unknown_cause_is_recorded_as_unknown_and_never_guessed() {
        let recorded = event("delivery:lonely", CausalKind::Delivery, "parent", vec![], 1);
        assert_eq!(recorded.causes, vec![CausalCause::Unknown]);
        assert!(recorded.has_unknown_cause());
        let trace = reconstruct(&[recorded], "delivery:lonely");
        assert!(!trace.complete);
        assert_eq!(trace.gaps, vec!["delivery:lonely:unknown".to_string()]);

        let missing = event(
            "decision:orphan",
            CausalKind::Decision,
            "parent",
            vec![cause("delivery:evicted")],
            2,
        );
        let trace = reconstruct(&[missing], "decision:orphan");
        assert!(!trace.complete);
        assert_eq!(trace.gaps, vec!["delivery:evicted".to_string()]);
    }

    #[test]
    fn a_record_without_causal_fields_reads_as_unknown() {
        let legacy: CausalEventRecord = serde_json::from_value(serde_json::json!({
            "project": "project",
            "event_id": "legacy",
            "seq": 1,
            "kind": "delivery",
            "work": {"runtime": "trajectory", "id": "chat"},
            "scope": "chat",
            "recorded_at_ms": 1
        }))
        .expect("a record without causal fields must load");
        assert!(legacy.causes.is_empty());
        assert_eq!(legacy.causes_or_unknown(), vec![CausalCause::Unknown]);
        assert!(!reconstruct(&[legacy], "legacy").complete);
    }

    #[test]
    fn retention_bounds_hold_and_an_evicted_ancestor_is_reported_as_a_gap() {
        let recorder = CausalRecorder::new();
        for index in 0..8 {
            let seq = recorder.next_seq();
            recorder.push(
                event(
                    &format!("event:{index}"),
                    CausalKind::Delivery,
                    "chat",
                    if index == 0 {
                        vec![]
                    } else {
                        vec![cause(&format!("event:{}", index - 1))]
                    },
                    seq,
                ),
                3,
            );
        }
        assert_eq!(recorder.events().len(), 3, "retention bounds the ring");
        assert_eq!(recorder.dropped(), 5);
        let trace = recorder.trace("event:7");
        assert!(!trace.complete, "an evicted ancestor is never invented");
        assert_eq!(trace.gaps, vec!["event:4".to_string()]);
    }

    #[tokio::test]
    async fn trace_rows_are_filtered_for_the_destination_model() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let guarded = dir.path().join("guarded.rs");
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            zones: vec![Zone {
                name: "guarded".into(),
                patterns: vec![guarded.to_string_lossy().into_owned()],
                send_to: vec!["trusted".into()],
                on_shell_read: ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let mut secret = event("evidence:secret", CausalKind::Evidence, "child", vec![], 1);
        secret.paths = vec![guarded.to_string_lossy().into_owned()];
        let open = event(
            "decision:open",
            CausalKind::Decision,
            "parent",
            vec![cause("evidence:secret")],
            2,
        );
        let events = vec![secret, open];

        let trusted = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        assert_eq!(
            clear_for_model(&gcx, Some(&trusted), events.clone())
                .unwrap()
                .len(),
            2
        );
        let other = BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        let trace = trace_for_model(&gcx, Some(&other), events.clone(), "decision:open").unwrap();
        assert_eq!(trace.events.len(), 1);
        assert!(
            !trace.complete,
            "a row withheld from this model leaves a gap, not a silent shortening"
        );
        assert_eq!(trace.gaps, vec!["evidence:secret".to_string()]);
        assert!(
            clear_for_model(&gcx, None, events).unwrap().is_empty(),
            "an unknown destination clears nothing"
        );
    }
}
