//! Builds the wait-for graph from live parking, wait, question and approval
//! state and walks it for cycles. Every source is snapshotted and released
//! before the walk, so no scheduler or session lock is held while analysing.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use refact_chat_api::{QuestionKind, QuestionState};
use refact_work_core::waitgraph::{
    detect_stable, DeadlockReport, Detection, RunFacts, WaitEdge, WaitGraph, WaitGraphBounds,
    WaitKind, WaitTarget,
};
use refact_work_core::{RunKey, TicketSnapshot, TicketState};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::app_state::AppState;

/// Principals that are the human operator rather than a run.
const USER_PRINCIPALS: [&str; 2] = ["user", "operator"];

fn node_version(app: &AppState, ticket: &TicketSnapshot) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.wait_graph.version_of(&ticket.run).hash(&mut hasher);
    ticket.park_count.hash(&mut hasher);
    ticket.state.hash(&mut hasher);
    ticket.seq.hash(&mut hasher);
    hasher.finish()
}

fn target_for(principal: &str, by_id: &HashMap<String, RunKey>) -> Option<WaitTarget> {
    if USER_PRINCIPALS.contains(&principal) {
        return Some(WaitTarget::User);
    }
    by_id.get(principal).cloned().map(WaitTarget::Run)
}

/// The run the scheduler currently tracks for a principal, if any. An
/// unresolved principal is simply not part of the graph.
pub fn resolve(app: &AppState, principal: &str) -> Option<RunKey> {
    app.work_scheduler
        .snapshot()
        .tickets
        .into_iter()
        .map(|ticket| ticket.run)
        .find(|run| run.work.id == principal)
}

/// Declare an await on one or more agents. An await-any produces an edge to
/// every selected id, so the cycle is only real when every branch is blocked.
pub fn declare_agent_wait(
    app: &AppState,
    waiter: &str,
    background_agent_id: Option<&str>,
    targets: &[String],
) -> refact_work_core::WaitHandle {
    let tickets = app.work_scheduler.snapshot().tickets;
    let by_id: HashMap<&str, &RunKey> = tickets
        .iter()
        .map(|ticket| (ticket.run.work.id.as_str(), &ticket.run))
        .collect();
    let waiter_id = background_agent_id
        .map(str::to_string)
        .unwrap_or_else(|| waiter.to_string());
    let from = by_id
        .get(waiter_id.as_str())
        .map(|run| (*run).clone())
        .unwrap_or_else(|| {
            if waiter_id != waiter {
                refact_work_core::WorkKey::background_agent(waiter_id.clone()).run(0)
            } else {
                refact_work_core::WorkKey::trajectory(waiter).run(0)
            }
        });
    let selected: Vec<WaitTarget> = match by_id.contains_key(waiter_id.as_str()) {
        true => targets
            .iter()
            .filter_map(|id| by_id.get(id.as_str()))
            .map(|run| WaitTarget::Run((*run).clone()))
            .collect(),
        false => Vec::new(),
    };
    app.wait_graph
        .declare(from, WaitKind::AgentWait, selected, "agent_wait")
}

