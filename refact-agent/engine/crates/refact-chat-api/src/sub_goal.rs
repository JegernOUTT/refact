use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::CriterionVerdict;

/// The session's own goal. Every declared lane hangs off this implicit root, so
/// the single-active-top-level-goal interface is unchanged: children are
/// composition under one goal, not a second top-level goal.
pub const ROOT_SUB_GOAL_ID: &str = "goal";

pub const DEFAULT_SUB_GOAL_MAX_DEPTH: usize = 4;
pub const DEFAULT_SUB_GOAL_MAX_FAN_OUT: usize = 16;

/// Wire-identical to `refact_work_core::identity::WorkKey` (`runtime` is the
/// snake_case `RuntimeKind`), expressed here so the chat API does not inherit
/// work-core's storage dependencies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubGoalOwner {
    pub runtime: String,
    pub id: String,
}

impl SubGoalOwner {
    pub fn new(runtime: impl Into<String>, id: impl Into<String>) -> Self {
        SubGoalOwner {
            runtime: runtime.into(),
            id: id.into(),
        }
    }

    pub fn trajectory(id: impl Into<String>) -> Self {
        SubGoalOwner::new("trajectory", id)
    }

    pub fn background_agent(id: impl Into<String>) -> Self {
        SubGoalOwner::new("background_agent", id)
    }
}

impl fmt::Display for SubGoalOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.runtime, self.id)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubGoalEdgeKind {
    #[default]
    Required,
    Optional,
}

impl SubGoalEdgeKind {
    pub fn is_required(self) -> bool {
        matches!(self, SubGoalEdgeKind::Required)
    }

    pub fn label(self) -> &'static str {
        match self {
            SubGoalEdgeKind::Required => "required",
            SubGoalEdgeKind::Optional => "optional",
        }
    }
}

/// What a lane actually produced.
///
/// `NotRun` means the lane was declared and nobody ever dispatched it. It is a
/// different statement from a dispatched lane that looked and found nothing,
/// which reports `Met` with no findings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubGoalOutcome {
    #[default]
    NotRun,
    Dispatched,
    Met,
    Failed,
    Blocked,
    Superseded,
}

impl SubGoalOutcome {
    pub fn is_met(self) -> bool {
        matches!(self, SubGoalOutcome::Met)
    }

    pub fn label(self) -> &'static str {
        match self {
            SubGoalOutcome::NotRun => "not_run",
            SubGoalOutcome::Dispatched => "dispatched",
            SubGoalOutcome::Met => "met",
            SubGoalOutcome::Failed => "failed",
            SubGoalOutcome::Blocked => "blocked",
            SubGoalOutcome::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubGoalNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<SubGoalOwner>,
    /// The criteria this lane owes its parent. Each one needs a met verdict
    /// before the lane counts as met.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criterion_ids: Vec<String>,
}

impl SubGoalNode {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        SubGoalNode {
            id: id.into(),
            title: title.into(),
            owner: None,
            criterion_ids: Vec::new(),
        }
    }

