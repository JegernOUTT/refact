use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tokio::sync::{broadcast, Notify};

use crate::call_validation::{ChatMessage, ChatUsage};
use crate::git::checkpoints::Checkpoint;
use super::config::{limits, timeouts, presentation};

pub use refact_chat_api::{
    ActiveCommandContext,
    BrowserMeta,
    BrowserTabInfo,
    BuddyThreadMeta,
    ChatCommand,
    ChatEvent,
    CommandRequest,
    DeltaOp,
    DiffBox,
    EventEnvelope,
    PauseReason,
    PendingSkillDeactivation,
    QueuedItem,
    RuntimeState,
    SessionState,
    TaskMeta,
    ThreadParams,
    TimelineEntry,
    ToolDecisionItem,
    WindowBounds,
    WorktreeMeta,
};

pub fn max_queue_size() -> usize {
    limits().max_queue_size
}
pub fn session_idle_timeout() -> std::time::Duration {
    timeouts().session_idle
}
pub fn session_cleanup_interval() -> std::time::Duration {
    timeouts().session_cleanup_interval
}
pub fn stream_idle_timeout() -> std::time::Duration {
    timeouts().stream_idle
}
pub fn stream_total_timeout() -> std::time::Duration {
    timeouts().stream_total
}
pub fn stream_heartbeat() -> std::time::Duration {
    timeouts().stream_heartbeat
}

impl crate::worktrees::scope::WorktreeThread for ThreadParams {
    fn worktree(&self) -> Option<&WorktreeMeta> {
        self.worktree.as_ref()
    }
}

pub struct ChatSession {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub messages: Vec<ChatMessage>,
    pub runtime: RuntimeState,
    pub draft_message: Option<ChatMessage>,
    pub draft_usage: Option<ChatUsage>,
    pub command_queue: VecDeque<CommandRequest>,
    pub event_seq: u64,
    pub event_tx: broadcast::Sender<Arc<String>>,
    pub trajectory_events_tx: Option<broadcast::Sender<super::trajectories::TrajectoryEvent>>,
    pub recent_request_ids: VecDeque<String>,
    pub recent_request_ids_set: HashSet<String>,
    pub abort_flag: Arc<AtomicBool>,
    pub user_interrupt_flag: Arc<AtomicBool>,
    pub queue_processor_running: Arc<AtomicBool>,
    pub queue_notify: Arc<Notify>,
    pub last_activity: Instant,
    pub trajectory_dirty: bool,
    pub trajectory_version: u64,
    pub created_at: String,
    pub closed: bool,
    pub closed_flag: Arc<AtomicBool>,
    pub external_reload_pending: bool,
    pub last_prompt_messages: Vec<ChatMessage>,
    pub cache_guard_snapshot: Option<serde_json::Value>,
    pub cache_guard_force_next: bool,
    pub task_agent_error: Option<String>,
    pub pending_browser_message: Option<PendingBrowserMessage>,
    pub active_command: ActiveCommandContext,
    pub skills_available_count: usize,
    pub skills_included: Vec<String>,
    pub pending_skill_deactivation: Option<PendingSkillDeactivation>,
    pub stop_hook_handle: Option<tokio::task::JoinHandle<()>>,
    pub suppress_auto_enrichment_for_next_turn: bool,
}

#[derive(Debug, Clone)]
pub struct PendingBrowserMessage {
    pub pending_message_id: String,
    pub content: serde_json::Value,
    pub attachments: Vec<serde_json::Value>,
    pub checkpoints: Vec<Checkpoint>,
    pub context_files: Vec<serde_json::Value>,
    pub suppress_auto_enrichment: bool,
    pub skill_activation_name: Option<String>,
    pub skill_context_msg: Option<ChatMessage>,
}