/// One consistent capture of everything that can block a run.
pub fn capture(app: &AppState) -> WaitGraph {
    let Some(child_chat_index) = app.agents.try_child_chat_index() else {
        return WaitGraph {
            incomplete: Some("agent registry busy".to_string()),
            ..WaitGraph::default()
        };
    };
    let bounds = WaitGraphBounds::from_settings();
    let snapshot = app.work_scheduler.snapshot();
    let declared = app.wait_graph.edges();
    let now = Utc::now();
    let questions = app.questions().list(None, now);
    let approvals = app
        .agent_approvals
        .undecided(now.timestamp_millis().max(0) as u64);

    let mut by_id: HashMap<String, RunKey> = HashMap::new();
    let mut runs: Vec<(RunKey, RunFacts)> = Vec::with_capacity(snapshot.tickets.len());
    for ticket in &snapshot.tickets {
        by_id
            .entry(ticket.run.work.id.clone())
            .or_insert_with(|| ticket.run.clone());
        runs.push((
            ticket.run.clone(),
            RunFacts {
                parked: matches!(ticket.state, TicketState::Parked | TicketState::Evicted),
                waiting_for_capacity: matches!(
                    ticket.state,
                    TicketState::Pending | TicketState::Reserved
                ),
                version: node_version(app, ticket),
            },
        ));
    }

    let ceiling = bounds.max_edges.saturating_add(1);
    let mut edges: Vec<WaitEdge> = Vec::new();
    let push = |edge: WaitEdge, edges: &mut Vec<WaitEdge>| {
        if edges.len() < ceiling {
            edges.push(edge);
        }
    };

    for edge in declared {
        push(edge, &mut edges);
    }

    for question in questions {
        if question.state != QuestionState::Pending {
            continue;
        }
        let Some(asker) = by_id.get(&question.asker).cloned() else {
            continue;
        };
        let target = if question.kind == QuestionKind::UserInput {
            Some(WaitTarget::User)
        } else {
            let answerer = child_chat_index
                .get(&question.answerer)
                .unwrap_or(&question.answerer);
            target_for(answerer, &by_id)
        };
        let Some(target) = target else {
            continue;
        };
        push(
            WaitEdge::new(
                asker,
                target,
                WaitKind::Question,
                format!("question {}", question.id),
            ),
            &mut edges,
        );
    }

    for approval in approvals {
        let Some(requester) = by_id.get(&approval.child_work_id).cloned() else {
            continue;
        };
        let Some(target) = target_for(&approval.parent_chat_id, &by_id) else {
            continue;
        };
        push(
            WaitEdge::new(
                requester,
                target,
                WaitKind::Approval,
                format!("approval {} for {}", approval.id, approval.tool_name),
            ),
            &mut edges,
        );
    }

    let holders: Vec<&TicketSnapshot> = snapshot
        .tickets
        .iter()
        .filter(|ticket| ticket.state == TicketState::Active)
        .collect();
    for ticket in &snapshot.tickets {
        if !matches!(ticket.state, TicketState::Pending | TicketState::Reserved) {
            continue;
        }
        for holder in &holders {
            if holder.scope.root != ticket.scope.root
                && holder.scope.project != ticket.scope.project
            {
                continue;
            }
            push(
                WaitEdge::new(
                    ticket.run.clone(),
                    WaitTarget::Run(holder.run.clone()),
                    WaitKind::Capacity,
                    format!("capacity held by {}", holder.run),
                ),
                &mut edges,
            );
        }
    }

    WaitGraph::build(runs, edges, bounds)
}

/// Detect on demand. A positive finding is recorded and a refusable wait in
/// the cycle is refused; cancelling the refused waiter stays opt-in. If no
/// cycle edge has a registered wait, auto-cancel retains the closing-edge source.
pub fn scan(app: &AppState) -> Detection {
    let settings = refact_work_core::coordination_settings();
    let outcome = detect_stable(|| capture(app), settings.deadlock_detection_attempts);
    match &outcome {
        Detection::Deadlock(report) => {
            tracing::warn!("{report}");
            let refused_waiter = app.wait_graph.refuse(report);
            if settings.deadlock_auto_cancel {
                let waiter = refused_waiter.as_ref().unwrap_or(&report.closing_edge.from);
                app.work_scheduler.cancel(waiter);
            }
        }
        Detection::Incomplete(reason) => tracing::warn!("deadlock detection incomplete: {reason}"),
        _ => {}
    }
    outcome
}

/// An owned periodic detector with a cancellation path and a joined shutdown.
pub struct DeadlockDetector {
    cancel: Arc<AtomicBool>,
    notify: Arc<Notify>,
    handle: JoinHandle<()>,
}

