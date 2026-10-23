use std::path::PathBuf;

use chrono::{DateTime, Utc};
pub use refact_chat_api::{AgentQuestionSummary, BackgroundAgentSummary};
use serde::{Deserialize, Serialize};
use refact_chat_api::{DeliveryStatusRecord, PendingDelivery, PushMode};

pub const NO_TEXT_RESULT_SUMMARY: &str =
    "<no text response — see child trajectory for full details>";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BgAgentKind {
    Subagent,
    /// Legacy persisted-record variant; new unified agents use Subagent.
    Delegate,
}

impl BgAgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Subagent => "subagent",
            Self::Delegate => "delegate",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BgAgentStatus {
    Queued,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    /// Deliberately closed by its parent: terminal, but the saved child can be
    /// resumed at the next generation.
    Closed,
}

impl BgAgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted | Self::Closed
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Closed => "closed",
        }
    }
}

/// The outcome of one finished generation, preserved when the same child is
/// closed, resumed or asked for a continued turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRunOutcome {
    pub run_generation: u64,
    pub status: BgAgentStatus,
    pub step_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_status: Option<String>,
}

pub type AgentQuestion = refact_chat_api::QuestionContract;

pub use refact_work_core::{CapKind, CapScope, CapStop};

#[cfg(test)]
mod cap_stop_tests {
    use super::CapStop;

    #[test]
    fn typed_cap_round_trips() {
        let stop = CapStop {
            key: super::CapKind::Steps,
            scope: super::CapScope::Child,
            limit: 100,
            used: 100,
            resumable: true,
            run_generation: 3,
        };
        assert_eq!(CapStop::from_error(&stop.as_error().unwrap()), Some(stop));
    }

    #[test]
    fn legacy_step_envelope_is_readable() {
        let stop = CapStop::from_error(
            r#"CAP_STOP:{"key":"max_steps","limit":10,"used":10,"resumable":true}"#,
        )
        .unwrap();
        assert_eq!(stop.scope, super::CapScope::Child);
        assert_eq!(stop.run_generation, 0);
    }

