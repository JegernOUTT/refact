use std::path::PathBuf;

use async_trait::async_trait;
use refact_buddy_core::snapshot::BuddySnapshot;
use refact_buddy_core::types::{BuddyRuntimeEvent, BuddySuggestion};
use refact_buddy_core::user_action::UserAction;
use refact_chat_api::{ChatCommand, ChatMessage, ContextFile, ThreadParams};
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use refact_tool_api::ToolDesc;

pub use refact_chat_api::{SessionState, TaskMeta};
pub use refact_tool_api::ToolDesc as RuntimeToolDesc;
pub use refact_buddy_core::types::BuddyRuntimeEvent as RuntimeBuddyEvent;
pub use refact_buddy_core::user_action::UserAction as RuntimeUserAction;
pub use refact_chat_history::trajectory_snapshot::TrajectorySnapshot as RuntimeTrajectorySnapshot;

#[async_trait]
pub trait ActivitySink: Send + Sync {
    async fn record_user_action(&self, action: UserAction);
}

#[async_trait]
pub trait BuddyEventSink: Send + Sync {
    async fn enqueue_event(&self, event: BuddyRuntimeEvent);
    async fn complete_event(&self, dedupe_key: &str, status: &str);
    async fn snapshot(&self) -> Option<BuddySnapshot>;
    async fn apply_chat_completion(&self, event: BuddyRuntimeEvent, xp: u64, mood: String);
    async fn report_error(
        &self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
    );
    async fn mark_chat_error(&self, event: BuddyRuntimeEvent);
    async fn maybe_add_suggestion(&self, suggestion: BuddySuggestion);
    async fn build_pulse_message(&self) -> Option<ChatMessage>;
    async fn render_runtime_event_fast(
        &self,
        workflow_id: &str,
        workflow_summary: &str,
        status: &str,
    ) -> Option<(String, Option<String>)>;
}

#[derive(Clone)]
pub struct ToolRegistryIndex {
    pub tools: Vec<ToolDesc>,
    pub mcp_lazy_mode: bool,
    pub mcp_total_count: usize,
    pub mcp_tool_index: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct ToolConfirmationCheck {
    pub tool_name: String,
    pub result: refact_tool_api::MatchConfirmDeny,
    pub integr_config_path: Option<String>,
}

#[derive(Clone)]
pub struct ToolPolicyInfo {
    pub name: String,
    pub effective_allow_parallel: bool,
}

#[derive(Clone)]
pub struct ToolExecutionResult {
    pub had_corrections: bool,
    pub messages: Vec<ChatMessage>,
    pub context_files: Vec<ContextFile>,
}

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    async fn get_tools_for_mode(&self, mode: &str, model_id: Option<&str>) -> Vec<ToolDesc>;
    async fn get_tools_index_for_mode(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> ToolRegistryIndex;
    async fn check_tool_confirmation(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ToolConfirmationCheck, String>>;
    async fn get_tool_policy_info(&self, mode: &str, model_id: Option<&str>)
        -> Vec<ToolPolicyInfo>;
    async fn execute_tool(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<ToolExecutionResult>, String>;
    async fn load_task_memories(&self, task_id: &str) -> Result<Vec<(PathBuf, String)>, String>;
}

#[derive(Clone)]
pub struct ChatSessionSnapshot {
    pub messages: Vec<ChatMessage>,
    pub thread: ThreadParams,
    pub session_state: SessionState,
}

#[derive(Clone)]
pub struct ChatSessionUpdate {
    pub messages: Vec<ChatMessage>,
}

#[derive(Clone)]
pub struct CreateSessionRequest {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub messages: Vec<ChatMessage>,
}

#[async_trait]
pub trait ChatSessionFacade: Send + Sync {
    async fn session_snapshot(&self, chat_id: &str) -> Result<ChatSessionSnapshot, String>;
    async fn update_session(&self, chat_id: &str, update: ChatSessionUpdate) -> Result<(), String>;
    async fn create_session(&self, request: CreateSessionRequest) -> Result<(), String>;
    async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String>;
    async fn session_state(&self, chat_id: &str) -> Result<Option<SessionState>, String>;
    async fn maybe_save_session(&self, chat_id: &str) -> Result<(), String>;
    async fn save_trajectory_snapshot(&self, snapshot: TrajectorySnapshot) -> Result<(), String>;
}
