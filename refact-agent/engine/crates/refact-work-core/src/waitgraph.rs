//! The wait-for graph and its cycle detector.
//!
//! Nodes are runs. A directed edge A -> B means "A is blocked until B does
//! something". A wait on a human, a process or a timer is a legitimate
//! terminal-ish state, never a cycle, so such a wait removes its node from the
//! graph instead of joining it.

use crate::identity::RunKey;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

pub const DEADLOCK_CODE: &str = "deadlock_detected";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    Child,
    AgentWait,
    Question,
    Approval,
    Capacity,
}

impl WaitKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WaitKind::Child => "child",
            WaitKind::AgentWait => "agent_wait",
            WaitKind::Question => "question",
            WaitKind::Approval => "approval",
            WaitKind::Capacity => "capacity",
        }
    }

    /// Contention for a permit is not a dependency by itself: the holder will
    /// release it unless the holder is itself blocked.
    pub fn is_advisory(&self) -> bool {
        matches!(self, WaitKind::Capacity)
    }
}

impl fmt::Display for WaitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitTarget {
    Run(RunKey),
    User,
    Process(String),
    Timer,
}

impl WaitTarget {
    pub fn run(&self) -> Option<&RunKey> {
        match self {
            WaitTarget::Run(run) => Some(run),
            _ => None,
        }
    }

    pub fn is_external(&self) -> bool {
        !matches!(self, WaitTarget::Run(_))
    }
}