impl DeadlockDetector {
    pub fn start(app: AppState) -> Option<DeadlockDetector> {
        let settings = refact_work_core::coordination_settings();
        if !settings.deadlock_detection_enabled {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let interval = Duration::from_millis(settings.deadlock_scan_interval_ms);
        let handle = tokio::spawn({
            let cancel = cancel.clone();
            let notify = notify.clone();
            async move {
                loop {
                    let woken = notify.notified();
                    tokio::pin!(woken);
                    woken.as_mut().enable();
                    if cancel.load(Ordering::Relaxed)
                        || app.runtime.shutdown_flag.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    scan(&app);
                    tokio::select! {
                        _ = &mut woken => {}
                        _ = tokio::time::sleep(interval) => {}
                    }
                }
            }
        });
        Some(DeadlockDetector {
            cancel,
            notify,
            handle,
        })
    }

    pub fn wake(&self) {
        self.notify.notify_waiters();
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    pub async fn shutdown(self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
        if let Err(error) = self.handle.await {
            if !error.is_cancelled() {
                tracing::warn!("deadlock detector task failed during shutdown: {error}");
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WaitGraphView {
    pub nodes: Vec<refact_work_core::WaitNode>,
    pub edges: Vec<WaitEdge>,
    pub withheld_nodes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadlock: Option<DeadlockReport>,
}

/// The graph restricted to runs the caller may already see. A run that the
/// privacy policy withholds is counted, never named.
pub fn visible_view(app: &AppState, visible: &HashSet<String>) -> WaitGraphView {
    let graph = capture(app);
    let total = graph.nodes.len();
    let nodes: Vec<_> = graph
        .nodes
        .into_values()
        .filter(|node| visible.contains(&node.run.work.to_string()))
        .collect();
    let named: HashSet<&RunKey> = nodes.iter().map(|node| &node.run).collect();
    let edges = graph
        .edges
        .into_iter()
        .filter(|edge| {
            named.contains(&edge.from) && edge.to.run().is_some_and(|target| named.contains(target))
        })
        .collect();
    let deadlock = app.wait_graph.latest_report().filter(|report| {
        report
            .participants
            .iter()
            .all(|run| visible.contains(&run.work.to_string()))
    });
    WaitGraphView {
        withheld_nodes: total.saturating_sub(nodes.len()),
        nodes,
        edges,
        incomplete: graph.incomplete,
        deadlock,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::AskQuestion;
    use refact_work_core::{
        AdmissionRequest, AdmissionScheduler, CheckoutKey, CoordinationSettings, ProjectKey,
        Ticket, WorkKey, WorkScope,
    };

    async fn app_with_cap(cap: usize) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut app = AppState::from_gcx(gcx).await;
        app.work_scheduler = Arc::new(AdmissionScheduler::new(CoordinationSettings {
            max_active_engine: cap,
            max_active_project: cap,
            max_active_root: cap,
            ..CoordinationSettings::default()
        }));
        app
    }

    fn run(id: &str) -> RunKey {
        WorkKey::background_agent(id).run(1)
    }

    fn scope(root: &str) -> WorkScope {
        WorkScope::new(ProjectKey::new("project"), CheckoutKey::new(root))
    }

    async fn parked(app: &AppState, id: &str) -> Ticket {
        app.work_scheduler
            .reserve(AdmissionRequest::new(run(id), scope("root")))
            .unwrap()
            .acquire()
            .await
            .unwrap()
            .park()
            .unwrap()
    }

    fn ask(app: &AppState, asker: &str, answerer: &str, kind: QuestionKind) {
        app.questions()
            .ask(
                AskQuestion::new(
                    kind,
                    asker,
                    answerer,
                    format!("{asker}->{answerer}"),
                    "May I continue?",
                ),
                Utc::now(),
            )
            .unwrap();
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn a_parent_waiting_on_a_child_that_asks_it_back_is_reported_with_both_names() {
        let app = app_with_cap(4).await;
        let _parent = parked(&app, "parent").await;
        let _child = parked(&app, "child").await;
        let waiting = app.wait_graph.declare(
            run("parent"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("child"))],
            "agent_wait",
        );
        ask(&app, "child", "parent", QuestionKind::AgentParent);

        let report = match scan(&app) {
            Detection::Deadlock(report) => report,
            other => panic!("expected a deadlock, got {other:?}"),
        };
        assert_eq!(report.participants.len(), 2);
        for id in ["parent", "child"] {
            assert!(report.involves(&run(id)), "{id} must be named");
        }
        let rendered = report.to_string();
        assert!(
            rendered.starts_with(refact_work_core::DEADLOCK_CODE),
            "{rendered}"
        );
        assert!(
            rendered.contains(&report.closing_edge.describe()),
            "{rendered}"
        );
        assert_eq!(
            app.wait_graph.latest_report().as_ref(),
            Some(&report),
            "the finding must be recorded"
        );
        let refused = report.closing_edge.from == run("parent");
        assert_eq!(waiting.refusal().is_some(), refused);
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn a_wait_on_the_user_is_never_a_deadlock() {
        for (kind, answerer) in [
            (QuestionKind::UserInput, "parent"),
            (QuestionKind::PermissionEscalation, "user"),
        ] {
            let app = app_with_cap(4).await;
            let _parent = parked(&app, "parent").await;
            let _child = parked(&app, "child").await;
            let _waiting = app.wait_graph.declare(
                run("parent"),
                WaitKind::AgentWait,
                vec![WaitTarget::Run(run("child"))],
                "agent_wait",
            );
            ask(&app, "child", answerer, kind);
            assert_eq!(scan(&app), Detection::NoDeadlock, "{kind:?}");
        }
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn a_process_or_timer_wait_is_never_a_deadlock() {
        for external in [WaitTarget::Process("pid".into()), WaitTarget::Timer] {
            let app = app_with_cap(4).await;
            let _parent = parked(&app, "parent").await;
            let _child = parked(&app, "child").await;
            let _waiting = app.wait_graph.declare(
                run("parent"),
                WaitKind::AgentWait,
                vec![WaitTarget::Run(run("child"))],
                "agent_wait",
            );
            let _external = app.wait_graph.declare(
                run("child"),
                WaitKind::Child,
                vec![external.clone(), WaitTarget::Run(run("parent"))],
                "external",
            );
            assert_eq!(scan(&app), Detection::NoDeadlock, "{external}");
        }
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn an_await_any_is_only_a_cycle_when_every_branch_is_blocked() {
        for other_is_blocked in [false, true] {
            let app = app_with_cap(4).await;
            let _parent = parked(&app, "parent").await;
            let _child = parked(&app, "child").await;
            let permit = app
                .work_scheduler
                .reserve(AdmissionRequest::new(run("other"), scope("root")))
                .unwrap()
                .acquire()
                .await
                .unwrap();
            let mut active = None;
            let mut idle = None;
            if other_is_blocked {
                idle = Some(permit.park().unwrap());
            } else {
                active = Some(permit);
            }
            let _held = (&active, &idle);
            let _waiting = app.wait_graph.declare(
                run("parent"),
                WaitKind::AgentWait,
                vec![WaitTarget::Run(run("child")), WaitTarget::Run(run("other"))],
                "await_any",
            );
            ask(&app, "child", "parent", QuestionKind::AgentParent);
            let outcome = scan(&app);
            assert_eq!(
                matches!(outcome, Detection::Deadlock(_)),
                other_is_blocked,
                "{outcome:?}"
            );
        }
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn an_await_any_declares_one_edge_per_selected_id() {
        let app = app_with_cap(4).await;
        let _parent = parked(&app, "parent").await;
        let _first = parked(&app, "first").await;
        let _second = parked(&app, "second").await;
        let waiting = declare_agent_wait(
            &app,
            "parent",
            None,
            &[
                "first".to_string(),
                "second".to_string(),
                "gone".to_string(),
            ],
        );
        let declared: Vec<_> = app
            .wait_graph
            .edges()
            .into_iter()
            .filter(|edge| edge.group == waiting.group())
            .collect();
        assert_eq!(
            declared.len(),
            2,
            "unresolved ids are simply not in the graph"
        );
        for id in ["first", "second"] {
            assert!(declared
                .iter()
                .any(|edge| edge.to == WaitTarget::Run(run(id))));
        }
        assert!(declared.iter().all(|edge| edge.from == run("parent")));
        drop(waiting);
        assert!(app.wait_graph.edges().is_empty());

        let untracked = declare_agent_wait(&app, "never-admitted", None, &["first".to_string()]);
        assert!(!untracked.is_tracked());
        assert!(app.wait_graph.edges().is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn a_long_chain_with_a_progressing_tail_is_never_reported() {
        let app = app_with_cap(4).await;
        let _a = parked(&app, "a").await;
        let _b = parked(&app, "b").await;
        let _c = app
            .work_scheduler
            .reserve(AdmissionRequest::new(run("c"), scope("root")))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let _first = app.wait_graph.declare(
            run("a"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("b"))],
            "agent_wait",
        );
        let _second = app.wait_graph.declare(
            run("b"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("c"))],
            "agent_wait",
        );
        assert_eq!(scan(&app), Detection::NoDeadlock);
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn a_pending_approval_is_an_edge_from_the_requester_to_its_approver() {
        let app = app_with_cap(4).await;
        let _parent = parked(&app, "parent").await;
        let _child = parked(&app, "child").await;
        app.agent_approvals
            .register(
                refact_work_core::PendingApproval {
                    id: "request".into(),
                    child_work_id: "child".into(),
                    parent_chat_id: "parent".into(),
                    run_generation: 1,
                    tool_call_id: "call".into(),
                    tool_name: "shell".into(),
                    args: serde_json::json!({}),
                    digest: None,
                    created_at: 0,
                    expires_at: u64::MAX,
                    execution: None,
                },
                0,
            )
            .unwrap();
        let graph = capture(&app);
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.kind == WaitKind::Approval
                && edge.from == run("child")
                && edge.to == WaitTarget::Run(run("parent"))));

        let _waiting = app.wait_graph.declare(
            run("parent"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("child"))],
            "agent_wait",
        );
        assert!(matches!(scan(&app), Detection::Deadlock(_)));
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn a_graph_past_its_bounds_is_truthfully_incomplete() {
        let app = app_with_cap(4).await;
        let _parent = parked(&app, "parent").await;
        let _child = parked(&app, "child").await;
        let _waiting = app.wait_graph.declare(
            run("parent"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("child"))],
            "agent_wait",
        );
        ask(&app, "child", "parent", QuestionKind::AgentParent);
        let guard = crate::tools::settings_guard::SettingsGuard::install(|settings| {
            settings.coordination.deadlock_max_nodes = 1;
        });
        match scan(&app) {
            Detection::Incomplete(reason) => {
                assert!(reason.contains("graph too large"), "{reason}");
                assert!(reason.contains("incomplete"), "{reason}");
            }
            other => panic!("expected an incomplete result, got {other:?}"),
        }
        drop(guard);
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn the_detector_is_cancelled_and_joined_on_shutdown() {
        let app = app_with_cap(4).await;
        let detector = DeadlockDetector::start(app.clone()).expect("detection is enabled");
        assert!(!detector.is_finished());
        detector.wake();
        tokio::time::timeout(Duration::from_secs(5), detector.shutdown())
            .await
            .expect("the detector must be joined, not detached");

        let disabled = crate::tools::settings_guard::SettingsGuard::install(|settings| {
            settings.coordination.deadlock_detection_enabled = false;
        });
        assert!(DeadlockDetector::start(app.clone()).is_none());
        drop(disabled);
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn the_visible_view_names_only_runs_the_caller_may_see() {
        let app = app_with_cap(4).await;
        let _parent = parked(&app, "parent").await;
        let _child = parked(&app, "child").await;
        let _waiting = app.wait_graph.declare(
            run("parent"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("child"))],
            "agent_wait",
        );
        ask(&app, "child", "parent", QuestionKind::AgentParent);
        assert!(matches!(scan(&app), Detection::Deadlock(_)));

        let everything: HashSet<String> = ["parent", "child"]
            .into_iter()
            .map(|id| WorkKey::background_agent(id).to_string())
            .collect();
        let full = visible_view(&app, &everything);
        assert_eq!(full.nodes.len(), 2);
        assert_eq!(full.edges.len(), 2);
        assert_eq!(full.withheld_nodes, 0);
        assert!(full.deadlock.is_some());

        let partial: HashSet<String> = [WorkKey::background_agent("parent").to_string()].into();
        let limited = visible_view(&app, &partial);
        assert_eq!(limited.nodes.len(), 1);
        assert!(limited.edges.is_empty());
        assert_eq!(limited.withheld_nodes, 1);
        assert!(
            limited.deadlock.is_none(),
            "a report naming a withheld run must not be disclosed"
        );
    }
}