    #[test]
    fn non_caps_and_malformed_or_unreached_caps_are_not_recoverable() {
        for error in [
            "queue_full",
            "provider quota exceeded",
            "CAP_STOP:not-json",
            r#"CAP_STOP:{"key":"provider_quota","limit":10,"used":10,"resumable":true}"#,
            r#"CAP_STOP:{"key":"max_steps","scope":"engine","limit":10,"used":10,"resumable":true}"#,
            r#"CAP_STOP:{"key":"max_steps","limit":10,"used":9,"resumable":true}"#,
            r#"CAP_STOP:{"key":"max_steps","limit":0,"used":0,"resumable":true}"#,
        ] {
            assert!(CapStop::from_error(error).is_none(), "{error}");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundAgent {
    pub schema_version: u32,
    #[serde(default = "initial_run_generation")]
    pub run_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_hook_generation: Option<u64>,
    pub agent_id: String,
    pub parent_chat_id: String,
    pub parent_root_chat_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub child_chat_id: Option<String>,
    pub kind: BgAgentKind,
    pub config_name: String,
    pub title: String,
    pub prompt: String,
    pub target_files: Vec<String>,
    pub status: BgAgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<DateTime<Utc>>,
    pub progress: Option<String>,
    pub step_count: u32,
    pub last_activity: Option<String>,
    pub result_summary: Option<String>,
    pub result_payload_path: Option<PathBuf>,
    pub error: Option<String>,
    pub edited_files: Vec<String>,
    pub diff_summary: Option<String>,
    pub conflict_summary: Option<String>,
    pub completion_message_id: Option<String>,
    pub completion_pushed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completion_push: PushMode,
    #[serde(default)]
    pub pending_deliveries: Vec<PendingDelivery>,
    #[serde(default)]
    pub delivery_ids: Vec<String>,
    /// Deliveries dropped before the runner consumed them (deadline passed or
    /// superseded by a newer delivery with the same key); bounded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_deliveries: Vec<DeliveryStatusRecord>,
    #[serde(default)]
    pub deferred_at: Option<DateTime<Utc>>,
    /// Why the parent closed this child; set only while the record is closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    /// Outcomes of earlier generations, oldest first; bounded by
    /// `agent_closed_retained_max`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_runs: Vec<AgentRunOutcome>,
    pub model: String,
    #[serde(default)]
    pub model_type: Option<String>,
    #[serde(default)]
    pub current_tool: Option<String>,
    #[serde(default)]
    pub goal_summary: Option<String>,
    #[serde(default)]
    pub plan_present: bool,
    #[serde(default)]
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub worktree_branch: Option<String>,
    #[serde(default)]
    pub merge_status: Option<String>,
    #[serde(default)]
    pub questions: Vec<AgentQuestion>,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable: Option<refact_chat_api::DeliverableContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable_status: Option<refact_chat_api::DeliverableStatus>,
    /// The reusable profile this child was launched from, when any; the key the
    /// handoff-sufficiency metric is grouped by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
    /// The child's own claim about the brief it was given. Absent is `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<refact_chat_api::HandoffReport>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub last_update_at: DateTime<Utc>,
    pub change_seq: u64,
}

fn initial_run_generation() -> u64 {
    1
}

impl BackgroundAgent {
    pub fn is_paused(&self) -> bool {
        !self.status.is_terminal() && self.paused_by.is_some()
    }

    pub fn pause_label(&self) -> Option<String> {
        self.is_paused().then(|| {
            format!(
                "paused (by {}, since {})",
                self.paused_by.as_deref().unwrap_or_default(),
                self.paused_at.map(|at| at.to_rfc3339()).unwrap_or_default(),
            )
        })
    }
}

impl From<&BackgroundAgent> for BackgroundAgentSummary {
    fn from(record: &BackgroundAgent) -> Self {
        Self {
            agent_id: record.agent_id.clone(),
            parent_chat_id: record.parent_chat_id.clone(),
            child_chat_id: record.child_chat_id.clone(),
            kind: record.kind.as_str().to_string(),
            status: record.status.as_str().to_string(),
            title: record.title.clone(),
            progress: record.progress.clone(),
            step_count: record.step_count,
            last_activity: record.last_activity.clone(),
            target_files: record.target_files.clone(),
            edited_files: record.edited_files.clone(),
            diff_summary: record.diff_summary.clone(),
            conflict_summary: record.conflict_summary.clone(),
            result_summary: record.result_summary.clone(),
            error: record.error.clone(),
            started_at: record.started_at.as_ref().map(DateTime::to_rfc3339),
            finished_at: record.finished_at.as_ref().map(DateTime::to_rfc3339),
            change_seq: record.change_seq,
            model: record.model.clone(),
            model_type: record.model_type.clone(),
            current_tool: record.current_tool.clone(),
            goal_summary: record.goal_summary.clone(),
            plan_present: record.plan_present,
            worktree_branch: record.worktree_branch.clone(),
            merge_status: record.merge_status.clone(),
            pending_questions: record
                .questions
                .iter()
                .filter(|question| question.state == refact_chat_api::QuestionState::Pending)
                .count() as u32,
            tokens_used: record.tokens_used,
            cost_usd: record.cost_usd,
            deliverable: record.deliverable.clone(),
            deliverable_status: record.deliverable_status.clone(),
            handoff: record.handoff.clone(),
            questions: record
                .questions
                .iter()
                .map(|question| AgentQuestionSummary {
                    id: question.id.clone(),
                    text: question.text.clone(),
                    kind: Some(question.kind),
                    state: Some(question.state),
                    correlation_id: Some(question.correlation_id.clone()),
                    answer: question.answered().map(str::to_string),
                    defaulted_answer: question.defaulted().map(str::to_string),
                    asked_at: question.asked_at,
                    answered_at: question.answered_at,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateAgentRequest {
    pub parent_chat_id: String,
    pub parent_root_chat_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub kind: BgAgentKind,
    pub config_name: String,
    pub title: String,
    pub prompt: String,
    pub target_files: Vec<String>,
    pub model: String,
    pub model_type: Option<String>,
    pub goal_summary: Option<String>,
    pub plan_present: bool,
    pub worktree_id: Option<String>,
    pub worktree_branch: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentListFilter {
    pub status: Option<Vec<BgAgentStatus>>,
    pub kind: Option<BgAgentKind>,
    pub include_terminal_within_hours: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct AgentCompletion {
    pub result_summary: String,
    pub edited_files: Vec<String>,
    pub diff_summary: Option<String>,
    pub conflict_summary: Option<String>,
    pub child_chat_id: Option<String>,
    pub deliverable_status: Option<refact_chat_api::DeliverableStatus>,
    /// Declared by the child at finish; `None` is `unknown`, never "sufficient".
    pub handoff: Option<refact_chat_api::HandoffReport>,
}

#[cfg(test)]
mod handoff_record_tests {
    use super::BackgroundAgent;

    #[test]
    fn a_legacy_record_without_the_handoff_fields_still_loads() {
        let legacy = serde_json::json!({
            "schema_version": 1,
            "agent_id": "agent-1",
            "parent_chat_id": "parent",
            "parent_root_chat_id": null,
            "parent_tool_call_id": null,
            "child_chat_id": null,
            "kind": "subagent",
            "config_name": "subagent",
            "title": "legacy",
            "prompt": "prompt",
            "target_files": [],
            "status": "completed",
            "progress": null,
            "step_count": 0,
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
            "created_at": "2026-01-01T00:00:00Z",
            "started_at": null,
            "finished_at": null,
            "last_update_at": "2026-01-01T00:00:00Z",
            "change_seq": 1
        });
        let record: BackgroundAgent = serde_json::from_value(legacy).unwrap();
        assert!(record.handoff.is_none());
        assert!(record.profile_name.is_none());
        let reserialized = serde_json::to_value(&record).unwrap();
        assert!(reserialized.get("handoff").is_none());
        assert!(reserialized.get("profile_name").is_none());
    }
}
