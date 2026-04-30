use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Notify};
use uuid::Uuid;

use crate::call_validation::{ChatMessage, ChatUsage};
use crate::git::checkpoints::Checkpoint;
use crate::worktrees::types::WorktreeMeta;
use super::config::{limits, timeouts, presentation};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTabInfo {
    pub tab_id: String,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub timestamp: String,
    pub source: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserMeta {
    pub browser_runtime_id: Option<String>,
    pub profile_dir: Option<String>,
    #[serde(default)]
    pub tab_urls: Vec<String>,
    pub active_tab_id: Option<String>,
    pub window_bounds: Option<WindowBounds>,
    #[serde(default)]
    pub attach_screenshot_on_send: bool,
    #[serde(default = "default_true")]
    pub mask_passwords: bool,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Generating,
    ExecutingTools,
    Paused,
    WaitingIde,
    WaitingUserInput,
    Completed,
    Error,
}

impl Default for SessionState {
    fn default() -> Self {
        SessionState::Idle
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionState::Idle => write!(f, "idle"),
            SessionState::Generating => write!(f, "generating"),
            SessionState::ExecutingTools => write!(f, "executing_tools"),
            SessionState::Paused => write!(f, "paused"),
            SessionState::WaitingIde => write!(f, "waiting_ide"),
            SessionState::WaitingUserInput => write!(f, "waiting_user_input"),
            SessionState::Completed => write!(f, "completed"),
            SessionState::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TaskMeta {
    pub task_id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadParams {
    pub id: String,
    pub title: String,
    pub model: String,
    pub mode: String,
    pub tool_use: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    pub context_tokens_cap: Option<usize>,
    pub include_project_info: bool,
    pub checkpoints_enabled: bool,
    #[serde(default)]
    pub is_title_generated: bool,
    #[serde(default)]
    pub auto_approve_editing_tools: bool,
    #[serde(default)]
    pub auto_approve_dangerous_commands: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_meta: Option<TaskMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_meta: Option<BrowserMeta>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_skill: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_enrichment_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buddy_meta: Option<crate::buddy::types::BuddyThreadMeta>,
}

impl Default for ThreadParams {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: "New Chat".to_string(),
            model: String::new(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            boost_reasoning: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            context_tokens_cap: None,
            include_project_info: true,
            checkpoints_enabled: true,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            task_meta: None,
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            previous_response_id: None,
            browser_meta: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
        }
    }
}

impl crate::worktrees::scope::WorktreeThread for ThreadParams {
    fn worktree(&self) -> Option<&WorktreeMeta> {
        self.worktree.as_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedItem {
    pub client_request_id: String,
    pub priority: bool,
    pub command_type: String,
    pub preview: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub state: SessionState,
    pub paused: bool,
    pub error: Option<String>,
    pub queue_size: usize,
    #[serde(default)]
    pub pause_reasons: Vec<PauseReason>,
    #[serde(default)]
    pub queued_items: Vec<QueuedItem>,
    #[serde(default, skip_serializing)]
    pub auto_approved_tool_ids: Vec<String>,
    #[serde(default, skip_serializing)]
    pub accepted_tool_ids: Vec<String>,
    #[serde(default, skip_serializing)]
    pub paused_message_index: Option<usize>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            state: SessionState::Idle,
            paused: false,
            error: None,
            queue_size: 0,
            pause_reasons: Vec::new(),
            queued_items: Vec::new(),
            auto_approved_tool_ids: Vec::new(),
            accepted_tool_ids: Vec::new(),
            paused_message_index: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseReason {
    #[serde(rename = "type")]
    pub reason_type: String,
    pub tool_name: String,
    pub command: String,
    pub rule: String,
    pub tool_call_id: String,
    pub integr_config_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Snapshot {
        thread: ThreadParams,
        runtime: RuntimeState,
        messages: Vec<ChatMessage>,
    },
    ThreadUpdated {
        #[serde(flatten)]
        params: serde_json::Value,
    },
    QueueUpdated {
        queue_size: usize,
        queued_items: Vec<QueuedItem>,
    },
    MessageAdded {
        message: ChatMessage,
        index: usize,
    },
    MessageUpdated {
        message_id: String,
        message: ChatMessage,
    },
    MessageRemoved {
        message_id: String,
    },
    MessagesTruncated {
        from_index: usize,
    },
    StreamStarted {
        message_id: String,
    },
    StreamDelta {
        message_id: String,
        ops: Vec<DeltaOp>,
    },
    StreamFinished {
        message_id: String,
        finish_reason: Option<String>,
    },
    PauseRequired {
        reasons: Vec<PauseReason>,
    },
    PauseCleared {},
    IdeToolRequired {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    SubchatUpdate {
        tool_call_id: String,
        subchat_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attached_files: Vec<String>,
    },
    RuntimeUpdated {
        state: SessionState,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Ack {
        client_request_id: String,
        accepted: bool,
        result: Option<serde_json::Value>,
    },
    BrowserFrame {
        tab_id: String,
        mime: String,
        data: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diff_boxes: Vec<DiffBox>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        changed_text: Option<String>,
    },
    BrowserStatus {
        runtime_id: String,
        connected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tabs: Vec<BrowserTabInfo>,
    },
    BrowserClosed {
        runtime_id: String,
        reason: String,
    },
    BrowserTimeline {
        events: Vec<TimelineEntry>,
    },
    BrowserContextOversize {
        total_bytes: usize,
        action_count: usize,
        action_bytes: usize,
        console_count: usize,
        console_bytes: usize,
        network_count: usize,
        network_bytes: usize,
        mutation_bytes: usize,
        pending_message_id: String,
    },
    BrowserToolbarAction {
        action: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DeltaOp {
    AppendContent {
        text: String,
    },
    AppendReasoning {
        text: String,
    },
    SetToolCalls {
        tool_calls: Vec<serde_json::Value>,
    },
    SetThinkingBlocks {
        blocks: Vec<serde_json::Value>,
    },
    AddCitation {
        citation: serde_json::Value,
    },
    AddServerContentBlock {
        block: serde_json::Value,
    },
    SetUsage {
        usage: serde_json::Value,
    },
    MergeExtra {
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub chat_id: String,
    #[serde(
        serialize_with = "serialize_seq_as_string",
        deserialize_with = "deserialize_seq_from_string"
    )]
    pub seq: u64,
    #[serde(flatten)]
    pub event: ChatEvent,
}

fn serialize_seq_as_string<S>(seq: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&seq.to_string())
}

fn deserialize_seq_from_string<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    s.parse().map_err(D::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatCommand {
    UserMessage {
        content: serde_json::Value,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        context_files: Vec<serde_json::Value>,
        #[serde(default)]
        suppress_auto_enrichment: bool,
    },
    RetryFromIndex {
        index: usize,
        content: serde_json::Value,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
    },
    SetParams {
        patch: serde_json::Value,
    },
    Abort {},
    ToolDecision {
        tool_call_id: String,
        accepted: bool,
    },
    ToolDecisions {
        decisions: Vec<ToolDecisionItem>,
    },
    IdeToolResult {
        tool_call_id: String,
        content: String,
        #[serde(default)]
        tool_failed: bool,
    },
    UpdateMessage {
        message_id: String,
        content: serde_json::Value,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
        #[serde(default)]
        regenerate: bool,
    },
    RemoveMessage {
        message_id: String,
        #[serde(default)]
        regenerate: bool,
    },
    Regenerate {},
    RestoreMessages {
        messages: Vec<serde_json::Value>,
    },
    BranchFromChat {
        source_chat_id: String,
        up_to_message_id: String,
    },
    BrowserContextDecision {
        pending_message_id: String,
        #[serde(default = "default_true")]
        include_actions: bool,
        #[serde(default = "default_true")]
        include_console: bool,
        #[serde(default = "default_true")]
        include_network: bool,
        #[serde(default = "default_true")]
        include_mutations: bool,
        #[serde(default = "default_true")]
        include_screenshot: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n_actions: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n_console: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n_network: Option<usize>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecisionItem {
    pub tool_call_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRequest {
    pub client_request_id: String,
    #[serde(default)]
    pub priority: bool,
    #[serde(flatten)]
    pub command: ChatCommand,
}

impl CommandRequest {
    pub fn to_queued_item(&self) -> QueuedItem {
        let (command_type, preview, content) = match &self.command {
            ChatCommand::UserMessage {
                content,
                context_files,
                ..
            } => {
                let full = extract_full_text_capped(content);
                let mut preview = extract_preview(content);
                if !context_files.is_empty() {
                    preview = format!("[+{} ctx] {}", context_files.len(), preview);
                }
                ("user_message".to_string(), preview, full)
            }
            ChatCommand::RetryFromIndex { content, index, .. } => (
                "retry_from_index".to_string(),
                format!("@{}: {}", index, extract_preview(content)),
                String::new(),
            ),
            ChatCommand::SetParams { patch } => {
                let model = patch.get("model").and_then(|v| v.as_str()).unwrap_or("");
                (
                    "set_params".to_string(),
                    format!("model={}", model),
                    String::new(),
                )
            }
            ChatCommand::Abort {} => ("abort".to_string(), String::new(), String::new()),
            ChatCommand::ToolDecision {
                tool_call_id,
                accepted,
            } => (
                "tool_decision".to_string(),
                format!("{}: {}", tool_call_id, accepted),
                String::new(),
            ),
            ChatCommand::ToolDecisions { decisions } => (
                "tool_decisions".to_string(),
                format!("{} decisions", decisions.len()),
                String::new(),
            ),
            ChatCommand::IdeToolResult { tool_call_id, .. } => (
                "ide_tool_result".to_string(),
                tool_call_id.clone(),
                String::new(),
            ),
            ChatCommand::UpdateMessage { message_id, .. } => (
                "update_message".to_string(),
                message_id.clone(),
                String::new(),
            ),
            ChatCommand::RemoveMessage { message_id, .. } => (
                "remove_message".to_string(),
                message_id.clone(),
                String::new(),
            ),
            ChatCommand::Regenerate {} => ("regenerate".to_string(), String::new(), String::new()),
            ChatCommand::RestoreMessages { messages } => (
                "restore_messages".to_string(),
                format!("{} messages", messages.len()),
                String::new(),
            ),
            ChatCommand::BranchFromChat { source_chat_id, .. } => (
                "branch_from_chat".to_string(),
                source_chat_id.clone(),
                String::new(),
            ),
            ChatCommand::BrowserContextDecision {
                pending_message_id, ..
            } => (
                "browser_context_decision".to_string(),
                pending_message_id.clone(),
                String::new(),
            ),
        };
        QueuedItem {
            client_request_id: self.client_request_id.clone(),
            priority: self.priority,
            command_type,
            preview,
            content,
        }
    }
}

const MAX_CONTENT_CHARS: usize = 8192;

fn extract_full_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .find_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .unwrap_or_default();
    }
    String::new()
}

fn extract_full_text_capped(content: &serde_json::Value) -> String {
    let text = extract_full_text(content);
    if text.chars().count() > MAX_CONTENT_CHARS {
        format!(
            "{}…",
            text.chars().take(MAX_CONTENT_CHARS).collect::<String>()
        )
    } else {
        text
    }
}

fn extract_preview(content: &serde_json::Value) -> String {
    let max_preview = presentation().preview_chars;
    let text = if let Some(s) = content.as_str() {
        s.to_string()
    } else if let Some(arr) = content.as_array() {
        arr.iter()
            .find_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "[Image attachment]".to_string())
    } else {
        String::new()
    };
    if text.chars().count() > max_preview {
        format!("{}…", text.chars().take(max_preview).collect::<String>())
    } else {
        text
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
    /// Mirrors `closed` as an atomic so SSE heartbeat loops can check it
    /// without acquiring the session mutex on every tick.
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

#[derive(Debug, Clone, Default)]
pub struct ActiveCommandContext {
    pub name: String,
    pub allowed_tools: Vec<String>,
    pub model_override: Option<String>,
    pub context_fork: Option<String>,
    pub started_at_index: Option<usize>,
    pub activation_tool_call_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingSkillDeactivation {
    pub start_index: usize,
    pub report: String,
    pub skill_name: String,
    pub activation_tool_call_id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_session_state_default() {
        assert_eq!(SessionState::default(), SessionState::Idle);
    }

    #[test]
    fn test_session_state_serde() {
        let state = SessionState::Generating;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"generating\"");

        let parsed: SessionState = serde_json::from_str("\"executing_tools\"").unwrap();
        assert_eq!(parsed, SessionState::ExecutingTools);
    }

    #[test]
    fn test_thread_params_default() {
        let params = ThreadParams::default();
        assert_eq!(params.title, "New Chat");
        assert_eq!(params.mode, "agent");
        assert_eq!(params.tool_use, "agent");
        assert!(params.boost_reasoning.is_none());
        assert!(params.reasoning_effort.is_none());
        assert!(params.temperature.is_none());
        assert!(params.frequency_penalty.is_none());
        assert!(params.max_tokens.is_none());
        assert!(params.parallel_tool_calls.is_none());
        assert!(params.include_project_info);
        assert!(params.checkpoints_enabled);
        assert!(!params.is_title_generated);
        assert!(params.context_tokens_cap.is_none());
        assert!(params.worktree.is_none());
        assert!(!params.id.is_empty());
    }

    #[test]
    fn test_runtime_state_default() {
        let runtime = RuntimeState::default();
        assert_eq!(runtime.state, SessionState::Idle);
        assert!(!runtime.paused);
        assert!(runtime.error.is_none());
        assert_eq!(runtime.queue_size, 0);
        assert!(runtime.pause_reasons.is_empty());
    }

    #[test]
    fn test_event_envelope_seq_serializes_as_string() {
        let envelope = EventEnvelope {
            chat_id: "test-123".to_string(),
            seq: 42,
            event: ChatEvent::PauseCleared {},
        };
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["seq"], "42");
        assert_eq!(json["chat_id"], "test-123");
    }

    #[test]
    fn test_event_envelope_seq_deserializes_from_string() {
        let json = r#"{"chat_id":"abc","seq":"999","type":"pause_cleared"}"#;
        let envelope: EventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.seq, 999);
        assert_eq!(envelope.chat_id, "abc");
    }

    #[test]
    fn test_event_envelope_invalid_seq_fails() {
        let json = r#"{"chat_id":"abc","seq":"not_a_number","type":"pause_cleared"}"#;
        let result: Result<EventEnvelope, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_chat_command_user_message_defaults() {
        let json = r#"{"type":"user_message","content":"hello"}"#;
        let cmd: ChatCommand = serde_json::from_str(json).unwrap();
        match cmd {
            ChatCommand::UserMessage {
                content,
                attachments,
                context_files,
                suppress_auto_enrichment: _,
            } => {
                assert_eq!(content, json!("hello"));
                assert!(attachments.is_empty());
                assert!(context_files.is_empty());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_chat_command_ide_tool_result_defaults() {
        let json = r#"{"type":"ide_tool_result","tool_call_id":"tc1","content":"result"}"#;
        let cmd: ChatCommand = serde_json::from_str(json).unwrap();
        match cmd {
            ChatCommand::IdeToolResult {
                tool_call_id,
                content,
                tool_failed,
            } => {
                assert_eq!(tool_call_id, "tc1");
                assert_eq!(content, "result");
                assert!(!tool_failed);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_chat_command_update_message_defaults() {
        let json = r#"{"type":"update_message","message_id":"m1","content":"new"}"#;
        let cmd: ChatCommand = serde_json::from_str(json).unwrap();
        match cmd {
            ChatCommand::UpdateMessage {
                message_id,
                content,
                attachments,
                regenerate,
            } => {
                assert_eq!(message_id, "m1");
                assert_eq!(content, json!("new"));
                assert!(attachments.is_empty());
                assert!(!regenerate);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_chat_command_remove_message_defaults() {
        let json = r#"{"type":"remove_message","message_id":"m1"}"#;
        let cmd: ChatCommand = serde_json::from_str(json).unwrap();
        match cmd {
            ChatCommand::RemoveMessage {
                message_id,
                regenerate,
            } => {
                assert_eq!(message_id, "m1");
                assert!(!regenerate);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_chat_command_all_variants_roundtrip() {
        let commands = vec![
            json!({"type":"user_message","content":"hi","attachments":[]}),
            json!({"type":"retry_from_index","index":2,"content":"retry","attachments":[]}),
            json!({"type":"set_params","patch":{"title":"New"}}),
            json!({"type":"abort"}),
            json!({"type":"tool_decision","tool_call_id":"tc1","accepted":true}),
            json!({"type":"tool_decisions","decisions":[{"tool_call_id":"tc1","accepted":false}]}),
            json!({"type":"ide_tool_result","tool_call_id":"tc1","content":"ok","tool_failed":false}),
            json!({"type":"update_message","message_id":"m1","content":"x","attachments":[],"regenerate":true}),
            json!({"type":"remove_message","message_id":"m1","regenerate":false}),
        ];
        for cmd_json in commands {
            let cmd: ChatCommand = serde_json::from_value(cmd_json.clone()).unwrap();
            let roundtrip = serde_json::to_value(&cmd).unwrap();
            assert_eq!(roundtrip["type"], cmd_json["type"]);
        }
    }

    #[test]
    fn test_delta_op_serde() {
        let ops = vec![
            DeltaOp::AppendContent {
                text: "hello".into(),
            },
            DeltaOp::AppendReasoning {
                text: "thinking".into(),
            },
            DeltaOp::SetToolCalls {
                tool_calls: vec![json!({"id":"1"})],
            },
            DeltaOp::SetThinkingBlocks {
                blocks: vec![json!({"type":"thinking"})],
            },
            DeltaOp::AddCitation {
                citation: json!({"url":"http://x"}),
            },
            DeltaOp::AddServerContentBlock {
                block: json!({"type":"server_tool_use","id":"srvtoolu_1","name":"web_search"}),
            },
            DeltaOp::SetUsage {
                usage: json!({"total_tokens":100}),
            },
            DeltaOp::MergeExtra {
                extra: serde_json::Map::new(),
            },
        ];
        for op in ops {
            let json = serde_json::to_value(&op).unwrap();
            let parsed: DeltaOp = serde_json::from_value(json).unwrap();
            assert_eq!(
                serde_json::to_string(&op).unwrap(),
                serde_json::to_string(&parsed).unwrap()
            );
        }
    }

    #[test]
    fn test_chat_event_snapshot_serde() {
        let event = ChatEvent::Snapshot {
            thread: ThreadParams::default(),
            runtime: RuntimeState::default(),
            messages: vec![],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "snapshot");
        let parsed: ChatEvent = serde_json::from_value(json).unwrap();
        matches!(parsed, ChatEvent::Snapshot { .. });
    }

    #[test]
    fn test_chat_event_stream_delta_serde() {
        let event = ChatEvent::StreamDelta {
            message_id: "m1".into(),
            ops: vec![DeltaOp::AppendContent { text: "x".into() }],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "stream_delta");
        assert_eq!(json["message_id"], "m1");
    }

    #[test]
    fn test_pause_reason_serde() {
        let reason = PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "shell".into(),
            rule: "ask".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: Some("/path".into()),
        };
        let json = serde_json::to_value(&reason).unwrap();
        assert_eq!(json["type"], "confirmation");
        assert_eq!(json["tool_name"], "shell");
        assert_eq!(json["integr_config_path"], "/path");
    }

    #[test]
    fn test_command_request_flattens_command() {
        let req = CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::Abort {},
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["client_request_id"], "req-1");
        assert_eq!(json["type"], "abort");
    }

    #[test]
    fn test_runtime_updated_serde() {
        let event = ChatEvent::RuntimeUpdated {
            state: SessionState::Completed,
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "runtime_updated");
        assert_eq!(json["state"], "completed");
        assert!(json.get("error").is_none());

        let event_with_error = ChatEvent::RuntimeUpdated {
            state: SessionState::Error,
            error: Some("test error".into()),
        };
        let json2 = serde_json::to_value(&event_with_error).unwrap();
        assert_eq!(json2["type"], "runtime_updated");
        assert_eq!(json2["state"], "error");
        assert_eq!(json2["error"], "test error");
    }

    #[test]
    fn test_browser_meta_serde_roundtrip_full() {
        let meta = BrowserMeta {
            browser_runtime_id: Some("rt-123".to_string()),
            profile_dir: Some("/tmp/chrome-profile".to_string()),
            tab_urls: vec![
                "https://example.com".to_string(),
                "https://test.com".to_string(),
            ],
            active_tab_id: Some("tab-1".to_string()),
            window_bounds: Some(WindowBounds {
                x: 100,
                y: 200,
                width: 1920,
                height: 1080,
            }),
            attach_screenshot_on_send: true,
            mask_passwords: false,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["browser_runtime_id"], "rt-123");
        assert_eq!(json["profile_dir"], "/tmp/chrome-profile");
        assert_eq!(json["tab_urls"].as_array().unwrap().len(), 2);
        assert_eq!(json["active_tab_id"], "tab-1");
        assert_eq!(json["window_bounds"]["x"], 100);
        assert_eq!(json["window_bounds"]["width"], 1920);
        assert_eq!(json["attach_screenshot_on_send"], true);
        assert_eq!(json["mask_passwords"], false);

        let roundtrip: BrowserMeta = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.browser_runtime_id.as_deref(), Some("rt-123"));
        assert_eq!(roundtrip.tab_urls.len(), 2);
        assert!(roundtrip.attach_screenshot_on_send);
        assert!(!roundtrip.mask_passwords);
    }

    #[test]
    fn test_browser_meta_serde_roundtrip_minimal() {
        let json_str = r#"{}"#;
        let meta: BrowserMeta = serde_json::from_str(json_str).unwrap();
        assert!(meta.browser_runtime_id.is_none());
        assert!(meta.profile_dir.is_none());
        assert!(meta.tab_urls.is_empty());
        assert!(meta.active_tab_id.is_none());
        assert!(meta.window_bounds.is_none());
        assert!(!meta.attach_screenshot_on_send);
        assert!(meta.mask_passwords);
    }

    #[test]
    fn test_thread_params_without_browser_meta_omits_field() {
        let params = ThreadParams::default();
        assert!(params.browser_meta.is_none());
        let json = serde_json::to_value(&params).unwrap();
        assert!(json.get("browser_meta").is_none());
    }

    #[test]
    fn test_thread_params_with_browser_meta_roundtrip() {
        let mut params = ThreadParams::default();
        params.browser_meta = Some(BrowserMeta {
            browser_runtime_id: Some("rt-456".to_string()),
            profile_dir: None,
            tab_urls: vec!["https://example.com".to_string()],
            active_tab_id: None,
            window_bounds: None,
            attach_screenshot_on_send: false,
            mask_passwords: true,
        });
        let json = serde_json::to_value(&params).unwrap();
        assert!(json.get("browser_meta").is_some());
        assert_eq!(json["browser_meta"]["browser_runtime_id"], "rt-456");

        let roundtrip: ThreadParams = serde_json::from_value(json).unwrap();
        assert!(roundtrip.browser_meta.is_some());
        let bm = roundtrip.browser_meta.unwrap();
        assert_eq!(bm.browser_runtime_id.as_deref(), Some("rt-456"));
        assert_eq!(bm.tab_urls.len(), 1);
    }

    #[test]
    fn test_thread_params_backward_compat_no_browser_meta() {
        let json_str = r#"{"id":"test","title":"Test","model":"gpt-4","mode":"agent","tool_use":"agent","include_project_info":true,"checkpoints_enabled":true}"#;
        let params: ThreadParams = serde_json::from_str(json_str).unwrap();
        assert!(params.browser_meta.is_none());
        assert_eq!(params.id, "test");
        assert_eq!(params.mode, "agent");
    }

    #[test]
    fn test_thread_params_backward_compat_no_worktree() {
        let json_str = r#"{"id":"test","title":"Test","model":"gpt-4","mode":"agent","tool_use":"agent","include_project_info":true,"checkpoints_enabled":true}"#;
        let params: ThreadParams = serde_json::from_str(json_str).unwrap();
        assert!(params.worktree.is_none());
        assert_eq!(params.id, "test");
        assert_eq!(params.mode, "agent");
    }

    #[test]
    fn test_chat_event_browser_frame_serde() {
        let event = ChatEvent::BrowserFrame {
            tab_id: "tab-1".to_string(),
            mime: "image/jpeg".to_string(),
            data: "base64data".to_string(),
            diff_boxes: vec![DiffBox {
                x: 10,
                y: 20,
                width: 100,
                height: 50,
            }],
            changed_text: Some("button clicked".to_string()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_frame");
        assert_eq!(json["tab_id"], "tab-1");
        assert_eq!(json["mime"], "image/jpeg");
        assert_eq!(json["data"], "base64data");
        assert_eq!(json["diff_boxes"][0]["x"], 10);
        assert_eq!(json["diff_boxes"][0]["width"], 100);
        assert_eq!(json["changed_text"], "button clicked");
        let parsed: ChatEvent = serde_json::from_value(json).unwrap();
        match parsed {
            ChatEvent::BrowserFrame {
                tab_id,
                mime,
                diff_boxes,
                changed_text,
                ..
            } => {
                assert_eq!(tab_id, "tab-1");
                assert_eq!(mime, "image/jpeg");
                assert_eq!(diff_boxes.len(), 1);
                assert_eq!(changed_text, Some("button clicked".to_string()));
            }
            _ => panic!("Expected BrowserFrame"),
        }
    }

    #[test]
    fn test_chat_event_browser_frame_minimal() {
        let event = ChatEvent::BrowserFrame {
            tab_id: "tab-2".to_string(),
            mime: "image/png".to_string(),
            data: "abc123".to_string(),
            diff_boxes: vec![],
            changed_text: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_frame");
        assert!(json.get("diff_boxes").is_none());
        assert!(json.get("changed_text").is_none());
    }

    #[test]
    fn test_chat_event_browser_status_serde() {
        let event = ChatEvent::BrowserStatus {
            runtime_id: "rt-1".to_string(),
            connected: true,
            active_tab: Some("tab-1".to_string()),
            url: Some("https://example.com".to_string()),
            title: Some("Example".to_string()),
            tabs: vec![BrowserTabInfo {
                tab_id: "tab-1".to_string(),
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
            }],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_status");
        assert_eq!(json["runtime_id"], "rt-1");
        assert_eq!(json["connected"], true);
        assert_eq!(json["active_tab"], "tab-1");
        assert_eq!(json["tabs"][0]["tab_id"], "tab-1");
        let parsed: ChatEvent = serde_json::from_value(json).unwrap();
        match parsed {
            ChatEvent::BrowserStatus {
                runtime_id,
                connected,
                tabs,
                ..
            } => {
                assert_eq!(runtime_id, "rt-1");
                assert!(connected);
                assert_eq!(tabs.len(), 1);
            }
            _ => panic!("Expected BrowserStatus"),
        }
    }

    #[test]
    fn test_chat_event_browser_status_minimal() {
        let event = ChatEvent::BrowserStatus {
            runtime_id: "rt-2".to_string(),
            connected: false,
            active_tab: None,
            url: None,
            title: None,
            tabs: vec![],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_status");
        assert_eq!(json["connected"], false);
        assert!(json.get("active_tab").is_none());
        assert!(json.get("url").is_none());
        assert!(json.get("tabs").is_none());
    }

    #[test]
    fn test_chat_event_browser_closed_serde() {
        let event = ChatEvent::BrowserClosed {
            runtime_id: "rt-1".to_string(),
            reason: "user_closed".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_closed");
        assert_eq!(json["runtime_id"], "rt-1");
        assert_eq!(json["reason"], "user_closed");
        let parsed: ChatEvent = serde_json::from_value(json).unwrap();
        match parsed {
            ChatEvent::BrowserClosed { runtime_id, reason } => {
                assert_eq!(runtime_id, "rt-1");
                assert_eq!(reason, "user_closed");
            }
            _ => panic!("Expected BrowserClosed"),
        }
    }

    #[test]
    fn test_chat_event_browser_timeline_serde() {
        let event = ChatEvent::BrowserTimeline {
            events: vec![
                TimelineEntry {
                    timestamp: "2025-01-01T10:00:00Z".to_string(),
                    source: "user".to_string(),
                    entry_type: "click".to_string(),
                    summary: "Clicked #submit-btn".to_string(),
                    details: Some(json!({"selector": "#submit-btn"})),
                },
                TimelineEntry {
                    timestamp: "2025-01-01T10:00:01Z".to_string(),
                    source: "agent".to_string(),
                    entry_type: "navigate".to_string(),
                    summary: "Navigated to page".to_string(),
                    details: None,
                },
            ],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_timeline");
        assert_eq!(json["events"].as_array().unwrap().len(), 2);
        assert_eq!(json["events"][0]["source"], "user");
        assert_eq!(json["events"][1]["type"], "navigate");
        let parsed: ChatEvent = serde_json::from_value(json).unwrap();
        match parsed {
            ChatEvent::BrowserTimeline { events } => {
                assert_eq!(events.len(), 2);
                assert_eq!(events[0].entry_type, "click");
            }
            _ => panic!("Expected BrowserTimeline"),
        }
    }

    #[test]
    fn test_diff_box_serde() {
        let db = DiffBox {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        let json = serde_json::to_value(&db).unwrap();
        assert_eq!(json["x"], 10);
        assert_eq!(json["y"], 20);
        assert_eq!(json["width"], 100);
        assert_eq!(json["height"], 50);
        let parsed: DiffBox = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, db);
    }

    #[test]
    fn test_browser_tab_info_serde() {
        let tab = BrowserTabInfo {
            tab_id: "t1".to_string(),
            url: "https://test.com".to_string(),
            title: "Test".to_string(),
        };
        let json = serde_json::to_value(&tab).unwrap();
        let parsed: BrowserTabInfo = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.tab_id, "t1");
        assert_eq!(parsed.url, "https://test.com");
    }

    #[test]
    fn test_timeline_entry_serde() {
        let entry = TimelineEntry {
            timestamp: "2025-01-01T10:00:00Z".to_string(),
            source: "user".to_string(),
            entry_type: "input".to_string(),
            summary: "Typed text".to_string(),
            details: Some(json!({"text": "typed text"})),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["timestamp"], "2025-01-01T10:00:00Z");
        assert_eq!(json["type"], "input");
        assert_eq!(json["summary"], "Typed text");
        let parsed: TimelineEntry = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.entry_type, "input");
    }

    #[test]
    fn test_browser_events_in_event_envelope() {
        let envelope = EventEnvelope {
            chat_id: "chat-1".to_string(),
            seq: 5,
            event: ChatEvent::BrowserFrame {
                tab_id: "t1".to_string(),
                mime: "image/jpeg".to_string(),
                data: "base64".to_string(),
                diff_boxes: vec![],
                changed_text: None,
            },
        };
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["chat_id"], "chat-1");
        assert_eq!(json["seq"], "5");
        assert_eq!(json["type"], "browser_frame");
    }

    #[test]
    fn test_chat_event_browser_toolbar_action_serde() {
        let event = ChatEvent::BrowserToolbarAction {
            action: "screenshot".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "browser_toolbar_action");
        assert_eq!(json["action"], "screenshot");
        let parsed: ChatEvent = serde_json::from_value(json).unwrap();
        match parsed {
            ChatEvent::BrowserToolbarAction { action } => {
                assert_eq!(action, "screenshot");
            }
            _ => panic!("Expected BrowserToolbarAction"),
        }
    }

    #[test]
    fn test_chat_event_browser_toolbar_action_in_envelope() {
        let envelope = EventEnvelope {
            chat_id: "chat-1".to_string(),
            seq: 10,
            event: ChatEvent::BrowserToolbarAction {
                action: "summarize".to_string(),
            },
        };
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["chat_id"], "chat-1");
        assert_eq!(json["seq"], "10");
        assert_eq!(json["type"], "browser_toolbar_action");
        assert_eq!(json["action"], "summarize");
    }
}
