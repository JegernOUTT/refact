use serde::{Deserialize, Serialize};

use crate::sub_goal::{SubGoalDeclaration, SubGoalGraph, SubGoalOwner, SubGoalReport};
use crate::{
    GoalAttempt, GoalBudget, GoalCriterion, GoalEvent, GoalProgress, GoalSnapshot, GoalStatus,
    GoalWait,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum GoalLedgerOp {
    Installed {
        version: u32,
        active: bool,
        budget: GoalBudget,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        criteria: Vec<GoalCriterion>,
    },
    StatusChanged {
        from: GoalStatus,
        to: GoalStatus,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    ProgressRecorded {
        tokens: u64,
        made_progress: bool,
        #[serde(default, skip_serializing_if = "crate::is_zero_u64")]
        cost_cents: u64,
    },
    VerifierAttemptRecorded {
        tokens: u64,
    },
    NoProgressNoted,
    NudgeRecorded,
    ProgressReset {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    AttemptPushed {
        attempt: GoalAttempt,
    },
    EventPushed {
        event: GoalEvent,
    },
    BudgetSet {
        budget: GoalBudget,
    },
    CriteriaSet {
        criteria: Vec<GoalCriterion>,
    },
    SnoozeSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until_ms: Option<u64>,
    },
    WaitSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<GoalWait>,
    },
    TransferredOut {
        target_chat_id: String,
    },
    TransferredIn {
        source_chat_id: String,
        #[serde(default)]
        progress: GoalProgress,
    },
    SubGoalsDeclared {
        declaration: SubGoalDeclaration,
    },
    SubGoalDispatched {
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<SubGoalOwner>,
    },
    SubGoalReported {
        node_id: String,
        report: SubGoalReport,
    },
    SubGoalSuperseded {
        node_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalLedgerEntry {
    pub seq: u64,
    pub at_ms: u64,
    #[serde(flatten)]
    pub op: GoalLedgerOp,
}

pub fn goal_budget_exhaustion_status(
    budget: &GoalBudget,
    progress: &GoalProgress,
    now_ms: u64,
) -> Option<GoalStatus> {
    if !budget.supervised {
        return None;
    }
    if let Some(no_progress_turns) = budget.no_progress_turns {
        if no_progress_turns > 0 && progress.no_progress_turns >= no_progress_turns {
            return Some(GoalStatus::NoProgress);
        }
    }
    if let Some(max_turns) = budget.max_turns {
        if max_turns > 0 && progress.turns_used >= max_turns {
            return Some(GoalStatus::BudgetExhausted);
        }
    }
    if let Some(max_tokens) = budget.max_tokens {
        if max_tokens > 0 && progress.tokens_used >= max_tokens {
            return Some(GoalStatus::BudgetExhausted);
        }
    }
    if let Some(max_cost_cents) = budget.max_cost_cents {
        if max_cost_cents > 0 && progress.cost_used_cents >= max_cost_cents {
            return Some(GoalStatus::BudgetExhausted);
        }
    }
    if let Some(max_minutes) = budget.max_minutes {
        let max_ms = u64::from(max_minutes).saturating_mul(60_000);
        if max_minutes > 0
            && progress.started_at_ms > 0
            && now_ms.saturating_sub(progress.started_at_ms) >= max_ms
        {
            return Some(GoalStatus::BudgetExhausted);
        }
    }
    None
}

fn transition_after_accounting(state: &mut GoalLedgerState, at_ms: u64) {
    match goal_budget_exhaustion_status(&state.budget, &state.progress, at_ms) {
        Some(status) => state.status = status,
        None if matches!(
            state.status,
            GoalStatus::BudgetExhausted | GoalStatus::NoProgress
        ) =>
        {
            state.status = GoalStatus::Active;
        }
        None => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalLedgerState {
    pub version: u32,
    pub active: bool,
    pub status: GoalStatus,
    pub budget: GoalBudget,
    pub progress: GoalProgress,
    pub attempts: Vec<GoalAttempt>,
    pub events: Vec<GoalEvent>,
    pub criteria: Vec<GoalCriterion>,
    pub snoozed_until_ms: Option<u64>,
    pub wait: Option<GoalWait>,
    pub stop_reason: Option<String>,
    pub transferred_from: Option<String>,
    pub transferred_to: Option<String>,
    pub sub_goals: SubGoalGraph,
    pub last_seq: u64,
}

impl GoalLedgerState {
    fn new(version: u32, active: bool, budget: GoalBudget, criteria: Vec<GoalCriterion>) -> Self {
        Self {
            version,
            active,
            status: if active {
                GoalStatus::Active
            } else {
                GoalStatus::Paused
            },
            budget,
            progress: GoalProgress::default(),
            attempts: Vec::new(),
            events: Vec::new(),
            criteria,
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: SubGoalGraph::default(),
            last_seq: 0,
        }
    }

    pub fn apply_to_snapshot(&self, snapshot: &mut GoalSnapshot) {
        snapshot.active = self.active;
        snapshot.status = self.status;
        snapshot.progress = self.progress.clone();
        snapshot.attempts = self.attempts.clone();
        snapshot.criteria = self.criteria.clone();
        snapshot.snoozed_until_ms = self.snoozed_until_ms;
        snapshot.wait = self.wait.clone();
        snapshot.stop_reason = self.stop_reason.clone();
        if self.transferred_from.is_some() {
            snapshot.transferred_from = self.transferred_from.clone();
        }
        if self.transferred_to.is_some() {
            snapshot.transferred_to = self.transferred_to.clone();
        }
        snapshot.sub_goals = self.sub_goals.clone();
    }
}

pub fn reduce_goal_ledger(entries: &[GoalLedgerEntry]) -> Option<GoalLedgerState> {
    let mut state: Option<GoalLedgerState> = None;
    for entry in entries {
        match &entry.op {
            GoalLedgerOp::Installed {
                version,
                active,
                budget,
                criteria,
            } => {
                state = Some(GoalLedgerState::new(
                    *version,
                    *active,
                    budget.clone(),
                    criteria.clone(),
                ));
            }
            op => {
                let Some(state) = state.as_mut() else {
                    continue;
                };
                apply_op(state, op, entry.at_ms);
            }
        }
        if let Some(state) = state.as_mut() {
            state.last_seq = entry.seq;
        }
    }
    state
}

fn apply_op(state: &mut GoalLedgerState, op: &GoalLedgerOp, at_ms: u64) {
    match op {
        GoalLedgerOp::Installed {
            ..
        } => {}
        GoalLedgerOp::StatusChanged {
            to,
            reason,
            ..
        } => {
            state.status = *to;
            if *to == GoalStatus::Stopped {
                state.stop_reason = (!reason.is_empty()).then(|| reason.clone());
            } else if matches!(to, GoalStatus::Active | GoalStatus::Verifying) {
                state.stop_reason = None;
            }
        }
        GoalLedgerOp::ProgressRecorded {
            tokens,
            made_progress,
            cost_cents,
        } => {
            if state.progress.started_at_ms == 0 {
                state.progress.started_at_ms = at_ms;
            }
            state.progress.turns_used = state.progress.turns_used.saturating_add(1);
            state.progress.tokens_used = state.progress.tokens_used.saturating_add(*tokens);
            state.progress.cost_used_cents =
                state.progress.cost_used_cents.saturating_add(*cost_cents);
            if *made_progress {
                state.progress.no_progress_turns = 0;
            } else {
                state.progress.no_progress_turns =
                    state.progress.no_progress_turns.saturating_add(1);
            }
            transition_after_accounting(state, at_ms);
        }
        GoalLedgerOp::VerifierAttemptRecorded {
            tokens,
        } => {
            if state.progress.started_at_ms == 0 {
                state.progress.started_at_ms = at_ms;
            }
            state.progress.turns_used = state.progress.turns_used.saturating_add(1);
            state.progress.tokens_used = state.progress.tokens_used.saturating_add(*tokens);
            transition_after_accounting(state, at_ms);
        }
        GoalLedgerOp::NoProgressNoted => {
            if state.progress.started_at_ms == 0 {
                state.progress.started_at_ms = at_ms;
            }
            state.progress.no_progress_turns = state.progress.no_progress_turns.saturating_add(1);
            transition_after_accounting(state, at_ms);
        }
        GoalLedgerOp::NudgeRecorded => {
            state.progress.last_nudge_at_ms = at_ms;
        }
        GoalLedgerOp::ProgressReset {
            ..
        } => {
            state.progress.no_progress_turns = 0;
            state.snoozed_until_ms = None;
            state.wait = None;
            if matches!(
                state.status,
                GoalStatus::BudgetExhausted | GoalStatus::NoProgress
            ) {
                state.status = goal_budget_exhaustion_status(&state.budget, &state.progress, at_ms)
                    .unwrap_or(GoalStatus::Active);
            }
        }
        GoalLedgerOp::AttemptPushed {
            attempt,
        } => {
            state.attempts.push(attempt.clone());
        }
        GoalLedgerOp::EventPushed {
            event,
        } => {
            state.events.push(event.clone());
        }
        GoalLedgerOp::BudgetSet {
            budget,
        } => {
            state.budget = budget.clone();
        }
        GoalLedgerOp::CriteriaSet {
            criteria,
        } => {
            state.criteria = criteria.clone();
        }
        GoalLedgerOp::SnoozeSet {
            until_ms,
        } => {
            state.snoozed_until_ms = *until_ms;
        }
        GoalLedgerOp::WaitSet {
            wait,
        } => {
            state.wait = wait.clone();
        }
        GoalLedgerOp::TransferredOut {
            target_chat_id,
        } => {
            state.active = false;
            state.status = GoalStatus::Transferred;
            state.transferred_to = Some(target_chat_id.clone());
        }
        GoalLedgerOp::TransferredIn {
            source_chat_id,
            progress,
        } => {
            state.active = true;
            state.status = GoalStatus::Active;
            state.progress = progress.clone();
            state.transferred_from = Some(source_chat_id.clone());
            state.transferred_to = None;
        }
        GoalLedgerOp::SubGoalsDeclared {
            declaration,
        } => {
            state.sub_goals.apply_declaration(declaration);
        }
        GoalLedgerOp::SubGoalDispatched {
            node_id,
            owner,
        } => {
            state.sub_goals.mark_dispatched(node_id, owner.clone());
        }
        GoalLedgerOp::SubGoalReported {
            node_id,
            report,
        } => {
            state.sub_goals.record_report(node_id, report.clone());
        }
        GoalLedgerOp::SubGoalSuperseded {
            node_id,
            reason,
        } => {
            state.sub_goals.supersede(node_id, reason);
        }
    }
}

pub fn seed_transferred_goal_ledger(
    target_goal: &GoalSnapshot,
    source_chat_id: &str,
    at_ms: u64,
) -> Vec<GoalLedgerEntry> {
    let mut entries = vec![
        GoalLedgerEntry {
            seq: 1,
            at_ms,
            op: GoalLedgerOp::Installed {
                version: target_goal.version,
                active: true,
                budget: target_goal.budget.clone(),
                criteria: target_goal.criteria.clone(),
            },
        },
        GoalLedgerEntry {
            seq: 2,
            at_ms,
            op: GoalLedgerOp::TransferredIn {
                source_chat_id: source_chat_id.to_string(),
                progress: target_goal.progress.clone(),
            },
        },
    ];
    if target_goal.status != GoalStatus::Active {
        entries.push(GoalLedgerEntry {
            seq: 3,
            at_ms,
            op: GoalLedgerOp::StatusChanged {
                from: GoalStatus::Active,
                to: target_goal.status,
                reason: target_goal.stop_reason.clone().unwrap_or_default(),
            },
        });
    }
    entries
}

pub fn status_changed_since(entries: &[GoalLedgerEntry], seq: u64) -> bool {
    entries.iter().filter(|entry| entry.seq > seq).any(|entry| {
        matches!(
            entry.op,
            GoalLedgerOp::StatusChanged { .. }
                | GoalLedgerOp::TransferredOut { .. }
                | GoalLedgerOp::TransferredIn { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: u64, at_ms: u64, op: GoalLedgerOp) -> GoalLedgerEntry {
        GoalLedgerEntry {
            seq,
            at_ms,
            op,
        }
    }

    fn installed(seq: u64) -> GoalLedgerEntry {
        entry(
            seq,
            1_000,
            GoalLedgerOp::Installed {
                version: 1,
                active: true,
                budget: GoalBudget::default(),
                criteria: Vec::new(),
            },
        )
    }

    #[test]
    fn reduce_replays_progress_and_status() {
        let entries = vec![
            installed(1),
            entry(
                2,
                2_000,
                GoalLedgerOp::ProgressRecorded {
                    tokens: 100,
                    made_progress: true,
                    cost_cents: 3,
                },
            ),
            entry(
                3,
                3_000,
                GoalLedgerOp::StatusChanged {
                    from: GoalStatus::Active,
                    to: GoalStatus::Stopped,
                    reason: "user stop".to_string(),
                },
            ),
        ];

        let state = reduce_goal_ledger(&entries).unwrap();

        assert_eq!(state.status, GoalStatus::Stopped);
        assert_eq!(state.stop_reason.as_deref(), Some("user stop"));
        assert_eq!(state.progress.turns_used, 1);
        assert_eq!(state.progress.tokens_used, 100);
        assert_eq!(state.progress.cost_used_cents, 3);
        assert_eq!(state.progress.started_at_ms, 2_000);
        assert_eq!(state.last_seq, 3);
    }

    #[test]
    fn reduce_stop_survives_later_bookkeeping() {
        let entries = vec![
            installed(1),
            entry(
                2,
                2_000,
                GoalLedgerOp::StatusChanged {
                    from: GoalStatus::Active,
                    to: GoalStatus::Stopped,
                    reason: String::new(),
                },
            ),
            entry(
                3,
                3_000,
                GoalLedgerOp::EventPushed {
                    event: GoalEvent {
                        at_ms: 3_000,
                        kind: "goal_pursuit".to_string(),
                        text: "nudge".to_string(),
                    },
                },
            ),
            entry(4, 4_000, GoalLedgerOp::NudgeRecorded),
        ];

        let state = reduce_goal_ledger(&entries).unwrap();

        assert_eq!(state.status, GoalStatus::Stopped);
        assert_eq!(state.progress.last_nudge_at_ms, 4_000);
        assert_eq!(state.events.len(), 1);
    }

    #[test]
    fn reduce_no_progress_budget_terminates_and_user_reset_heals() {
        let budget = GoalBudget {
            supervised: true,
            no_progress_turns: Some(2),
            ..Default::default()
        };
        let mut entries = vec![entry(
            1,
            1_000,
            GoalLedgerOp::Installed {
                version: 1,
                active: true,
                budget,
                criteria: Vec::new(),
            },
        )];
        entries.push(entry(2, 2_000, GoalLedgerOp::NoProgressNoted));
        entries.push(entry(3, 3_000, GoalLedgerOp::NoProgressNoted));
        assert_eq!(
            reduce_goal_ledger(&entries).unwrap().status,
            GoalStatus::NoProgress
        );

        entries.push(entry(
            4,
            4_000,
            GoalLedgerOp::ProgressReset {
                reason: "user_message".to_string(),
            },
        ));
        let state = reduce_goal_ledger(&entries).unwrap();
        assert_eq!(state.status, GoalStatus::Active);
        assert_eq!(state.progress.no_progress_turns, 0);
    }

    #[test]
    fn reduce_transfer_roundtrip() {
        let entries = vec![
            installed(1),
            entry(
                2,
                2_000,
                GoalLedgerOp::TransferredOut {
                    target_chat_id: "target".to_string(),
                },
            ),
        ];
        let state = reduce_goal_ledger(&entries).unwrap();
        assert!(!state.active);
        assert_eq!(state.status, GoalStatus::Transferred);
        assert_eq!(state.transferred_to.as_deref(), Some("target"));
    }

    #[test]
    fn ordinary_explicit_limits_never_exhaust_but_supervised_limits_do() {
        let mut budget = GoalBudget {
            max_turns: Some(1),
            max_minutes: Some(1),
            max_tokens: Some(1),
            max_cost_cents: Some(1),
            no_progress_turns: Some(1),
            explicit: true,
            ..Default::default()
        };
        let progress = GoalProgress {
            turns_used: 5,
            tokens_used: 5,
            cost_used_cents: 5,
            no_progress_turns: 5,
            started_at_ms: 1,
            ..Default::default()
        };
        assert_eq!(
            goal_budget_exhaustion_status(&budget, &progress, 120_000),
            None
        );
        budget.supervised = true;
        assert_eq!(
            goal_budget_exhaustion_status(&budget, &progress, 120_000),
            Some(GoalStatus::NoProgress)
        );
    }

    #[test]
    fn reduce_max_cost_budget_exhausts() {
        let entries = vec![
            entry(
                1,
                1_000,
                GoalLedgerOp::Installed {
                    version: 1,
                    active: true,
                    budget: GoalBudget {
                        supervised: true,
                        max_cost_cents: Some(10),
                        ..Default::default()
                    },
                    criteria: Vec::new(),
                },
            ),
            entry(
                2,
                2_000,
                GoalLedgerOp::ProgressRecorded {
                    tokens: 5,
                    made_progress: true,
                    cost_cents: 12,
                },
            ),
        ];
        assert_eq!(
            reduce_goal_ledger(&entries).unwrap().status,
            GoalStatus::BudgetExhausted
        );
    }

    #[test]
    fn status_changed_since_detects_interleaved_transitions() {
        let entries = vec![
            installed(1),
            entry(
                2,
                2_000,
                GoalLedgerOp::StatusChanged {
                    from: GoalStatus::Active,
                    to: GoalStatus::Verifying,
                    reason: String::new(),
                },
            ),
            entry(3, 3_000, GoalLedgerOp::NudgeRecorded),
        ];
        assert!(!status_changed_since(&entries, 2));

        let mut with_stop = entries.clone();
        with_stop.push(entry(
            4,
            4_000,
            GoalLedgerOp::StatusChanged {
                from: GoalStatus::Verifying,
                to: GoalStatus::Stopped,
                reason: "user".to_string(),
            },
        ));
        assert!(status_changed_since(&with_stop, 2));
    }

    #[test]
    fn a_legacy_ledger_without_sub_goal_ops_replays_identically() {
        let legacy = serde_json::json!([
            {"seq": 1, "at_ms": 1000, "op": "installed", "version": 1, "active": true,
             "budget": {"cooldown_ms": 1000, "no_progress_token_threshold": 200000}},
            {"seq": 2, "at_ms": 2000, "op": "progress_recorded", "tokens": 42, "made_progress": true},
            {"seq": 3, "at_ms": 3000, "op": "transferred_out", "target_chat_id": "elsewhere"}
        ]);
        let entries: Vec<GoalLedgerEntry> = serde_json::from_value(legacy).unwrap();
        let state = reduce_goal_ledger(&entries).unwrap();

        assert_eq!(state.status, GoalStatus::Transferred);
        assert_eq!(state.transferred_to.as_deref(), Some("elsewhere"));
        assert_eq!(state.progress.tokens_used, 42);
        assert!(state.sub_goals.is_empty());
        assert_eq!(state.last_seq, 3);

        let mut snapshot = GoalSnapshot::default();
        state.apply_to_snapshot(&mut snapshot);
        assert!(snapshot.sub_goals.is_empty());
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(
            json.get("sub_goals").is_none(),
            "an empty sub-goal graph must not appear on the wire"
        );
    }

    #[test]
    fn sub_goal_ops_replay_deterministically_and_decide_the_parent() {
        let declaration = SubGoalDeclaration {
            nodes: vec![
                crate::SubGoalNode::new("a", "A").with_criteria(["c1"]),
                crate::SubGoalNode::new("b", "B").with_criteria(["c2"]),
            ],
            edges: vec![
                crate::SubGoalEdge::required(crate::ROOT_SUB_GOAL_ID, "a"),
                crate::SubGoalEdge::optional(crate::ROOT_SUB_GOAL_ID, "b"),
            ],
        };
        let entries = vec![
            installed(1),
            entry(
                2,
                2_000,
                GoalLedgerOp::SubGoalsDeclared {
                    declaration: declaration.clone(),
                },
            ),
            entry(
                3,
                3_000,
                GoalLedgerOp::SubGoalDispatched {
                    node_id: "a".to_string(),
                    owner: Some(SubGoalOwner::background_agent("bg-1")),
                },
            ),
            entry(
                4,
                4_000,
                GoalLedgerOp::SubGoalReported {
                    node_id: "a".to_string(),
                    report: SubGoalReport {
                        claimed_outcome: crate::SubGoalOutcome::Met,
                        criteria_verdicts: vec![crate::CriterionVerdict {
                            id: "c1".to_string(),
                            met: true,
                            ..Default::default()
                        }],
                        result_summary: "DONE".to_string(),
                        reported_at_ms: 4_000,
                    },
                },
            ),
        ];

        let state = reduce_goal_ledger(&entries).unwrap();
        assert_eq!(state, reduce_goal_ledger(&entries).unwrap());

        let rollup = crate::aggregate_sub_goals(&state.sub_goals, crate::ROOT_SUB_GOAL_ID);
        assert!(rollup.parent_may_proceed());
        assert_eq!(rollup.required_total, 1);
        assert_eq!(rollup.optional_lanes().len(), 1);
        assert_eq!(
            rollup.optional_lanes()[0].outcome,
            crate::SubGoalOutcome::NotRun
        );

        let round_tripped: Vec<GoalLedgerEntry> =
            serde_json::from_value(serde_json::to_value(&entries).unwrap()).unwrap();
        assert_eq!(reduce_goal_ledger(&round_tripped).unwrap(), state);
    }

    #[test]
    fn a_superseded_required_lane_replays_as_blocking() {
        let entries = vec![
            installed(1),
            entry(
                2,
                2_000,
                GoalLedgerOp::SubGoalsDeclared {
                    declaration: SubGoalDeclaration {
                        nodes: vec![crate::SubGoalNode::new("a", "A")],
                        edges: vec![crate::SubGoalEdge::required(crate::ROOT_SUB_GOAL_ID, "a")],
                    },
                },
            ),
            entry(
                3,
                3_000,
                GoalLedgerOp::SubGoalReported {
                    node_id: "a".to_string(),
                    report: SubGoalReport {
                        claimed_outcome: crate::SubGoalOutcome::Met,
                        ..Default::default()
                    },
                },
            ),
            entry(
                4,
                4_000,
                GoalLedgerOp::SubGoalSuperseded {
                    node_id: "a".to_string(),
                    reason: "scope changed".to_string(),
                },
            ),
        ];
        let state = reduce_goal_ledger(&entries).unwrap();
        let rollup = crate::aggregate_sub_goals(&state.sub_goals, crate::ROOT_SUB_GOAL_ID);
        assert!(!rollup.parent_may_proceed());
    }
}