    pub fn with_owner(mut self, owner: SubGoalOwner) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn with_criteria<I, S>(mut self, criterion_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.criterion_ids = criterion_ids.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubGoalEdge {
    pub parent_id: String,
    pub child_id: String,
    #[serde(default)]
    pub kind: SubGoalEdgeKind,
}

impl SubGoalEdge {
    pub fn required(parent_id: impl Into<String>, child_id: impl Into<String>) -> Self {
        SubGoalEdge {
            parent_id: parent_id.into(),
            child_id: child_id.into(),
            kind: SubGoalEdgeKind::Required,
        }
    }

    pub fn optional(parent_id: impl Into<String>, child_id: impl Into<String>) -> Self {
        SubGoalEdge {
            parent_id: parent_id.into(),
            child_id: child_id.into(),
            kind: SubGoalEdgeKind::Optional,
        }
    }
}

/// A child's structured answer to its parent.
///
/// `result_summary` is a human-readable companion and is never an input to
/// [`aggregate_sub_goals`]; `criteria_verdicts` is the decision input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubGoalReport {
    #[serde(default)]
    pub claimed_outcome: SubGoalOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria_verdicts: Vec<CriterionVerdict>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result_summary: String,
    #[serde(default, skip_serializing_if = "crate::is_zero_u64")]
    pub reported_at_ms: u64,
}

impl SubGoalReport {
    pub fn met_verdict(&self, criterion_id: &str) -> bool {
        self.criteria_verdicts
            .iter()
            .any(|verdict| verdict.id == criterion_id && verdict.met)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubGoalDeclaration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<SubGoalNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<SubGoalEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum SubGoalDeclarationError {
    EmptyNodeId,
    ReservedNodeId {
        node_id: String,
    },
    DuplicateNode {
        node_id: String,
    },
    UnknownNode {
        parent_id: String,
        child_id: String,
        missing: String,
    },
    SelfEdge {
        node_id: String,
    },
    Cycle {
        participants: Vec<String>,
    },
    DepthExceeded {
        node_id: String,
        depth: usize,
        max_depth: usize,
    },
    FanOutExceeded {
        parent_id: String,
        fan_out: usize,
        max_fan_out: usize,
    },
}

impl fmt::Display for SubGoalDeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubGoalDeclarationError::EmptyNodeId => f.write_str("sub-goal id must not be empty"),
            SubGoalDeclarationError::ReservedNodeId { node_id } => write!(
                f,
                "sub-goal id '{node_id}' is reserved for the top-level goal"
            ),
            SubGoalDeclarationError::DuplicateNode { node_id } => {
                write!(f, "sub-goal '{node_id}' is already declared")
            }
            SubGoalDeclarationError::UnknownNode {
                parent_id,
                child_id,
                missing,
            } => write!(
                f,
                "edge {parent_id} -> {child_id} names undeclared sub-goal '{missing}'"
            ),
            SubGoalDeclarationError::SelfEdge { node_id } => {
                write!(f, "sub-goal '{node_id}' cannot depend on itself")
            }
            SubGoalDeclarationError::Cycle { participants } => write!(
                f,
                "sub-goal cycle rejected: {}",
                participants.join(" -> ")
            ),
            SubGoalDeclarationError::DepthExceeded {
                node_id,
                depth,
                max_depth,
            } => write!(
                f,
                "sub-goal '{node_id}' sits at depth {depth}, over the limit of {max_depth}"
            ),
            SubGoalDeclarationError::FanOutExceeded {
                parent_id,
                fan_out,
                max_fan_out,
            } => write!(
                f,
                "sub-goal '{parent_id}' would own {fan_out} children, over the limit of {max_fan_out}"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubGoalLimits {
    pub max_depth: usize,
    pub max_fan_out: usize,
}

impl Default for SubGoalLimits {
    fn default() -> Self {
        SubGoalLimits {
            max_depth: DEFAULT_SUB_GOAL_MAX_DEPTH,
            max_fan_out: DEFAULT_SUB_GOAL_MAX_FAN_OUT,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubGoalNodeState {
    #[serde(flatten)]
    pub node: SubGoalNode,
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub dispatched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<SubGoalReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubGoalGraph {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<SubGoalNodeState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<SubGoalEdge>,
}

impl SubGoalGraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }

    pub fn node(&self, node_id: &str) -> Option<&SubGoalNodeState> {
        self.nodes.iter().find(|state| state.node.id == node_id)
    }

    fn node_mut(&mut self, node_id: &str) -> Option<&mut SubGoalNodeState> {
        self.nodes.iter_mut().find(|state| state.node.id == node_id)
    }

    /// Rejects a declaration before any ledger op is written: unknown or
    /// duplicate ids, self edges, cycles, and the Settings-bounded depth and
    /// fan-out caps.
    pub fn validate_declaration(
        &self,
        declaration: &SubGoalDeclaration,
        limits: SubGoalLimits,
    ) -> Result<(), SubGoalDeclarationError> {
        let mut known: BTreeSet<String> = self
            .nodes
            .iter()
            .map(|state| state.node.id.clone())
            .collect();
        known.insert(ROOT_SUB_GOAL_ID.to_string());
        for node in &declaration.nodes {
            if node.id.is_empty() {
                return Err(SubGoalDeclarationError::EmptyNodeId);
            }
            if node.id == ROOT_SUB_GOAL_ID {
                return Err(SubGoalDeclarationError::ReservedNodeId {
                    node_id: node.id.clone(),
                });
            }
            if !known.insert(node.id.clone()) {
                return Err(SubGoalDeclarationError::DuplicateNode {
                    node_id: node.id.clone(),
                });
            }
        }

        let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in self.edges.iter().chain(declaration.edges.iter()) {
            if edge.parent_id == edge.child_id {
                return Err(SubGoalDeclarationError::SelfEdge {
                    node_id: edge.parent_id.clone(),
                });
            }
            for endpoint in [&edge.parent_id, &edge.child_id] {
                if !known.contains(endpoint) {
                    return Err(SubGoalDeclarationError::UnknownNode {
                        parent_id: edge.parent_id.clone(),
                        child_id: edge.child_id.clone(),
                        missing: endpoint.clone(),
                    });
                }
            }
            let children = adjacency.entry(edge.parent_id.clone()).or_default();
            if !children.contains(&edge.child_id) {
                children.push(edge.child_id.clone());
            }
        }

        if let Some(participants) = first_cycle(&adjacency) {
            return Err(SubGoalDeclarationError::Cycle { participants });
        }

        for (parent_id, children) in &adjacency {
            if children.len() > limits.max_fan_out {
                return Err(SubGoalDeclarationError::FanOutExceeded {
                    parent_id: parent_id.clone(),
                    fan_out: children.len(),
                    max_fan_out: limits.max_fan_out,
                });
            }
        }

        for (node_id, depth) in longest_depths(&adjacency) {
            if depth > limits.max_depth {
                return Err(SubGoalDeclarationError::DepthExceeded {
                    node_id,
                    depth,
                    max_depth: limits.max_depth,
                });
            }
        }
        Ok(())
    }

    /// Replay-time application. Validation already happened at declaration, so
    /// this is total and deterministic: a duplicate id or edge is skipped
    /// rather than rejected, and an old ledger replays unchanged.
    pub fn apply_declaration(&mut self, declaration: &SubGoalDeclaration) {
        for node in &declaration.nodes {
            if node.id.is_empty() || node.id == ROOT_SUB_GOAL_ID || self.node(&node.id).is_some() {
                continue;
            }
            self.nodes.push(SubGoalNodeState {
                node: node.clone(),
                dispatched: false,
                superseded: None,
                report: None,
            });
        }
        for edge in &declaration.edges {
            let already = self.edges.iter().any(|existing| {
                existing.parent_id == edge.parent_id && existing.child_id == edge.child_id
            });
            if !already {
                self.edges.push(edge.clone());
            }
        }
    }

    pub fn mark_dispatched(&mut self, node_id: &str, owner: Option<SubGoalOwner>) {
        let Some(state) = self.node_mut(node_id) else {
            return;
        };
        state.dispatched = true;
        if owner.is_some() {
            state.node.owner = owner;
        }
    }

    pub fn record_report(&mut self, node_id: &str, report: SubGoalReport) {
        let Some(state) = self.node_mut(node_id) else {
            return;
        };
        state.dispatched = true;
        state.report = Some(report);
    }

    pub fn supersede(&mut self, node_id: &str, reason: &str) {
        let Some(state) = self.node_mut(node_id) else {
            return;
        };
        state.superseded = Some(reason.to_string());
    }
}

fn first_cycle(adjacency: &BTreeMap<String, Vec<String>>) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Open,
        Done,
    }

    let mut marks: BTreeMap<&str, Mark> = BTreeMap::new();
    let mut stack: Vec<&str> = Vec::new();

    fn walk<'a>(
        node: &'a str,
        adjacency: &'a BTreeMap<String, Vec<String>>,
        marks: &mut BTreeMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match marks.get(node) {
            Some(Mark::Done) => return None,
            Some(Mark::Open) => {
                let start = stack
                    .iter()
                    .position(|entry| *entry == node)
                    .unwrap_or_default();
                let mut participants: Vec<String> = stack[start..]
                    .iter()
                    .map(|entry| entry.to_string())
                    .collect();
                participants.push(node.to_string());
                return Some(participants);
            }
            None => {}
        }
        marks.insert(node, Mark::Open);
        stack.push(node);
        if let Some(children) = adjacency.get(node) {
            for child in children {
                if let Some(cycle) = walk(child.as_str(), adjacency, marks, stack) {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        marks.insert(node, Mark::Done);
        None
    }

    for parent in adjacency.keys() {
        if let Some(cycle) = walk(parent.as_str(), adjacency, &mut marks, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

/// Longest path length from the root to each reachable node. Safe to compute
/// iteratively because the caller has already rejected cycles.
fn longest_depths(adjacency: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, usize> {
    let mut depths: BTreeMap<String, usize> = BTreeMap::new();
    depths.insert(ROOT_SUB_GOAL_ID.to_string(), 0);
    let total_edges: usize = adjacency.values().map(Vec::len).sum();
    for _ in 0..=total_edges {
        let mut changed = false;
        for (parent, children) in adjacency {
            let Some(parent_depth) = depths.get(parent).copied() else {
                continue;
            };
            for child in children {
                let candidate = parent_depth + 1;
                match depths.get_mut(child) {
                    Some(existing) if *existing < candidate => {
                        *existing = candidate;
                        changed = true;
                    }
                    Some(_) => {}
                    None => {
                        depths.insert(child.clone(), candidate);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    depths.remove(ROOT_SUB_GOAL_ID);
    depths
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubGoalLane {
    pub node_id: String,
    pub title: String,
    pub kind: SubGoalEdgeKind,
    pub owner: Option<SubGoalOwner>,
    pub outcome: SubGoalOutcome,
    pub dispatched: bool,
    pub unmet_criteria: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubGoalRollup {
    pub required_total: usize,
    pub required_met: usize,
    pub lanes: Vec<SubGoalLane>,
}

impl SubGoalRollup {
    pub fn blocking(&self) -> Vec<&SubGoalLane> {
        self.lanes
            .iter()
            .filter(|lane| lane.kind.is_required() && !lane.outcome.is_met())
            .collect()
    }

    /// Mechanical: every required lane met, or the parent stays put.
    pub fn parent_may_proceed(&self) -> bool {
        self.blocking().is_empty()
    }

    pub fn reasons(&self) -> Vec<String> {
        self.blocking()
            .into_iter()
            .map(|lane| lane.reason.clone())
            .collect()
    }

    pub fn optional_lanes(&self) -> Vec<&SubGoalLane> {
        self.lanes
            .iter()
            .filter(|lane| !lane.kind.is_required())
            .collect()
    }
}

/// The lane's real state, derived from structured verdicts only. A child that
/// claims success while a criterion it owes is unmet reads as `Failed`.
pub fn sub_goal_effective_outcome(state: &SubGoalNodeState) -> (SubGoalOutcome, Vec<String>) {
    if state.superseded.is_some() {
        return (SubGoalOutcome::Superseded, Vec::new());
    }
    let Some(report) = state.report.as_ref() else {
        let outcome = if state.dispatched {
            SubGoalOutcome::Dispatched
        } else {
            SubGoalOutcome::NotRun
        };
        return (outcome, state.node.criterion_ids.clone());
    };

    let mut unmet: Vec<String> = Vec::new();
    for verdict in &report.criteria_verdicts {
        if !verdict.met && !unmet.contains(&verdict.id) {
            unmet.push(verdict.id.clone());
        }
    }
    for criterion_id in &state.node.criterion_ids {
        if !report.met_verdict(criterion_id) && !unmet.contains(criterion_id) {
            unmet.push(criterion_id.clone());
        }
    }
    if !unmet.is_empty() {
        return (SubGoalOutcome::Failed, unmet);
    }

    let outcome = match report.claimed_outcome {
        SubGoalOutcome::Met => SubGoalOutcome::Met,
        SubGoalOutcome::NotRun | SubGoalOutcome::Dispatched => SubGoalOutcome::Dispatched,
        other => other,
    };
    (outcome, Vec::new())
}

fn lane_reason(
    title: &str,
    node_id: &str,
    kind: SubGoalEdgeKind,
    outcome: SubGoalOutcome,
    unmet: &[String],
) -> String {
    let name = if title.is_empty() { node_id } else { title };
    let kind_label = kind.label();
    match outcome {
        SubGoalOutcome::Met => format!("{kind_label} sub-goal '{name}' is met"),
        SubGoalOutcome::NotRun => format!(
            "{kind_label} sub-goal '{name}' was never dispatched [not_run] — this is not a result"
        ),
        SubGoalOutcome::Dispatched => {
            format!("{kind_label} sub-goal '{name}' is still running and has reported no verdict")
        }
        SubGoalOutcome::Blocked => format!("{kind_label} sub-goal '{name}' reported blocked"),
        SubGoalOutcome::Superseded => format!("{kind_label} sub-goal '{name}' was superseded"),
        SubGoalOutcome::Failed => {
            if unmet.is_empty() {
                format!("{kind_label} sub-goal '{name}' reported failed")
            } else {
                format!(
                    "{kind_label} sub-goal '{name}' failed criteria: {}",
                    unmet.join(", ")
                )
            }
        }
    }
}

/// Rolls a parent's outgoing edges up into one mechanical decision. Reads
/// `criteria_verdicts`; never reads `result_summary`.
pub fn aggregate_sub_goals(graph: &SubGoalGraph, parent_id: &str) -> SubGoalRollup {
    let mut rollup = SubGoalRollup::default();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.parent_id == parent_id)
    {
        let Some(state) = graph.node(&edge.child_id) else {
            continue;
        };
        let (outcome, unmet_criteria) = sub_goal_effective_outcome(state);
        if edge.kind.is_required() {
            rollup.required_total += 1;
            if outcome.is_met() {
                rollup.required_met += 1;
            }
        }
        rollup.lanes.push(SubGoalLane {
            node_id: state.node.id.clone(),
            title: state.node.title.clone(),
            kind: edge.kind,
            owner: state.node.owner.clone(),
            outcome,
            dispatched: state.dispatched,
            reason: lane_reason(
                &state.node.title,
                &state.node.id,
                edge.kind,
                outcome,
                &unmet_criteria,
            ),
            unmet_criteria,
        });
    }
    rollup
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(id: &str, met: bool) -> CriterionVerdict {
        CriterionVerdict {
            id: id.to_string(),
            met,
            note: String::new(),
            source: crate::VerdictSource::Measured,
            evidence: None,
        }
    }

    fn graph_with(nodes: Vec<SubGoalNode>, edges: Vec<SubGoalEdge>) -> SubGoalGraph {
        let declaration = SubGoalDeclaration { nodes, edges };
        let mut graph = SubGoalGraph::default();
        graph
            .validate_declaration(&declaration, SubGoalLimits::default())
            .expect("declaration is valid");
        graph.apply_declaration(&declaration);
        graph
    }

    #[test]
    fn a_child_claiming_done_with_a_failed_criterion_does_not_let_the_parent_proceed() {
        let mut graph = graph_with(
            vec![SubGoalNode::new("lint", "Lint the crate").with_criteria(["c1", "c2"])],
            vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "lint")],
        );
        graph.record_report(
            "lint",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c1", true), verdict("c2", false)],
                result_summary: "DONE. Everything works, shipping it.".to_string(),
                reported_at_ms: 10,
            },
        );

        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);

        assert!(!rollup.parent_may_proceed());
        assert_eq!(rollup.required_total, 1);
        assert_eq!(rollup.required_met, 0);
        assert_eq!(rollup.lanes[0].outcome, SubGoalOutcome::Failed);
        assert_eq!(rollup.lanes[0].unmet_criteria, vec!["c2".to_string()]);
        assert!(rollup.reasons()[0].contains("c2"));
    }

    #[test]
    fn the_result_summary_is_never_an_input_to_the_decision() {
        let build = |summary: &str, met: bool| {
            let mut graph = graph_with(
                vec![SubGoalNode::new("lane", "Lane").with_criteria(["c1"])],
                vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "lane")],
            );
            graph.record_report(
                "lane",
                SubGoalReport {
                    claimed_outcome: SubGoalOutcome::Met,
                    criteria_verdicts: vec![verdict("c1", met)],
                    result_summary: summary.to_string(),
                    reported_at_ms: 1,
                },
            );
            aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID)
        };

        assert_eq!(build("DONE", true), build("catastrophic failure", true));
        assert_eq!(build("DONE", false), build("catastrophic failure", false));
        assert!(build("total disaster", true).parent_may_proceed());
        assert!(!build("DONE", false).parent_may_proceed());
    }

    #[test]
    fn all_required_edges_met_by_evidence_lets_the_parent_proceed() {
        let mut graph = graph_with(
            vec![
                SubGoalNode::new("a", "A").with_criteria(["c1"]),
                SubGoalNode::new("b", "B").with_criteria(["c2"]),
            ],
            vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "a"),
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "b"),
            ],
        );
        graph.record_report(
            "a",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c1", true)],
                ..Default::default()
            },
        );
        graph.record_report(
            "b",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c2", true)],
                ..Default::default()
            },
        );

        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);

        assert!(rollup.parent_may_proceed());
        assert_eq!(rollup.required_total, 2);
        assert_eq!(rollup.required_met, 2);
        assert!(rollup.reasons().is_empty());
    }

    #[test]
    fn an_optional_failed_edge_is_reported_and_does_not_block() {
        let mut graph = graph_with(
            vec![
                SubGoalNode::new("must", "Must").with_criteria(["c1"]),
                SubGoalNode::new("maybe", "Maybe").with_criteria(["c2"]),
            ],
            vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "must"),
                SubGoalEdge::optional(ROOT_SUB_GOAL_ID, "maybe"),
            ],
        );
        graph.record_report(
            "must",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c1", true)],
                ..Default::default()
            },
        );
        graph.record_report(
            "maybe",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c2", false)],
                ..Default::default()
            },
        );

        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);

        assert!(rollup.parent_may_proceed());
        assert_eq!(rollup.required_total, 1);
        let optional = rollup.optional_lanes();
        assert_eq!(optional.len(), 1);
        assert_eq!(optional[0].outcome, SubGoalOutcome::Failed);
        assert!(rollup.reasons().is_empty());
    }

    #[test]
    fn an_undispatched_optional_lane_reads_as_not_run_not_as_an_empty_result() {
        let mut graph = graph_with(
            vec![
                SubGoalNode::new("never", "Never dispatched"),
                SubGoalNode::new("empty", "Looked and found nothing"),
            ],
            vec![
                SubGoalEdge::optional(ROOT_SUB_GOAL_ID, "never"),
                SubGoalEdge::optional(ROOT_SUB_GOAL_ID, "empty"),
            ],
        );
        graph.record_report(
            "empty",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: Vec::new(),
                result_summary: "no findings".to_string(),
                reported_at_ms: 5,
            },
        );

        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);
        let never = rollup
            .lanes
            .iter()
            .find(|lane| lane.node_id == "never")
            .unwrap();
        let empty = rollup
            .lanes
            .iter()
            .find(|lane| lane.node_id == "empty")
            .unwrap();

        assert_eq!(never.outcome, SubGoalOutcome::NotRun);
        assert!(!never.dispatched);
        assert!(never.reason.contains("not_run"));
        assert!(never.reason.contains("not a result"));

        assert_eq!(empty.outcome, SubGoalOutcome::Met);
        assert!(empty.dispatched);
        assert_ne!(never.outcome, empty.outcome);
    }

    #[test]
    fn an_undispatched_required_lane_blocks_the_parent() {
        let graph = graph_with(
            vec![SubGoalNode::new("lane", "Lane")],
            vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "lane")],
        );
        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);
        assert!(!rollup.parent_may_proceed());
        assert_eq!(rollup.lanes[0].outcome, SubGoalOutcome::NotRun);
    }

    #[test]
    fn a_dispatched_lane_without_a_verdict_blocks_and_is_not_not_run() {
        let mut graph = graph_with(
            vec![SubGoalNode::new("lane", "Lane")],
            vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "lane")],
        );
        graph.mark_dispatched("lane", Some(SubGoalOwner::background_agent("bg-1")));

        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);

        assert_eq!(rollup.lanes[0].outcome, SubGoalOutcome::Dispatched);
        assert!(!rollup.parent_may_proceed());
        assert_eq!(
            rollup.lanes[0].owner,
            Some(SubGoalOwner::background_agent("bg-1"))
        );
    }

    #[test]
    fn a_superseded_required_lane_blocks() {
        let mut graph = graph_with(
            vec![SubGoalNode::new("lane", "Lane").with_criteria(["c1"])],
            vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "lane")],
        );
        graph.record_report(
            "lane",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c1", true)],
                ..Default::default()
            },
        );
        graph.supersede("lane", "plan changed");

        let rollup = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);
        assert_eq!(rollup.lanes[0].outcome, SubGoalOutcome::Superseded);
        assert!(!rollup.parent_may_proceed());
    }

    #[test]
    fn a_declared_cycle_is_rejected_and_names_its_participants() {
        let graph = graph_with(
            vec![
                SubGoalNode::new("a", "A"),
                SubGoalNode::new("b", "B"),
                SubGoalNode::new("c", "C"),
            ],
            vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "a"),
                SubGoalEdge::required("a", "b"),
                SubGoalEdge::required("b", "c"),
            ],
        );

        let closing = SubGoalDeclaration {
            nodes: Vec::new(),
            edges: vec![SubGoalEdge::required("c", "a")],
        };
        let error = graph
            .validate_declaration(&closing, SubGoalLimits::default())
            .expect_err("cycle must be rejected");

        let SubGoalDeclarationError::Cycle { participants } = &error else {
            panic!("expected a cycle error, got {error:?}");
        };
        for expected in ["a", "b", "c"] {
            assert!(
                participants.iter().any(|entry| entry == expected),
                "cycle {participants:?} does not name {expected}"
            );
        }
        assert!(error.to_string().contains("->"));
    }

    #[test]
    fn a_self_edge_is_rejected() {
        let graph = graph_with(
            vec![SubGoalNode::new("a", "A")],
            vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "a")],
        );
        let error = graph
            .validate_declaration(
                &SubGoalDeclaration {
                    nodes: Vec::new(),
                    edges: vec![SubGoalEdge::required("a", "a")],
                },
                SubGoalLimits::default(),
            )
            .expect_err("self edge must be rejected");
        assert!(matches!(
            error,
            SubGoalDeclarationError::SelfEdge { ref node_id } if node_id == "a"
        ));
    }

    #[test]
    fn depth_and_fan_out_caps_are_enforced() {
        let limits = SubGoalLimits {
            max_depth: 2,
            max_fan_out: 2,
        };

        let too_deep = SubGoalDeclaration {
            nodes: vec![
                SubGoalNode::new("a", "A"),
                SubGoalNode::new("b", "B"),
                SubGoalNode::new("c", "C"),
            ],
            edges: vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "a"),
                SubGoalEdge::required("a", "b"),
                SubGoalEdge::required("b", "c"),
            ],
        };
        let error = SubGoalGraph::default()
            .validate_declaration(&too_deep, limits)
            .expect_err("depth cap must bite");
        assert!(matches!(
            error,
            SubGoalDeclarationError::DepthExceeded {
                ref node_id,
                depth: 3,
                max_depth: 2
            } if node_id == "c"
        ));

        let too_wide = SubGoalDeclaration {
            nodes: vec![
                SubGoalNode::new("a", "A"),
                SubGoalNode::new("b", "B"),
                SubGoalNode::new("c", "C"),
            ],
            edges: vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "a"),
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "b"),
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "c"),
            ],
        };
        let error = SubGoalGraph::default()
            .validate_declaration(&too_wide, limits)
            .expect_err("fan-out cap must bite");
        assert!(matches!(
            error,
            SubGoalDeclarationError::FanOutExceeded {
                fan_out: 3,
                max_fan_out: 2,
                ..
            }
        ));
    }

    #[test]
    fn a_diamond_is_a_dag_and_is_accepted() {
        let declaration = SubGoalDeclaration {
            nodes: vec![
                SubGoalNode::new("left", "Left"),
                SubGoalNode::new("right", "Right"),
                SubGoalNode::new("join", "Join"),
            ],
            edges: vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "left"),
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "right"),
                SubGoalEdge::required("left", "join"),
                SubGoalEdge::required("right", "join"),
            ],
        };
        SubGoalGraph::default()
            .validate_declaration(&declaration, SubGoalLimits::default())
            .expect("a diamond has no cycle");
    }

    #[test]
    fn an_unknown_endpoint_and_a_duplicate_id_are_rejected() {
        let graph = graph_with(
            vec![SubGoalNode::new("a", "A")],
            vec![SubGoalEdge::required(ROOT_SUB_GOAL_ID, "a")],
        );

        let unknown = graph
            .validate_declaration(
                &SubGoalDeclaration {
                    nodes: Vec::new(),
                    edges: vec![SubGoalEdge::required("a", "ghost")],
                },
                SubGoalLimits::default(),
            )
            .expect_err("unknown endpoint must be rejected");
        assert!(matches!(
            unknown,
            SubGoalDeclarationError::UnknownNode { ref missing, .. } if missing == "ghost"
        ));

        let duplicate = graph
            .validate_declaration(
                &SubGoalDeclaration {
                    nodes: vec![SubGoalNode::new("a", "A again")],
                    edges: Vec::new(),
                },
                SubGoalLimits::default(),
            )
            .expect_err("duplicate id must be rejected");
        assert!(matches!(
            duplicate,
            SubGoalDeclarationError::DuplicateNode { ref node_id } if node_id == "a"
        ));

        let reserved = graph
            .validate_declaration(
                &SubGoalDeclaration {
                    nodes: vec![SubGoalNode::new(ROOT_SUB_GOAL_ID, "root")],
                    edges: Vec::new(),
                },
                SubGoalLimits::default(),
            )
            .expect_err("the root id is reserved");
        assert!(matches!(
            reserved,
            SubGoalDeclarationError::ReservedNodeId { .. }
        ));
    }

    #[test]
    fn a_nested_parent_aggregates_only_its_own_edges() {
        let mut graph = graph_with(
            vec![
                SubGoalNode::new("mid", "Mid"),
                SubGoalNode::new("leaf", "Leaf").with_criteria(["c1"]),
            ],
            vec![
                SubGoalEdge::required(ROOT_SUB_GOAL_ID, "mid"),
                SubGoalEdge::required("mid", "leaf"),
            ],
        );
        graph.record_report(
            "leaf",
            SubGoalReport {
                claimed_outcome: SubGoalOutcome::Met,
                criteria_verdicts: vec![verdict("c1", true)],
                ..Default::default()
            },
        );

        let mid = aggregate_sub_goals(&graph, "mid");
        assert!(mid.parent_may_proceed());
        assert_eq!(mid.lanes.len(), 1);

        let root = aggregate_sub_goals(&graph, ROOT_SUB_GOAL_ID);
        assert_eq!(root.lanes.len(), 1);
        assert_eq!(root.lanes[0].node_id, "mid");
        assert!(!root.parent_may_proceed());
    }

    #[test]
    fn a_report_round_trips_through_json_with_its_verdicts() {
        let report = SubGoalReport {
            claimed_outcome: SubGoalOutcome::Blocked,
            criteria_verdicts: vec![verdict("c1", false)],
            result_summary: "waiting on review".to_string(),
            reported_at_ms: 42,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"blocked\""));
        let parsed: SubGoalReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, report);
    }

    #[test]
    fn an_owner_serializes_exactly_like_a_work_key() {
        let owner = SubGoalOwner::background_agent("bg-7");
        let json = serde_json::to_value(&owner).unwrap();
        assert_eq!(json["runtime"], "background_agent");
        assert_eq!(json["id"], "bg-7");
        assert_eq!(owner.to_string(), "background_agent:bg-7");
    }
}
