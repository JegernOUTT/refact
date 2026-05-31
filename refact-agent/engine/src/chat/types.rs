use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tokio::sync::{broadcast, Notify};

use crate::call_validation::{ChatMessage, ChatUsage};
use crate::git::checkpoints::Checkpoint;
use super::config::{limits, timeouts};

pub use refact_chat_api::{
    ActiveCommandContext, BackgroundAgentSummary, BrowserMeta, BrowserTabInfo, BuddyThreadMeta,
    ChatCommand, ChatEvent, CommandRequest, CompressionPhase, CompressionReason, DeltaOp, DiffBox,
    EventEnvelope, PauseReason, PendingSkillDeactivation, QueuedItem, RuntimeState, SessionState,
    TaskMeta, ThreadParams, TimelineEntry, ToolDecisionItem, WindowBounds, WorktreeMeta,
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

#[derive(Debug)]
pub struct BurstGuard {
    inner: tokio::sync::Mutex<BurstGuardInner>,
}

#[derive(Debug, Default)]
struct BurstGuardInner {
    recent: VecDeque<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstGuardDecision {
    Allow,
    Defer,
}

impl BurstGuard {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(BurstGuardInner::default()),
        }
    }

    pub async fn record_and_check(&self) -> BurstGuardDecision {
        let now = chrono::Utc::now();
        let mut guard = self.inner.lock().await;
        while let Some(front) = guard.recent.front() {
            if now.signed_duration_since(*front).num_seconds() > 10 {
                guard.recent.pop_front();
            } else {
                break;
            }
        }
        if guard.recent.len() >= 5 {
            BurstGuardDecision::Defer
        } else {
            guard.recent.push_back(now);
            BurstGuardDecision::Allow
        }
    }
}

impl Default for BurstGuard {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ChatSession {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub messages: Vec<ChatMessage>,
    pub runtime: RuntimeState,
    pub is_compressing: bool,
    pub compression_phase: Option<CompressionPhase>,
    pub compression_reason: Option<CompressionReason>,
    pub(crate) compression_attempt_generation: u64,
    pub(crate) active_compression_attempt: Option<u64>,
    pub draft_message: Option<ChatMessage>,
    pub draft_usage: Option<ChatUsage>,
    pub command_queue: VecDeque<CommandRequest>,
    pub event_seq: u64,
    pub event_tx: broadcast::Sender<Arc<String>>,
    pub trajectory_events_tx: Option<broadcast::Sender<super::trajectories::TrajectoryEvent>>,
    pub recent_request_ids: VecDeque<String>,
    pub recent_request_ids_set: HashSet<String>,
    pub abort_flag: Arc<AtomicBool>,
    pub abort_notify: Arc<Notify>,
    pub user_interrupt_flag: Arc<AtomicBool>,
    pub queue_processor_running: Arc<AtomicBool>,
    pub queue_notify: Arc<Notify>,
    pub last_activity: Instant,
    pub last_stream_delta_at: Option<Instant>,
    pub last_tool_started_at: Option<Instant>,
    pub last_tool_progress_at: Option<Instant>,
    pub trajectory_dirty: bool,
    pub trajectory_version: u64,
    pub created_at: String,
    pub closed: bool,
    pub closed_flag: Arc<AtomicBool>,
    pub external_reload_pending: bool,
    pub last_prompt_messages: Vec<ChatMessage>,
    pub tier1_compact_attempts: usize,
    pub tier1_compaction_disabled: bool,
    pub cache_guard_snapshot: Option<serde_json::Value>,
    pub cache_guard_force_next: bool,
    pub task_agent_error: Option<String>,
    pub pending_browser_message: Option<PendingBrowserMessage>,
    pub post_tool_side_effects: VecDeque<ChatMessage>,
    pub active_command: ActiveCommandContext,
    pub skills_available_count: usize,
    pub skills_included: Vec<String>,
    pub pending_skill_deactivation: Option<PendingSkillDeactivation>,
    pub stop_hook_handle: Option<tokio::task::JoinHandle<()>>,
    pub(crate) openai_codex_websocket: super::openai_codex_ws::OpenAICodexWebSocketSession,
    pub suppress_auto_enrichment_for_next_turn: bool,
    pub wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    pub waiting_for_card_ids: Vec<String>,
    pub background_completion_burst: BurstGuard,
    /// Latest known background agent summaries for this parent chat, keyed by `agent_id`.
    /// Kept in sync by `emit_background_agent_update` and snapshot enrichment paths so
    /// every `ChatEvent::Snapshot` carries the current agent set instead of an empty list.
    pub background_agents: HashMap<String, BackgroundAgentSummary>,
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
