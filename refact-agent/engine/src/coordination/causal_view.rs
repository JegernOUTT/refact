//! Reading the causal DAG (Addition F, part 2): the viewer's projection and
//! the replay of recorded coordination decisions.
//!
//! **Replay does not reproduce model output.** It cannot: the same prompt does
//! not yield the same tokens. What replays identically is the *coordination*
//! layer — the order events were admitted into a scope, the edges the recorder
//! resolved for them, and the completeness verdict served with the trace. The
//! recorded events are the inputs, the recorded [`CausalTrace`] is the outcome,
//! and a mismatch between them is a finding naming the diverging step.
//!
//! **Incomplete stays incomplete.** A gap is a gap: a cause naming an event the
//! set does not hold is reported, never bridged. A gap is also not a divergence
//! — only a cause naming an event the set *does* hold, admitted at or after the
//! event it supposedly caused, is something the recorder could not have
//! produced.
//!
//! **Privacy applies before reconstruction**, so a withheld row surfaces as a
//! gap rather than a silently shortened chain, and never reaches the replay.
//!
//! Recording stays in [`super::causal`]; this module only reads.

use std::collections::{HashMap, HashSet};

use refact_work_core::CausalEventRecord;

use crate::app_state::AppState;

use super::causal::{self, CausalTrace};
use super::error::WorkError;
use super::paging::ViewBinding;
use super::scope::WorkCaller;

/// What replay does, in the response, so no caller has to infer it.
pub const REPLAY_SCOPE: &str = "Replay re-runs the recorded coordination decisions only: admission order, resolved causes and the completeness verdict. It does not re-run any model and does not reproduce model output.";

/// Gaps reported on one page. A trace missing thousands of ancestors is
/// summarised by its count, not by thousands of rows.
pub const MAX_REPORTED_GAPS: usize = 64;

/// Findings reported on one page, bounded for the same reason.
pub const MAX_REPORTED_FINDINGS: usize = 64;