impl fmt::Display for WaitTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WaitTarget::Run(run) => write!(f, "{run}"),
            WaitTarget::User => f.write_str("user"),
            WaitTarget::Process(id) => write!(f, "process:{id}"),
            WaitTarget::Timer => f.write_str("timer"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitEdge {
    pub from: RunKey,
    pub to: WaitTarget,
    pub kind: WaitKind,
    /// Members of one await-any wait share a group: the wait is only blocked
    /// when every branch of the group is blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u64>,
    pub detail: String,
}

impl WaitEdge {
    pub fn new(from: RunKey, to: WaitTarget, kind: WaitKind, detail: impl Into<String>) -> Self {
        WaitEdge {
            from,
            to,
            kind,
            group: None,
            detail: detail.into(),
        }
    }

    pub fn with_group(mut self, group: u64) -> Self {
        self.group = Some(group);
        self
    }

    pub fn describe(&self) -> String {
        format!("{} -> {} ({})", self.from, self.to, self.kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitGraphBounds {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub attempts: usize,
}

impl Default for WaitGraphBounds {
    fn default() -> Self {
        WaitGraphBounds {
            max_nodes: 4096,
            max_edges: 16384,
            attempts: 3,
        }
    }
}

impl WaitGraphBounds {
    pub fn from_settings() -> Self {
        let settings = crate::coordination_settings();
        WaitGraphBounds {
            max_nodes: settings.deadlock_max_nodes,
            max_edges: settings.deadlock_max_edges,
            attempts: settings.deadlock_detection_attempts,
        }
    }
}

/// What the scheduler knows about a run, before any wait edge is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunFacts {
    pub parked: bool,
    pub waiting_for_capacity: bool,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitNode {
    pub run: RunKey,
    pub blocked: bool,
    pub waiting_on_external: bool,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WaitGraph {
    pub nodes: BTreeMap<RunKey, WaitNode>,
    pub edges: Vec<WaitEdge>,
    pub raw_edges: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<String>,
}

impl WaitGraph {
    pub fn build(
        runs: impl IntoIterator<Item = (RunKey, RunFacts)>,
        raw: Vec<WaitEdge>,
        bounds: WaitGraphBounds,
    ) -> Self {
        let mut nodes: BTreeMap<RunKey, WaitNode> = runs
            .into_iter()
            .map(|(run, facts)| {
                (
                    run.clone(),
                    WaitNode {
                        run,
                        blocked: facts.parked || facts.waiting_for_capacity,
                        waiting_on_external: false,
                        version: facts.version,
                    },
                )
            })
            .collect();
        let raw_edges = raw.len();
        let incomplete = if nodes.len() > bounds.max_nodes {
            Some(format!(
                "graph too large: {} runs exceed the configured bound of {}, deadlock detection incomplete",
                nodes.len(),
                bounds.max_nodes
            ))
        } else if raw_edges > bounds.max_edges {
            Some(format!(
                "graph too large: {raw_edges} wait edges exceed the configured bound of {}, deadlock detection incomplete",
                bounds.max_edges
            ))
        } else {
            None
        };

        for edge in &raw {
            match &edge.to {
                WaitTarget::Run(target) => {
                    if !edge.kind.is_advisory() && nodes.contains_key(target) {
                        if let Some(node) = nodes.get_mut(&edge.from) {
                            node.blocked = true;
                        }
                    }
                }
                _ => {
                    if let Some(node) = nodes.get_mut(&edge.from) {
                        node.waiting_on_external = true;
                    }
                }
            }
        }
        for node in nodes.values_mut() {
            if node.waiting_on_external {
                node.blocked = false;
            }
        }

        let mut viable: HashMap<u64, bool> = HashMap::new();
        for edge in &raw {
            if let Some(group) = edge.group {
                let progressable = edge
                    .to
                    .run()
                    .and_then(|target| nodes.get(target))
                    .is_some_and(|target| target.blocked);
                let entry = viable.entry(group).or_insert(true);
                *entry = *entry && progressable;
            }
        }

        let edges = raw
            .into_iter()
            .filter(|edge| {
                if nodes
                    .get(&edge.from)
                    .is_none_or(|from| from.waiting_on_external)
                {
                    return false;
                }
                let Some(target) = edge.to.run().and_then(|target| nodes.get(target)) else {
                    return false;
                };
                if edge.kind.is_advisory() && !target.blocked {
                    return false;
                }
                edge.group
                    .is_none_or(|group| viable.get(&group).copied().unwrap_or(false))
            })
            .collect();

        WaitGraph {
            nodes,
            edges,
            raw_edges,
            incomplete,
        }
    }

    fn adjacency(&self) -> BTreeMap<RunKey, Vec<usize>> {
        let mut adjacency: BTreeMap<RunKey, Vec<usize>> = BTreeMap::new();
        for (index, edge) in self.edges.iter().enumerate() {
            adjacency.entry(edge.from.clone()).or_default().push(index);
        }
        adjacency
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadlockReport {
    pub participants: Vec<RunKey>,
    pub cycle: Vec<WaitEdge>,
    pub closing_edge: WaitEdge,
    pub attempts: usize,
}

impl DeadlockReport {
    pub fn code(&self) -> &'static str {
        DEADLOCK_CODE
    }

    pub fn involves(&self, run: &RunKey) -> bool {
        self.participants.contains(run)
    }
}

impl fmt::Display for DeadlockReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self
            .participants
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        write!(
            f,
            "{DEADLOCK_CODE}: {} runs wait on each other: {}; the closing edge is {}",
            self.participants.len(),
            names.join(" -> "),
            self.closing_edge.describe()
        )
    }
}

impl std::error::Error for DeadlockReport {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    NoDeadlock,
    Deadlock(DeadlockReport),
    Incomplete(String),
    Inconsistent { attempts: usize },
}

impl Detection {
    pub fn report(&self) -> Option<&DeadlockReport> {
        match self {
            Detection::Deadlock(report) => Some(report),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    Gray,
    Black,
}

struct Frame {
    node: RunKey,
    next: usize,
    incoming: Option<usize>,
}

/// One O(V+E) pass over a graph that was already captured and released.
pub fn detect(graph: &WaitGraph) -> Detection {
    if let Some(reason) = &graph.incomplete {
        return Detection::Incomplete(reason.clone());
    }
    let adjacency = graph.adjacency();
    let empty: Vec<usize> = Vec::new();
    let mut color: HashMap<RunKey, Color> = HashMap::new();
    for start in graph.nodes.keys() {
        if color.contains_key(start) {
            continue;
        }
        let mut depth: HashMap<RunKey, usize> = HashMap::new();
        let mut stack: Vec<Frame> = vec![Frame {
            node: start.clone(),
            next: 0,
            incoming: None,
        }];
        color.insert(start.clone(), Color::Gray);
        depth.insert(start.clone(), 0);
        loop {
            let Some((node, next)) = stack.last().map(|frame| (frame.node.clone(), frame.next))
            else {
                break;
            };
            let outgoing = adjacency.get(&node).unwrap_or(&empty);
            if next >= outgoing.len() {
                color.insert(node.clone(), Color::Black);
                depth.remove(&node);
                stack.pop();
                continue;
            }
            if let Some(frame) = stack.last_mut() {
                frame.next += 1;
            }
            let edge_index = outgoing[next];
            let Some(target) = graph.edges[edge_index].to.run().cloned() else {
                continue;
            };
            match color.get(&target) {
                Some(Color::Black) => {}
                Some(Color::Gray) => {
                    let position = depth.get(&target).copied().unwrap_or(0);
                    let closing_edge = graph.edges[edge_index].clone();
                    let mut cycle: Vec<WaitEdge> = stack[position + 1..]
                        .iter()
                        .filter_map(|frame| frame.incoming)
                        .map(|index| graph.edges[index].clone())
                        .collect();
                    cycle.push(closing_edge.clone());
                    let participants = stack[position..]
                        .iter()
                        .map(|frame| frame.node.clone())
                        .collect();
                    return Detection::Deadlock(DeadlockReport {
                        participants,
                        cycle,
                        closing_edge,
                        attempts: 1,
                    });
                }
                None => {
                    color.insert(target.clone(), Color::Gray);
                    depth.insert(target.clone(), stack.len());
                    stack.push(Frame {
                        node: target,
                        next: 0,
                        incoming: Some(edge_index),
                    });
                }
            }
        }
    }
    Detection::NoDeadlock
}

/// A cycle is only reported when a second capture proves that every
/// participant kept the same state during the walk. A run that changed state
/// invalidates the finding and detection re-runs.
pub fn detect_stable(mut capture: impl FnMut() -> WaitGraph, attempts: usize) -> Detection {
    let attempts = attempts.max(1);
    let mut outcome = Detection::Inconsistent { attempts: 0 };
    for attempt in 1..=attempts {
        let graph = capture();
        match detect(&graph) {
            Detection::Deadlock(mut report) => {
                let verify = capture();
                let stable = report.participants.iter().all(|run| {
                    graph.nodes.get(run).zip(verify.nodes.get(run)).is_some_and(
                        |(before, after)| {
                            before.version == after.version && before.blocked == after.blocked
                        },
                    )
                });
                if stable {
                    report.attempts = attempt;
                    return Detection::Deadlock(report);
                }
                outcome = Detection::Inconsistent { attempts: attempt };
            }
            other => return other,
        }
    }
    outcome
}

struct LiveWait {
    from: RunKey,
    targets: Vec<WaitTarget>,
    kind: WaitKind,
    detail: String,
    refusal: Option<DeadlockReport>,
}

#[derive(Default)]
struct RegistryState {
    next_group: u64,
    waits: BTreeMap<u64, LiveWait>,
    versions: HashMap<RunKey, u64>,
    latest: Option<DeadlockReport>,
}

#[derive(Default)]
struct RegistryInner {
    state: Mutex<RegistryState>,
    notify: Notify,
}

impl RegistryInner {
    fn lock(&self) -> MutexGuard<'_, RegistryState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn bump(state: &mut RegistryState, wait: &LiveWait) {
    let runs = std::iter::once(wait.from.clone()).chain(
        wait.targets
            .iter()
            .filter_map(|target| target.run().cloned()),
    );
    for run in runs {
        *state.versions.entry(run).or_insert(0) += 1;
    }
}

/// Live waits declared by the code that is actually blocked. A declaration
/// lives exactly as long as its guard, so the graph never outlives the wait.
#[derive(Clone, Default)]
pub struct WaitRegistry {
    inner: Arc<RegistryInner>,
}

impl WaitRegistry {
    pub fn declare(
        &self,
        from: RunKey,
        kind: WaitKind,
        targets: Vec<WaitTarget>,
        detail: impl Into<String>,
    ) -> WaitHandle {
        let bounds = WaitGraphBounds::from_settings();
        let mut state = self.inner.lock();
        let edges: usize = state.waits.values().map(|wait| wait.targets.len()).sum();
        if targets.is_empty() || edges.saturating_add(targets.len()) > bounds.max_edges {
            return WaitHandle {
                inner: self.inner.clone(),
                group: None,
            };
        }
        let group = state.next_group.wrapping_add(1);
        state.next_group = group;
        let wait = LiveWait {
            from,
            targets,
            kind,
            detail: detail.into(),
            refusal: None,
        };
        bump(&mut state, &wait);
        if state.versions.len() > bounds.max_edges.saturating_mul(4) {
            let live: Vec<RunKey> = state
                .waits
                .values()
                .flat_map(|wait| {
                    std::iter::once(wait.from.clone()).chain(
                        wait.targets
                            .iter()
                            .filter_map(|target| target.run().cloned()),
                    )
                })
                .collect();
            state.versions.retain(|run, _| live.contains(run));
        }
        state.waits.insert(group, wait);
        drop(state);
        self.inner.notify.notify_waiters();
        WaitHandle {
            inner: self.inner.clone(),
            group: Some(group),
        }
    }

    pub fn edges(&self) -> Vec<WaitEdge> {
        let state = self.inner.lock();
        state
            .waits
            .iter()
            .flat_map(|(group, wait)| {
                wait.targets.iter().map(move |target| {
                    WaitEdge::new(
                        wait.from.clone(),
                        target.clone(),
                        wait.kind,
                        wait.detail.clone(),
                    )
                    .with_group(*group)
                })
            })
            .collect()
    }

    pub fn version_of(&self, run: &RunKey) -> u64 {
        self.inner.lock().versions.get(run).copied().unwrap_or(0)
    }

    pub fn live_waits(&self) -> usize {
        self.inner.lock().waits.len()
    }

    /// Record the finding and refuse the newest edge that closed the loop.
    /// Whether the refused waiter is also cancelled is policy, decided by the
    /// caller; the registry only ever reports.
    pub fn refuse(&self, report: &DeadlockReport) -> bool {
        let mut state = self.inner.lock();
        state.latest = Some(report.clone());
        let group = report.closing_edge.group;
        let refused = match group.and_then(|group| state.waits.get_mut(&group)) {
            Some(wait) if wait.from == report.closing_edge.from => {
                wait.refusal = Some(report.clone());
                true
            }
            _ => false,
        };
        drop(state);
        self.inner.notify.notify_waiters();
        refused
    }

    pub fn latest_report(&self) -> Option<DeadlockReport> {
        self.inner.lock().latest.clone()
    }
}

pub struct WaitHandle {
    inner: Arc<RegistryInner>,
    group: Option<u64>,
}

impl WaitHandle {
    pub fn is_tracked(&self) -> bool {
        self.group.is_some()
    }

    pub fn group(&self) -> Option<u64> {
        self.group
    }

    pub fn refusal(&self) -> Option<DeadlockReport> {
        let group = self.group?;
        self.inner
            .lock()
            .waits
            .get(&group)
            .and_then(|wait| wait.refusal.clone())
    }

    /// Resolves when this wait is refused because it closed a cycle. An
    /// untracked wait never resolves, so it is safe in a `select!` arm.
    pub async fn refused(&self) -> DeadlockReport {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(report) = self.refusal() {
                return report;
            }
            notified.await;
        }
    }
}

impl Drop for WaitHandle {
    fn drop(&mut self) {
        let Some(group) = self.group else {
            return;
        };
        {
            let mut state = self.inner.lock();
            if let Some(wait) = state.waits.remove(&group) {
                bump(&mut state, &wait);
            }
        }
        self.inner.notify.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkKey;

    fn run(id: &str) -> RunKey {
        WorkKey::background_agent(id).run(1)
    }

    fn blocked(version: u64) -> RunFacts {
        RunFacts {
            parked: true,
            waiting_for_capacity: false,
            version,
        }
    }

    fn running() -> RunFacts {
        RunFacts::default()
    }

    fn edge(from: &str, to: &str, kind: WaitKind) -> WaitEdge {
        WaitEdge::new(run(from), WaitTarget::Run(run(to)), kind, "test")
    }

    fn graph(nodes: Vec<(&str, RunFacts)>, edges: Vec<WaitEdge>) -> WaitGraph {
        WaitGraph::build(
            nodes.into_iter().map(|(id, facts)| (run(id), facts)),
            edges,
            WaitGraphBounds::default(),
        )
    }

    #[test]
    fn a_parent_waiting_on_a_child_that_asks_it_a_question_is_a_deadlock() {
        let graph = graph(
            vec![("a", blocked(1)), ("b", blocked(1))],
            vec![
                edge("a", "b", WaitKind::AgentWait).with_group(1),
                edge("b", "a", WaitKind::Question),
            ],
        );
        let report = match detect(&graph) {
            Detection::Deadlock(report) => report,
            other => panic!("expected a deadlock, got {other:?}"),
        };
        assert_eq!(report.participants.len(), 2);
        assert!(report.involves(&run("a")) && report.involves(&run("b")));
        assert_eq!(report.cycle.len(), 2);
        let rendered = report.to_string();
        assert!(rendered.starts_with(DEADLOCK_CODE), "{rendered}");
        for name in [run("a").to_string(), run("b").to_string()] {
            assert!(rendered.contains(&name), "{rendered}");
        }
        assert!(
            rendered.contains(&report.closing_edge.describe()),
            "{rendered}"
        );
    }

    #[test]
    fn a_three_node_loop_through_a_question_and_an_await_any_is_detected() {
        let graph = graph(
            vec![("a", blocked(1)), ("b", blocked(1)), ("c", blocked(1))],
            vec![
                edge("a", "b", WaitKind::AgentWait).with_group(7),
                edge("a", "c", WaitKind::AgentWait).with_group(7),
                edge("b", "c", WaitKind::Question),
                edge("c", "a", WaitKind::Approval),
            ],
        );
        let report = match detect(&graph) {
            Detection::Deadlock(report) => report,
            other => panic!("expected a deadlock, got {other:?}"),
        };
        assert_eq!(report.participants.len(), 3);
        for id in ["a", "b", "c"] {
            assert!(report.involves(&run(id)), "{id} must be named");
        }
    }

    #[test]
    fn a_long_chain_whose_tail_progresses_is_never_reported() {
        let graph = graph(
            vec![("a", blocked(1)), ("b", blocked(1)), ("c", running())],
            vec![
                edge("a", "b", WaitKind::AgentWait).with_group(1),
                edge("b", "c", WaitKind::AgentWait).with_group(2),
            ],
        );
        assert_eq!(detect(&graph), Detection::NoDeadlock);
    }

    #[test]
    fn waiting_on_a_human_a_process_or_a_timer_is_never_a_cycle() {
        for external in [
            WaitTarget::User,
            WaitTarget::Process("pid-1".into()),
            WaitTarget::Timer,
        ] {
            let graph = graph(
                vec![("a", blocked(1)), ("b", blocked(1))],
                vec![
                    edge("a", "b", WaitKind::AgentWait).with_group(1),
                    edge("b", "a", WaitKind::Question),
                    WaitEdge::new(run("b"), external.clone(), WaitKind::Question, "human"),
                ],
            );
            assert_eq!(detect(&graph), Detection::NoDeadlock, "{external}");
            assert!(graph.nodes[&run("b")].waiting_on_external);
            assert!(!graph.nodes[&run("b")].blocked);
        }
    }

    #[test]
    fn an_await_any_is_only_a_cycle_when_every_branch_is_blocked() {
        let edges = |escape: RunFacts| {
            graph(
                vec![("a", blocked(1)), ("b", blocked(1)), ("c", escape)],
                vec![
                    edge("a", "b", WaitKind::AgentWait).with_group(3),
                    edge("a", "c", WaitKind::AgentWait).with_group(3),
                    edge("b", "a", WaitKind::Question),
                ],
            )
        };
        assert_eq!(detect(&edges(running())), Detection::NoDeadlock);
        let both_blocked = edges(blocked(1));
        assert_eq!(both_blocked.edges.len(), 3);
        assert!(matches!(
            detect(&both_blocked),
            Detection::Deadlock(report) if report.participants.len() == 2
        ));
    }

    #[test]
    fn capacity_edges_only_count_when_the_holder_is_itself_blocked() {
        let build = |holder_is_blocked: bool| {
            let mut edges = vec![edge("queued", "holder", WaitKind::Capacity)];
            if holder_is_blocked {
                edges.push(edge("holder", "queued", WaitKind::Question));
            }
            graph(
                vec![
                    (
                        "queued",
                        RunFacts {
                            parked: false,
                            waiting_for_capacity: true,
                            version: 1,
                        },
                    ),
                    ("holder", running()),
                ],
                edges,
            )
        };
        let free = build(false);
        assert!(free.edges.is_empty());
        assert_eq!(detect(&free), Detection::NoDeadlock);
        let contended = build(true);
        assert_eq!(contended.edges.len(), 2);
        assert!(matches!(detect(&contended), Detection::Deadlock(_)));
    }

    #[test]
    fn a_graph_past_its_bounds_reports_incomplete_not_no_deadlock() {
        for bounds in [
            WaitGraphBounds {
                max_nodes: 1,
                ..WaitGraphBounds::default()
            },
            WaitGraphBounds {
                max_edges: 1,
                ..WaitGraphBounds::default()
            },
        ] {
            let graph = WaitGraph::build(
                [
                    (run("a"), blocked(1)),
                    (run("b"), blocked(1)),
                    (run("c"), blocked(1)),
                ],
                vec![
                    edge("a", "b", WaitKind::Question),
                    edge("b", "a", WaitKind::Question),
                ],
                bounds,
            );
            match detect(&graph) {
                Detection::Incomplete(reason) => {
                    assert!(reason.contains("graph too large"), "{reason}");
                    assert!(reason.contains("incomplete"), "{reason}");
                }
                other => panic!("expected an incomplete result, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_run_that_changes_state_mid_walk_re_runs_instead_of_reporting() {
        let cycle = |version: u64| {
            graph(
                vec![("a", blocked(version)), ("b", blocked(1))],
                vec![
                    edge("a", "b", WaitKind::AgentWait).with_group(1),
                    edge("b", "a", WaitKind::Question),
                ],
            )
        };
        let mut captures = 0;
        let outcome = detect_stable(
            || {
                captures += 1;
                cycle(captures as u64)
            },
            3,
        );
        assert_eq!(outcome, Detection::Inconsistent { attempts: 3 });
        assert_eq!(captures, 6);

        let mut captures = 0;
        let outcome = detect_stable(
            || {
                captures += 1;
                cycle(if captures > 2 { 9 } else { captures as u64 })
            },
            3,
        );
        assert!(
            matches!(&outcome, Detection::Deadlock(report) if report.attempts == 2),
            "{outcome:?}"
        );
    }

    #[test]
    fn declared_waits_live_exactly_as_long_as_their_guard() {
        let registry = WaitRegistry::default();
        assert!(registry.edges().is_empty());
        let before = registry.version_of(&run("a"));
        let handle = registry.declare(
            run("a"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("b")), WaitTarget::Run(run("c"))],
            "agent_wait",
        );
        assert!(handle.is_tracked());
        let edges = registry.edges();
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|edge| edge.group == handle.group()));
        assert!(registry.version_of(&run("a")) > before);
        assert_eq!(registry.live_waits(), 1);
        drop(handle);
        assert!(registry.edges().is_empty());
        assert_eq!(registry.live_waits(), 0);
        assert!(registry.version_of(&run("a")) > before + 1);

        let empty = registry.declare(run("a"), WaitKind::Child, vec![], "nothing");
        assert!(!empty.is_tracked());
        assert!(registry.edges().is_empty());
    }

    #[test]
    fn a_refusal_lands_on_the_wait_that_closed_the_loop() {
        let registry = WaitRegistry::default();
        let closing = registry.declare(
            run("b"),
            WaitKind::Question,
            vec![WaitTarget::Run(run("a"))],
            "question",
        );
        let other = registry.declare(
            run("a"),
            WaitKind::AgentWait,
            vec![WaitTarget::Run(run("b"))],
            "agent_wait",
        );
        let report = DeadlockReport {
            participants: vec![run("a"), run("b")],
            cycle: vec![],
            closing_edge: WaitEdge::new(
                run("b"),
                WaitTarget::Run(run("a")),
                WaitKind::Question,
                "question",
            )
            .with_group(closing.group().unwrap()),
            attempts: 1,
        };
        assert!(registry.refuse(&report));
        assert_eq!(closing.refusal().as_ref(), Some(&report));
        assert!(other.refusal().is_none());
        assert_eq!(registry.latest_report().as_ref(), Some(&report));
    }
}
