use std::path::PathBuf;

use chrono::{DateTime, Utc};
pub use refact_chat_api::{AgentQuestionSummary, BackgroundAgentSummary};
use serde::{Deserialize, Serialize};

pub const NO_TEXT_RESULT_SUMMARY: &str =
    "<no text response — see child trajectory for full details>";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BgAgentKind {
    Subagent,
    #[doc = "Legacy persisted-record variant; new unified agents use Subagent."]
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
}

impl BgAgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentQuestion {
    pub id: String,
    pub text: String,
    pub asked_at: DateTime<Utc>,
    pub answer: Option<String>,
    pub answered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundAgent {
    pub schema_version: u32,
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
    pub deferred_at: Option<DateTime<Utc>>,
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
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub last_update_at: DateTime<Utc>,
    pub change_seq: u64,
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
                .filter(|question| question.answer.is_none())
                .count() as u32,
            tokens_used: record.tokens_used,
            cost_usd: record.cost_usd,
            questions: record
                .questions
                .iter()
                .map(|question| AgentQuestionSummary {
                    id: question.id.clone(),
                    text: question.text.clone(),
                    answer: question.answer.clone(),
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
}