/// A coordination decision the recorded trace claims, that replaying the
/// recorded inputs does not produce.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayDivergence {
    /// The trace presents its steps in an order the recorded sequence numbers
    /// do not produce.
    AdmissionOrder { recorded: String, replayed: String },
    /// A cause names an event that was admitted at or after the event it
    /// supposedly caused. The recorder resolves causes from events already
    /// admitted, so it could never have produced this edge.
    CauseNotYetAdmitted { cause: String },
    /// The completeness verdict served with the trace is not the verdict the
    /// recorded events produce.
    GateVerdict { recorded: bool, replayed: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReplayFinding {
    /// The diverging step, named.
    pub step: String,
    pub scope: String,
    pub divergence: ReplayDivergence,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReplayReport {
    pub root: String,
    /// Recorded steps fed back through the coordination state machine.
    pub steps: usize,
    pub findings: Vec<ReplayFinding>,
    pub finding_count: usize,
    /// Always false. Stated rather than implied.
    pub reproduces_model_output: bool,
    pub scope_note: String,
}

impl ReplayReport {
    pub fn reproduced(&self) -> bool {
        self.finding_count == 0
    }
}

/// Recorded admission order: the sequence the recorder assigned, which is
/// monotonic per recorder and therefore a total order over recorded events.
fn admission_order(events: &[CausalEventRecord]) -> Vec<&CausalEventRecord> {
    let mut ordered: Vec<&CausalEventRecord> = events.iter().collect();
    ordered.sort_by(|left, right| {
        left.seq
            .cmp(&right.seq)
            .then(left.event_id.cmp(&right.event_id))
    });
    ordered
}

/// Feed the recorded inputs back through the coordination state machine and
/// compare with the outcome that was recorded.
pub fn replay(events: &[CausalEventRecord], recorded: &CausalTrace) -> ReplayReport {
    let held: HashMap<&str, &CausalEventRecord> = events
        .iter()
        .map(|event| (event.event_id.as_str(), event))
        .collect();
    let replayed_order = admission_order(&recorded.events);
    let mut findings: Vec<ReplayFinding> = Vec::new();

    for event in &replayed_order {
        for cause in event.causes_or_unknown() {
            let Some(id) = cause.event_id() else { continue };
            // An absent cause is a gap the trace already reports honestly, not
            // a decision the machine got wrong.
            let Some(parent) = held.get(id) else { continue };
            if (parent.seq, parent.event_id.as_str()) >= (event.seq, event.event_id.as_str()) {
                findings.push(ReplayFinding {
                    step: event.event_id.clone(),
                    scope: event.scope.clone(),
                    divergence: ReplayDivergence::CauseNotYetAdmitted {
                        cause: id.to_string(),
                    },
                });
            }
        }
    }

    // The trace presents newest first, so oldest first is the order in which
    // the recorder admitted them.
    for (presented, replayed) in recorded.events.iter().rev().zip(replayed_order.iter()) {
        if presented.event_id != replayed.event_id {
            findings.push(ReplayFinding {
                step: presented.event_id.clone(),
                scope: presented.scope.clone(),
                divergence: ReplayDivergence::AdmissionOrder {
                    recorded: presented.event_id.clone(),
                    replayed: replayed.event_id.clone(),
                },
            });
            break;
        }
    }

    let verdict = causal::reconstruct(events, &recorded.root);
    if verdict.complete != recorded.complete {
        findings.push(ReplayFinding {
            step: recorded.root.clone(),
            scope: held
                .get(recorded.root.as_str())
                .map(|event| event.scope.clone())
                .unwrap_or_default(),
            divergence: ReplayDivergence::GateVerdict {
                recorded: recorded.complete,
                replayed: verdict.complete,
            },
        });
    }

    let finding_count = findings.len();
    findings.truncate(MAX_REPORTED_FINDINGS);
    ReplayReport {
        root: recorded.root.clone(),
        steps: replayed_order.len(),
        findings,
        finding_count,
        reproduces_model_output: false,
        scope_note: REPLAY_SCOPE.to_string(),
    }
}

/// Every recorded event this engine can still see: the live ring, plus the
/// durable ledger so a pre-restart trace is still readable. `false` means the
/// ledger was unavailable and anything older than this process is missing.
pub async fn events_for(app: &AppState) -> (Vec<CausalEventRecord>, bool) {
    let live = causal::recorder().events();
    let Some(ledger) = causal::ledger_for(app).await else {
        return (live, false);
    };
    match ledger.events().await {
        Ok(stored) => {
            let mut merged = live;
            let held: HashSet<String> = merged.iter().map(|event| event.event_id.clone()).collect();
            merged.extend(
                stored
                    .into_iter()
                    .filter(|event| !held.contains(&event.event_id)),
            );
            (merged, true)
        }
        Err(_) => (live, false),
    }
}

#[derive(Debug, Clone, Default)]
pub struct CausalRequest {
    pub root: String,
    pub rows: Option<usize>,
    pub cursor: Option<String>,
}

/// One bounded page of a reconstructed trace, with the replay of the decisions
/// it records.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CausalPage {
    pub snapshot_id: String,
    pub seq: u64,
    pub project: String,
    pub root: String,
    /// This page of the trace, newest recorded step first.
    pub events: Vec<CausalEventRecord>,
    pub total: usize,
    pub returned: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// True only when every ancestor was found *and* the durable ledger was
    /// readable. Either one missing leaves a trace that is not the whole story.
    pub complete: bool,
    pub gaps: Vec<String>,
    pub gap_count: usize,
    /// False when the durable ledger could not be read, so everything older
    /// than this process is absent from the page.
    pub durable: bool,
    pub generated_at_ms: i64,
    pub replay: ReplayReport,
}

fn fingerprint(
    trace: &CausalTrace,
    replay: &ReplayReport,
    durable: bool,
) -> Result<String, WorkError> {
    serde_json::to_string(&(trace, replay, durable))
        .map(|value| crate::file_activity::digest_of(&value))
        .map_err(|_| WorkError::unavailable())
}

/// The trace of one recorded event, filtered for this caller and paged.
pub async fn page(
    app: &AppState,
    caller: &WorkCaller,
    request: CausalRequest,
) -> Result<CausalPage, WorkError> {
    let root = request.root.trim().to_string();
    if root.is_empty() {
        return Err(WorkError::invalid(
            "a causal trace requires a root event id",
        ));
    }
    let settings = crate::file_activity::settings();
    let disclosure = match &caller.model {
        Some(model) => super::privacy_guard::policy_key(&app.gcx, model)?,
        None => "operator".to_string(),
    };
    let (recorded, durable) = events_for(app).await;
    // Filtering precedes reconstruction so a withheld row becomes a gap the
    // viewer shows, never a chain that quietly skips it.
    let visible = match &caller.model {
        Some(model) => {
            let cleared = causal::clear_for_model(&app.gcx, Some(model), recorded)?;
            if super::privacy_guard::policy_key(&app.gcx, model)? != disclosure {
                return Err(WorkError::policy_unavailable());
            }
            cleared
        }
        None => recorded,
    };
    let trace = causal::reconstruct(&visible, &root);
    let replay = replay(&visible, &trace);
    let window = app.work_views.page(
        ViewBinding {
            project: super::roots::project_identity(&caller.source_root),
            principal: caller.principal.clone(),
            disclosure,
            filter: format!("causal:{root}"),
        },
        fingerprint(&trace, &replay, durable)?,
        trace.events.len(),
        request.rows.unwrap_or(settings.overview_default_rows),
        request.cursor.as_deref(),
        &settings,
        super::service::now_ms(),
    )?;
    let events = trace.events[window.offset..window.end].to_vec();
    let gap_count = trace.gaps.len();
    let mut gaps = trace.gaps.clone();
    gaps.truncate(MAX_REPORTED_GAPS);
    Ok(CausalPage {
        snapshot_id: window.snapshot_id,
        seq: window.seq,
        project: super::roots::project_identity(&caller.source_root),
        root,
        returned: events.len(),
        events,
        total: trace.events.len(),
        truncated: window.next_cursor.is_some(),
        next_cursor: window.next_cursor,
        complete: trace.complete && durable,
        gaps,
        gap_count,
        durable,
        generated_at_ms: refact_work_core::now_ms(),
        replay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::{CausalCause, CausalKind, ProjectKey, WorkKey};

    fn event(id: &str, scope: &str, causes: Vec<CausalCause>, seq: i64) -> CausalEventRecord {
        CausalEventRecord {
            project: ProjectKey::new("project"),
            event_id: id.to_string(),
            seq,
            kind: CausalKind::Delivery,
            work: WorkKey::trajectory(scope.to_string()),
            scope: scope.to_string(),
            causes,
            recorded_at_ms: 1_000 + seq,
            paths: Vec::new(),
        }
    }

    fn cause(id: &str) -> CausalCause {
        CausalCause::Event(id.to_string())
    }

    fn run() -> Vec<CausalEventRecord> {
        vec![
            event("delivery:m", "parent", vec![CausalCause::Unknown], 1),
            event("spawn:child", "child", vec![cause("delivery:m")], 2),
            event("evidence:f", "child", vec![cause("spawn:child")], 3),
            event("decision:d", "parent", vec![cause("evidence:f")], 4),
        ]
    }
}
