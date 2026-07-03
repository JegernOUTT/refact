use refact_chat_api::{
    BuddyThreadMeta, ClaudeCodeIdentity, FrozenRequestPrefix, GoalLedgerEntry, GoalSnapshot,
    TaskMeta, ThreadParams, WorktreeMeta,
};
use refact_core::chat_types::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct TrajectorySnapshot {
    pub chat_id: String,
    pub title: String,
    pub model: String,
    pub mode: String,
    pub tool_use: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
    pub boost_reasoning: bool,
    pub checkpoints_enabled: bool,
    pub context_tokens_cap: Option<usize>,
    pub include_project_info: bool,
    pub is_title_generated: bool,
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
    pub autonomous_no_confirm: bool,
    pub version: u64,
    pub task_meta: Option<TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub parent_id: Option<String>,
    pub link_type: Option<String>,
    pub root_chat_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub thinking_budget: Option<usize>,
    pub temperature: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub max_tokens: Option<usize>,
    pub parallel_tool_calls: Option<bool>,
    pub previous_response_id: Option<String>,
    pub active_skill: Option<String>,
    pub auto_enrichment_enabled: Option<bool>,
    pub buddy_meta: Option<BuddyThreadMeta>,
    pub auto_compact_enabled: Option<bool>,
    pub frozen_request_prefix: Option<FrozenRequestPrefix>,
    pub claude_code_identity: Option<ClaudeCodeIdentity>,
    pub reactive_compact_attempts: Option<usize>,
    pub wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    pub waiting_for_card_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<GoalSnapshot>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "tolerant_goal_ledger"
    )]
    pub goal_ledger: Vec<GoalLedgerEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_verification_blocked_until_ms: Option<u64>,
}

fn tolerant_goal_ledger<'de, D>(deserializer: D) -> Result<Vec<GoalLedgerEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<serde_json::Value>::deserialize(deserializer).unwrap_or_default();
    Ok(raw
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect())
}

impl TrajectorySnapshot {
    pub fn from_thread_parts(
        chat_id: String,
        thread: &ThreadParams,
        messages: Vec<ChatMessage>,
        created_at: String,
        version: u64,
    ) -> Self {
        Self {
            chat_id,
            title: thread.title.clone(),
            model: thread.model.clone(),
            mode: thread.mode.clone(),
            tool_use: thread.tool_use.clone(),
            messages,
            created_at,
            boost_reasoning: thread.boost_reasoning.unwrap_or(false),
            checkpoints_enabled: thread.checkpoints_enabled,
            context_tokens_cap: thread.context_tokens_cap,
            include_project_info: thread.include_project_info,
            is_title_generated: thread.is_title_generated,
            auto_approve_editing_tools: thread.auto_approve_editing_tools,
            auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
            autonomous_no_confirm: thread.autonomous_no_confirm,
            version,
            task_meta: thread.task_meta.clone(),
            worktree: thread.worktree.clone(),
            parent_id: thread.parent_id.clone(),
            link_type: thread.link_type.clone(),
            root_chat_id: thread.root_chat_id.clone(),
            reasoning_effort: thread.reasoning_effort.clone(),
            thinking_budget: thread.thinking_budget,
            temperature: thread.temperature,
            frequency_penalty: thread.frequency_penalty,
            max_tokens: thread.max_tokens,
            parallel_tool_calls: thread.parallel_tool_calls,
            previous_response_id: thread.previous_response_id.clone(),
            active_skill: thread.active_skill.clone(),
            auto_enrichment_enabled: thread.auto_enrichment_enabled,
            buddy_meta: thread.buddy_meta.clone(),
            auto_compact_enabled: thread.auto_compact_enabled,
            frozen_request_prefix: thread.frozen_request_prefix.clone(),
            claude_code_identity: thread.claude_code_identity.clone(),
            reactive_compact_attempts: thread.reactive_compact_attempts,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            goal: None,
            goal_ledger: Vec::new(),
            goal_verification_blocked_until_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::{GoalBudget, GoalLedgerOp, GoalProgress, GoalStatus};

    fn goal_snapshot() -> GoalSnapshot {
        GoalSnapshot {
            content: "ship the card".to_string(),
            version: 1,
            active: true,
            status: GoalStatus::Active,
            budget: GoalBudget::default(),
            progress: GoalProgress {
                turns_used: 2,
                tokens_used: 120,
                started_at_ms: 123,
                no_progress_turns: 0,
                last_nudge_at_ms: 456,
                cost_used_cents: 0,
            },
            attempts: Vec::new(),
            events: Vec::new(),
            criteria: Vec::new(),
            snoozed_until_ms: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
        }
    }

    fn snapshot() -> TrajectorySnapshot {
        TrajectorySnapshot::from_thread_parts(
            "chat-id".to_string(),
            &ThreadParams::default(),
            Vec::new(),
            "2026-06-18T00:00:00Z".to_string(),
            1,
        )
    }

    #[test]
    fn from_thread_parts_defaults_goal_none() {
        let snapshot = snapshot();

        assert_eq!(snapshot.goal, None);
    }

    #[test]
    fn trajectory_snapshot_goal_roundtrip() {
        let mut snapshot = snapshot();
        let goal = goal_snapshot();
        snapshot.goal = Some(goal.clone());

        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: TrajectorySnapshot = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.goal, Some(goal));
    }

    #[test]
    fn tolerant_goal_ledger_skips_unknown_ops_and_keeps_known() {
        let mut snapshot = snapshot();
        snapshot.goal_ledger = vec![GoalLedgerEntry {
            seq: 1,
            at_ms: 1_000,
            op: GoalLedgerOp::NudgeRecorded,
        }];
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["goal_ledger"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"seq": 2, "at_ms": 2_000, "op": "from_the_future", "x": 1}));

        let decoded: TrajectorySnapshot = serde_json::from_value(value).unwrap();

        assert_eq!(decoded.goal_ledger.len(), 1);
        assert_eq!(decoded.goal_ledger[0].seq, 1);
    }

    #[test]
    fn goal_verification_blocked_until_ms_roundtrips() {
        let mut snapshot = snapshot();
        snapshot.goal_verification_blocked_until_ms = Some(4_242);

        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: TrajectorySnapshot = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.goal_verification_blocked_until_ms, Some(4_242));
    }

    #[test]
    fn trajectory_snapshot_missing_goal_defaults_none() {
        let snapshot = snapshot();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();

        assert!(value.get("goal").is_none());
        let decoded: TrajectorySnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.goal, None);
    }
}
