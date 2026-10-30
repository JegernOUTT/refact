use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex as AMutex, mpsc};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::caps::{resolve_chat_model, resolve_model};
use crate::tools::tools_description::ToolDesc;
use crate::tools::tools_list::get_available_tools;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMeta, ChatToolCall, SamplingParameters, ChatMessage, ChatUsage,
    ReasoningEffort, ChatModelType, SubchatParameters, ContextFile,
};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::chat::prepare::{prepare_chat_passthrough, ChatPrepareOptions};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::llm::params::CacheControl;
use crate::chat::stream_core::{
    run_llm_stream, StreamRunParams, ChoiceFinal, StreamCollector, normalize_tool_call,
    LlmStreamError, clear_unbound_openai_codex_websocket_session,
};
use crate::chat::diagnostics::{make_ui_only_error_message, safe_provider_error_diagnostic};
use crate::chat::retry_policy::{
    classify_llm_error_for_retry, retry_delay_for_attempt, sleep_or_abort, MAX_LLM_RETRY_ATTEMPTS,
};
use crate::chat::tools::{execute_tools, resolve_tool_call_aliases, ExecuteToolsOptions};
use crate::chat::types::{TaskMeta, ThreadParams, TrajectoryCommitIntent};
use crate::worktrees::types::WorktreeMeta;
use crate::chat::trajectories::save_trajectory_as_with_intent;
use crate::chat::trajectory_ops::sanitize_messages_for_new_thread;
use crate::stats::event::{canonicalize_mode_for_stats, split_model_provider, LlmCallEvent};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeReference;
use refact_privacy::{Destination, DestinationId, DestinationKind, PrivacyAuditError, PrivacyAudited};

const MAX_CONTEXT_LIMIT_COMPACT_ATTEMPTS: usize = 1;
const MAX_EMPTY_CHOICE_RETRIES: usize = 2;
const EMPTY_CHOICE_ERROR_PREFIX: &str = "subchat produced no visible content";
const PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS: usize = 2_000;
const PARENT_COMPACTION_DIAGNOSTIC_REDACTION_LOOKAHEAD_CHARS: usize = 512;
const PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED: &str = "\n...[truncated]";
const PARTIAL_OUTPUT_STREAM_ERROR: &str =
    "Stream interrupted after partial output and all retry attempts failed.";
const GUARDED_REPORT_INSTRUCTION: &str = "Guarded file contents may be used to complete the task, but do not quote or reproduce them verbatim in the final report.";

fn partial_output_stream_error_message(original: &str) -> String {
    format!(
        "{} Original error: {}",
        PARTIAL_OUTPUT_STREAM_ERROR,
        safe_provider_error_diagnostic(original),
    )
}

fn should_compact_context_limit_error(
    error: &str,
    attempts: usize,
    abort_flag: &Option<Arc<AtomicBool>>,
) -> bool {
    classify_llm_error_for_retry(error).is_context_limit()
        && attempts < MAX_CONTEXT_LIMIT_COMPACT_ATTEMPTS
        && !is_aborted(abort_flag)
}

fn subchat_retries_allowed(config: &SubchatConfig) -> bool {
    config.tool_name != "mode_transition" && !matches!(config.tools, ToolsPolicy::Supervised { .. })
}

fn append_runner_deliveries(
    messages: &mut Vec<ChatMessage>,
    deliveries: Vec<refact_chat_api::PendingDelivery>,
) -> bool {
    let mut wake = false;
    for delivery in deliveries {
        if messages.iter().any(|message| {
            refact_core::chat_types::delivery_id_of_message(message) == Some(delivery.id.as_str())
        }) {
            continue;
        }
        wake |= delivery.wake;
        if delivery.push == refact_chat_api::PushMode::Preempt {
            messages.push(event(
                EventSubkind::CancellationNote,
                "runner",
                json!({"source": delivery.source, "delivery_id": delivery.id}),
                format!("The current response was interrupted by {}. Follow the delivered instructions below.", delivery.source),
            ));
        }
        messages.extend(delivery.stamp_messages());
    }
    wake
}

fn runner_tool_window_closed(messages: &[ChatMessage]) -> bool {
    let Some(index) = messages
        .iter()
        .rposition(|message| message.role == "assistant")
    else {
        return true;
    };
    messages[index].tool_calls.as_ref().map_or(true, |calls| {
        calls.iter().all(|call| {
            messages[index + 1..].iter().any(|message| {
                matches!(
                    message.role.as_str(),
                    "tool" | "diff" | "context_file" | "plain_text"
                ) && message.tool_call_id == call.id
            })
        })
    })
}

async fn import_runner_local_deliveries(
    app: &AppState,
    agent_id: &str,
    chat_id: &str,
    messages: &[ChatMessage],
) {
    let projected = match refact_core::active_context::active_context(messages) {
        Ok(view) => view.messages,
        Err(error) => {
            warn!(%chat_id, %error, "Cannot import deliveries into unavailable active context");
            return;
        }
    };
    if !runner_tool_window_closed(&projected) {
        return;
    }
    let session = app.chat.sessions.read().await.get(chat_id).cloned();
    let Some(session) = session else {
        return;
    };
    let deliveries = session.lock().await.pending_deliveries.clone();
    for mut delivery in deliveries {
        if let Some(tool_id) = delivery.after_tool_call_id.as_deref() {
            let ready = projected.iter().enumerate().any(|(index, message)| {
                message.role == "assistant"
                    && message.tool_calls.as_ref().is_some_and(|calls| {
                        calls.iter().any(|call| {
                            (tool_id.is_empty() || call.id == tool_id)
                                && projected[index + 1..].iter().any(|result| {
                                    result.role == "tool" && result.tool_call_id == call.id
                                })
                        })
                    })
            });
            if !ready {
                continue;
            }
        }
        delivery.after_tool_call_id = None;
        let id = delivery.id.clone();
        match app.agents.enqueue_delivery(agent_id, delivery).await {
            Ok(_) => {
                let _ = session
                    .lock()
                    .await
                    .update_pending_delivery(&id, None, true);
            }
            Err(error) => warn!(%agent_id, %id, %error, "Failed to import runner local delivery"),
        }
    }
}

pub(crate) async fn drain_background_agent_inbox(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    messages: &mut Vec<ChatMessage>,
) {
    if !runner_tool_window_closed(messages) {
        return;
    }
    let (app, agent_id, chat_id, model) = {
        let ccx = ccx.lock().await;
        (
            ccx.app.clone(),
            ccx.background_agent_id.clone(),
            ccx.chat_id.clone(),
            ccx.current_model.clone(),
        )
    };
    let Some(agent_id) = agent_id else {
        return;
    };
    import_runner_local_deliveries(&app, &agent_id, &chat_id, messages).await;
    {
        let session = app.chat.sessions.read().await.get(&chat_id).cloned();
        let mut session = match session.as_ref() {
            Some(session) => Some(session.lock().await),
            None => None,
        };
        // A yielded wait opens C only after every tool result has landed. Keep
        // the boundary through local tick import, and retain it on drain errors.
        let include_when_idle = session
            .as_ref()
            .is_some_and(|session| session.wait_delivery_boundary);
        match crate::agents::delivery::drain_authorized(&app, &agent_id, include_when_idle).await {
            Ok(deliveries) => {
                append_runner_deliveries(messages, deliveries);
                if let Some(session) = session.as_mut() {
                    session.wait_delivery_boundary = false;
                }
            }
            Err(error) => warn!(%agent_id, %error, "Failed to drain runner deliveries"),
        }
    }
    crate::agents::delivery::publish_runner_queue(&app, &agent_id).await;
    for message in app.agents.drain_inbox(&agent_id).await {
        let sender = match message.from.as_str() {
            "user" => "[message from user]".to_string(),
            "parent" => "[message from parent]".to_string(),
            from => format!("[notice from sibling {from}]"),
        };
        messages.push(event(
            EventSubkind::SystemNotice,
            "agents.inbox",
            json!({ "from": message.from, "queued_at": message.queued_at }),
            format!("{sender}\n{}", message.text),
        ));
    }
    // Native steering notes are authorized again at consume time against the
    // chat and model that will actually see them. A note whose grant was
    // revoked, expired, re-epoched, or rebound while queued is dropped here;
    // nothing is ever composed from request text.
    for note in app.agents.drain_native_notes(&agent_id).await {
        match note.consume(&chat_id, &model) {
            Ok(text) => messages.push(event(
                EventSubkind::SystemNotice,
                "agents.native_note",
                json!({
                    "from": "supervisor",
                    "control_id": note.control_id,
                    "note_id": note.note_id,
                    "queued_at": note.queued_at,
                }),
                format!("[steering note from supervisor]\n{text}"),
            )),
            Err(error) => warn!(
                "dropping queued native steering note {} for {}: {}",
                note.control_id, agent_id, error
            ),
        }
    }
}

/// Cooperative pause barrier at a model/tool boundary. Parks while the native
/// owner has a pause requested and returns `Ok(true)` when the wait ended by
/// cancellation, in which case the caller must not dispatch anything else.
/// The messages the caller already holds (the in-flight outcome) are never
/// touched. Agents without a live registry runtime never park.
pub(crate) async fn hold_native_pause(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
) -> bool {
    let (app, agent_id) = {
        let ccx = ccx.lock().await;
        (ccx.app.clone(), ccx.background_agent_id.clone())
    };
    let Some(agent_id) = agent_id else {
        return is_aborted(&config.abort_flag);
    };
    let Some(gate) = app.agents.pause_gate(&agent_id).await else {
        return is_aborted(&config.abort_flag);
    };
    gate.hold(config.abort_flag.as_deref()).await || is_aborted(&config.abort_flag)
}

async fn emit_parent_compaction_diagnostics(
    config: &SubchatConfig,
    error: &str,
    attempt: usize,
    compacted: bool,
) {
    let (Some(parent_tx), Some(tool_call_id)) = (
        config.parent_subchat_tx.as_ref(),
        config.parent_tool_call_id.as_deref(),
    ) else {
        return;
    };
    let status = parent_compaction_diagnostic_status(error, attempt, compacted);
    let msg = json!({
        "tool_call_id": tool_call_id,
        "subchat_id": status,
        "attached_files": []
    });
    let _ = parent_tx.lock().await.send(msg);
}

fn parent_compaction_diagnostic_status(error: &str, attempt: usize, compacted: bool) -> String {
    let prefix = if compacted {
        format!(
            "Context limit error handled by rebuilding active context (attempt {}):\n",
            attempt,
        )
    } else {
        format!(
            "Context limit error could not rebuild active context (attempt {}):\n",
            attempt,
        )
    };
    let remaining = PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS.saturating_sub(prefix.len());
    format!(
        "{}{}",
        prefix,
        redact_and_cap_parent_diagnostic(error, remaining)
    )
}

fn redact_and_cap_parent_diagnostic(error: &str, max_chars: usize) -> String {
    redact_and_cap_context_limit_diagnostic(error, max_chars)
}

fn safe_context_limit_error_for_log(error: &str) -> String {
    redact_and_cap_context_limit_diagnostic(error, PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS)
}

fn redact_and_cap_context_limit_diagnostic(error: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let (window, window_truncated) = bounded_context_limit_diagnostic_window(
        error,
        max_chars,
        PARENT_COMPACTION_DIAGNOSTIC_REDACTION_LOOKAHEAD_CHARS,
    );
    let mut redacted = redact_context_limit_diagnostic_sensitive(window)
        .trim()
        .to_string();
    if window_truncated || error.len() > max_chars {
        redacted = format!(
            "{}{}",
            redacted.trim_end(),
            PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED,
        );
    }
    cap_context_limit_diagnostic(&redacted, max_chars)
}

fn bounded_context_limit_diagnostic_window(
    error: &str,
    max_chars: usize,
    extra_scan_chars: usize,
) -> (&str, bool) {
    let scan_cap = max_chars.saturating_add(extra_scan_chars);
    let window = crate::llm::safe_truncate(error, scan_cap);
    (window, window.len() < error.len())
}

fn redact_context_limit_diagnostic_sensitive(text: &str) -> String {
    static SK_PLACEHOLDER: OnceLock<regex::Regex> = OnceLock::new();
    let redacted = refact_core::string_utils::redact_sensitive(text);
    SK_PLACEHOLDER
        .get_or_init(|| regex::Regex::new(r"sk-[A-Za-z0-9_-]{8,}").unwrap())
        .replace_all(&redacted, "[REDACTED_SK_TOKEN]")
        .into_owned()
}

fn cap_context_limit_diagnostic(redacted: &str, max_chars: usize) -> String {
    if redacted.len() <= max_chars {
        return redacted.to_string();
    }
    if max_chars <= PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED.len() {
        return crate::llm::safe_truncate(PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED, max_chars)
            .to_string();
    }
    let keep = max_chars - PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED.len();
    format!(
        "{}{}",
        crate::llm::safe_truncate(&redacted, keep).trim_end(),
        PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED,
    )
}

fn append_reactive_compaction_diagnostic(
    messages: &mut Vec<ChatMessage>,
    error: &str,
    preserve_last_message: bool,
) {
    if preserve_last_message {
        if let Some(last) = messages.pop() {
            messages.push(make_ui_only_error_message(error));
            messages.push(last);
            return;
        }
    }
    messages.push(make_ui_only_error_message(error));
}

async fn apply_subchat_reactive_compaction(
    gcx: Arc<GlobalContext>,
    config: &SubchatConfig,
    messages: &mut Vec<ChatMessage>,
    error: &str,
    attempt: usize,
    preserve_last_message: bool,
) -> bool {
    if !subchat_retries_allowed(config) {
        return false;
    }
    let input = match refact_core::active_context::active_context(messages) {
        Ok(view) => view.messages,
        Err(_) => return false,
    };
    let owner = config.trace_parent.trace_folder_owner();
    let outcome = Box::pin(crate::agentic::mode_transition::reconstruct_context(
        gcx,
        crate::agentic::mode_transition::ReconstructionRequest {
            messages: &input,
            target_mode: &config.mode,
            target_mode_description: "Continue this subchat with rebuilt context",
            parent_chat_id: owner.as_deref(),
            model_override: Some(config.model.clone()),
            chat_model: None,
            abort_flag: config.abort_flag.clone(),
            hints: None,
            target_budget_symbols: Some(config.n_ctx.saturating_mul(2)),
            preserve_goal_messages: true,
            live_agents_scope: config.chat_id.as_deref(),
        },
    ))
    .await;
    let compacted = match outcome {
        Ok(outcome) => match refact_core::active_context::make_reconstruction_report(
            outcome.messages,
            refact_core::active_context::ReconstructionMetadata {
                model: Some(outcome.model),
                trigger: Some("subchat_context_limit".into()),
                from_mode: Some(config.mode.clone()),
                to_mode: Some(config.mode.clone()),
                ..Default::default()
            },
        ) {
            Ok(report) => {
                messages.push(report);
                true
            }
            Err(_) => false,
        },
        Err(failure) => {
            warn!(
                "Subchat context rebuild failed: {}",
                safe_provider_error_diagnostic(&failure)
            );
            false
        }
    };
    if !compacted {
        append_reactive_compaction_diagnostic(messages, error, preserve_last_message);
    }

    emit_parent_compaction_diagnostics(config, error, attempt, compacted).await;
    compacted
}

fn get_context_files_from_messages(messages: &[ChatMessage]) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for msg in messages {
        if msg.role == "context_file" {
            match &msg.content {
                ChatContent::ContextFiles(files) => {
                    for file in files {
                        if seen.insert(file.file_name.clone()) {
                            paths.push(file.file_name.clone());
                        }
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
                        for file in files {
                            if seen.insert(file.file_name.clone()) {
                                paths.push(file.file_name.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    paths
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ToolsPolicy {
    All,
    None,
    Only(Vec<String>),
    /// Constructed only by the native authority-guarded spawn path.
    Supervised {
        tools: Vec<String>,
        budget: crate::chat::types::GoalBudget,
    },
}

impl ToolsPolicy {
    pub fn from_option(opt: Option<Vec<String>>) -> Self {
        match opt {
            None => ToolsPolicy::All,
            Some(v) if v.is_empty() => ToolsPolicy::None,
            Some(v) => ToolsPolicy::Only(v),
        }
    }

    fn to_subset_for_llm(&self) -> Option<Vec<String>> {
        match self {
            ToolsPolicy::All => None,
            ToolsPolicy::None => Some(vec![]),
            ToolsPolicy::Only(v) => Some(v.clone()),
            ToolsPolicy::Supervised { tools, .. } => Some(tools.clone()),
        }
    }

    pub(crate) fn allows_tool(&self, tool_name: &str) -> bool {
        match self {
            ToolsPolicy::All => true,
            ToolsPolicy::None => false,
            ToolsPolicy::Only(v) => v.iter().any(|t| t == tool_name),
            ToolsPolicy::Supervised { tools, .. } => tools.iter().any(|t| t == tool_name),
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WrapUpConfig {
    pub depth: usize,
    pub tokens_cnt: usize,
    pub prompt: String,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TraceParent {
    pub chat_id: Option<String>,
    pub root_chat_id: Option<String>,
}

impl TraceParent {
    pub fn unattributed() -> Self {
        Self::default()
    }

    pub fn from_parts(chat_id: Option<&str>, root_chat_id: Option<&str>) -> Self {
        let normalize = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Self {
            chat_id: normalize(chat_id),
            root_chat_id: normalize(root_chat_id),
        }
    }

    pub fn chat(chat_id: &str) -> Self {
        Self::from_parts(Some(chat_id), None)
    }

    pub fn rooted(chat_id: &str, root_chat_id: &str) -> Self {
        Self::from_parts(Some(chat_id), Some(root_chat_id))
    }

    pub fn trace_folder_owner(&self) -> Option<String> {
        self.root_chat_id.clone().or_else(|| self.chat_id.clone())
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SubchatConfig {
    pub tool_name: String,
    pub history_source_model: Option<String>,
    pub stateful: bool,
    pub autonomous_no_confirm: bool,
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
    pub chat_id: Option<String>,
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub link_type: Option<String>,
    pub root_chat_id: Option<String>,
    pub tools: ToolsPolicy,
    pub max_steps: usize,
    pub prepend_system_prompt: bool,
    pub wrap_up: Option<WrapUpConfig>,
    pub task_meta: Option<TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub model: String,
    pub mode: String,
    pub n_ctx: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub implicit_context_window: bool,
    pub max_new_tokens: usize,
    pub temperature: Option<f32>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub cache_control: CacheControl,
    pub parent_tool_call_id: Option<String>,
    #[serde(skip)]
    pub parent_subchat_tx: Option<Arc<AMutex<mpsc::UnboundedSender<Value>>>>,
    #[serde(skip)]
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub soft_abort: bool,
    pub activity_stamp: Option<Arc<AtomicU64>>,
    pub background_agent_id: Option<String>,
    pub subchat_depth: usize,
    pub final_step_force_answer: bool,
    pub buddy_meta: Option<crate::buddy::types::BuddyThreadMeta>,
    #[serde(skip)]
    pub step_progress: Option<Arc<dyn Fn(SubchatProgress) + Send + Sync>>,
    pub trace_parent: TraceParent,
}

impl SubchatConfig {
    pub fn context_tokens_cap(&self) -> Option<usize> {
        (!self.implicit_context_window).then_some(self.n_ctx)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubchatProgress {
    Step(usize),
    ToolStarted {
        name: String,
        arg_preview: Option<String>,
    },
    ToolsFinished,
    Usage {
        tokens_delta: u64,
        cost_delta: Option<f64>,
    },
}

struct SubchatProgressCollector {
    sender: Option<mpsc::UnboundedSender<Value>>,
    tool_call_id: Option<String>,
    activity_stamp: Option<Arc<AtomicU64>>,
    thinking_tail: String,
    reasoning_tail: String,
    content_tail: String,
    last_sent: String,
    last_sent_at: std::time::Instant,
}

impl SubchatProgressCollector {
    fn new(
        sender: Option<mpsc::UnboundedSender<Value>>,
        tool_call_id: Option<String>,
        activity_stamp: Option<Arc<AtomicU64>>,
    ) -> Self {
        Self {
            sender,
            tool_call_id,
            activity_stamp,
            thinking_tail: String::new(),
            reasoning_tail: String::new(),
            content_tail: String::new(),
            last_sent: String::new(),
            last_sent_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(60))
                .unwrap_or_else(std::time::Instant::now),
        }
    }

    fn append_tail(buf: &mut String, text: &str, max_chars: usize) {
        if text.is_empty() {
            return;
        }
        buf.push_str(text);
        if buf.len() > max_chars {
            let mut start = buf.len().saturating_sub(max_chars);
            while start < buf.len() && !buf.is_char_boundary(start) {
                start += 1;
            }
            buf.drain(..start);
        }
    }

    fn extract_thinking_preview(blocks: &[serde_json::Value]) -> Option<String> {
        for block in blocks.iter().rev() {
            let Some(obj) = block.as_object() else {
                continue;
            };
            for key in ["thinking", "text", "content"] {
                if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                    if !s.trim().is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
        None
    }

    fn normalize_preview(text: &str) -> String {
        // Preserve newlines for markdown rendering, just normalize CRLF/CR.
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    fn maybe_send_update(&mut self) {
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        let Some(tool_call_id) = self.tool_call_id.as_ref() else {
            return;
        };

        let raw = if !self.thinking_tail.trim().is_empty() {
            &self.thinking_tail
        } else if !self.reasoning_tail.trim().is_empty() {
            &self.reasoning_tail
        } else {
            &self.content_tail
        };

        let mut progress = Self::normalize_preview(raw);
        if progress.is_empty() {
            return;
        }

        // UI renders markdown up to ~50k chars; keep progress within that.
        const MAX_CHARS: usize = 50_000;
        let truncated = crate::llm::safe_truncate(&progress, MAX_CHARS);
        if truncated.len() != progress.len() {
            progress = format!("{}…", truncated);
        }

        if self.last_sent == progress {
            return;
        }

        let now = std::time::Instant::now();
        if now.duration_since(self.last_sent_at) < std::time::Duration::from_millis(750)
            && !self.last_sent.is_empty()
        {
            return;
        }

        let msg = json!({
            "tool_call_id": tool_call_id,
            "subchat_id": progress,
        });
        let _ = sender.send(msg);

        self.last_sent = progress;
        self.last_sent_at = now;
    }
}

impl StreamCollector for SubchatProgressCollector {
    fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<crate::chat::types::DeltaOp>) {
        stamp_activity(&self.activity_stamp);
        for op in ops {
            match op {
                crate::chat::types::DeltaOp::AppendReasoning { text } => {
                    if self.thinking_tail.trim().is_empty() {
                        Self::append_tail(&mut self.reasoning_tail, &text, 50_000);
                    }
                }
                crate::chat::types::DeltaOp::SetReasoning { text } => {
                    if self.thinking_tail.trim().is_empty() {
                        self.reasoning_tail.clear();
                        Self::append_tail(&mut self.reasoning_tail, &text, 50_000);
                    }
                }
                crate::chat::types::DeltaOp::AppendContent { text } => {
                    if self.thinking_tail.trim().is_empty() && self.reasoning_tail.trim().is_empty()
                    {
                        Self::append_tail(&mut self.content_tail, &text, 50_000);
                    }
                }
                crate::chat::types::DeltaOp::SetThinkingBlocks { blocks } => {
                    if let Some(preview) = Self::extract_thinking_preview(&blocks) {
                        self.thinking_tail = preview;
                    }
                }
                _ => {}
            }
        }

        self.maybe_send_update();
    }

    fn on_usage(&mut self, _usage: &ChatUsage) {}

    fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
}

pub struct SubchatResult {
    pub messages: Vec<ChatMessage>,
    /// Reserved for provider-local usage metadata returned by nested agent calls.
    pub metering: serde_json::Map<String, serde_json::Value>,
    /// Set when `config.stateful == true`, allows caller to reference the saved trajectory.
    /// Intentionally public API - callers may use it for trajectory linking.
    #[allow(dead_code)]
    pub chat_id: Option<String>,
    pub aborted: bool,
}

struct AuditedSubchatReport(Vec<(usize, refact_privacy::FileRecord)>);

impl PrivacyAudited for AuditedSubchatReport {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, PrivacyAuditError> {
        Ok(self.0.clone())
    }
}

fn unique_privacy_records(
    messages: &[ChatMessage],
) -> Result<Vec<(usize, refact_privacy::FileRecord)>, PrivacyAuditError> {
    refact_privacy::records_from_messages(messages).map(|indexed| {
        indexed
            .into_iter()
            .fold(Vec::new(), |mut records, (message_index, record)| {
                if !records.iter().any(|(_, existing)| existing == &record) {
                    records.push((message_index, record));
                }
                records
            })
    })
}

fn subagent_destination(model_id: &str) -> Destination {
    let id = model_id
        .split_once('/')
        .map_or(model_id, |(provider, _)| provider);
    Destination {
        id: DestinationId(id.to_string()),
        kind: DestinationKind::SubagentModel,
        display_name: model_id.to_string(),
    }
}

fn gate_subchat_boundary(
    gcx: &Arc<GlobalContext>,
    messages: &[ChatMessage],
    model_id: &str,
) -> Result<(), String> {
    let records = unique_privacy_records(messages).map_err(|error| error.to_string())?;
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let compiled = policy.compile().map_err(|error| error.to_string())?;
    match refact_privacy::clear(
        AuditedSubchatReport(records),
        &subagent_destination(model_id),
        &compiled,
    ) {
        Ok(_) => Ok(()),
        Err(refusal) if refusal.offending.is_empty() => Err(refusal.message),
        Err(refusal) => Err(refusal.model_facing().to_string()),
    }
}

fn prepare_subchat_messages(
    gcx: &Arc<GlobalContext>,
    messages: Vec<ChatMessage>,
    model_id: &str,
) -> Result<Vec<ChatMessage>, String> {
    let messages = refact_core::active_context::active_context(&messages)
        .map_err(|e| e.to_string())?
        .messages;
    gate_subchat_boundary(gcx, &messages, model_id)?;
    if refact_privacy::records_from_messages(&messages)
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Ok(messages);
    }

    let mut prepared = Vec::with_capacity(messages.len() + 1);
    prepared.push(ChatMessage::new(
        "system".to_string(),
        GUARDED_REPORT_INSTRUCTION.to_string(),
    ));
    prepared.extend(messages);
    Ok(prepared)
}

fn apply_subchat_report_policy(
    policy: &refact_privacy::SubagentPolicy,
    messages: &mut [ChatMessage],
) -> Result<(), PrivacyAuditError> {
    let Some(report_index) = messages
        .iter()
        .rposition(|message| message.role == "assistant")
    else {
        return Ok(());
    };
    let records = unique_privacy_records(messages)?;
    messages[report_index].extra.remove("privacy");
    if policy.report_declassifies {
        return Ok(());
    }

    crate::privacy::records::merge_records(
        &mut messages[report_index],
        records.into_iter().map(|(_, record)| record),
    );
    Ok(())
}

pub fn apply_launch_context_window(
    config: &mut SubchatConfig,
    model_n_ctx: usize,
    explicit_cap: Option<usize>,
    implicit: bool,
) -> Result<(), String> {
    match explicit_cap {
        Some(0) => return Err("context_tokens_cap must be > 0".into()),
        Some(cap) if model_n_ctx > 0 && cap > model_n_ctx => {
            return Err(format!(
                "context_tokens_cap ({cap}) exceeds model '{}' n_ctx ({model_n_ctx})",
                config.model
            ))
        }
        Some(cap) => {
            config.n_ctx = cap;
            config.implicit_context_window = false;
        }
        None if implicit => {
            if model_n_ctx == 0 {
                return Err(format!(
                    "model '{}' has no known context window",
                    config.model
                ));
            }
            config.n_ctx = model_n_ctx;
            config.implicit_context_window = true;
        }
        None => return Ok(()),
    }
    if config.n_ctx > 1 {
        config.max_new_tokens = config.max_new_tokens.min(config.n_ctx - 1);
    }
    Ok(())
}

fn scale_subchat_budget(value: usize, new_n_ctx: usize, old_n_ctx: usize) -> usize {
    if value == 0 || old_n_ctx == 0 || new_n_ctx >= old_n_ctx {
        return value;
    }

    (((value as u128) * (new_n_ctx as u128)) / (old_n_ctx as u128)) as usize
}

fn parse_subchat_cache_control(
    tool_name: &str,
    value: Option<&str>,
) -> Result<CacheControl, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(CacheControl::Ephemeral),
        Some(value) if value.eq_ignore_ascii_case("off") => Ok(CacheControl::Off),
        Some(value) if value.eq_ignore_ascii_case("ephemeral") => Ok(CacheControl::Ephemeral),
        Some(value) => Err(format!(
            "invalid cache_control '{}' for '{}', expected: off, ephemeral",
            value, tool_name
        )),
    }
}

fn normalize_subchat_params_for_model(
    tool_name: &str,
    params: &mut SubchatParameters,
    model_rec: &crate::caps::ChatModelRecord,
) {
    let requested_n_ctx = params.subchat_n_ctx;
    let requested_max_new_tokens = params.subchat_max_new_tokens;
    let requested_tokens_for_rag = params.subchat_tokens_for_rag;

    if model_rec.base.n_ctx > 0 && params.subchat_n_ctx > model_rec.base.n_ctx {
        params.subchat_n_ctx = model_rec.base.n_ctx;

        if requested_tokens_for_rag > 0 {
            params.subchat_max_new_tokens = scale_subchat_budget(
                requested_max_new_tokens,
                params.subchat_n_ctx,
                requested_n_ctx,
            )
            .max(1);
            params.subchat_tokens_for_rag = scale_subchat_budget(
                requested_tokens_for_rag,
                params.subchat_n_ctx,
                requested_n_ctx,
            );
        }

        info!(
            "normalized subchat '{}' budget for model '{}' from n_ctx={} to n_ctx={}, max_new_tokens={}, tokens_for_rag={}",
            tool_name,
            model_rec.base.id,
            requested_n_ctx,
            params.subchat_n_ctx,
            params.subchat_max_new_tokens,
            params.subchat_tokens_for_rag,
        );
    }

    if let Some(max_output_tokens) = model_rec.max_output_tokens.filter(|v| *v > 0) {
        if params.subchat_max_new_tokens > max_output_tokens {
            params.subchat_max_new_tokens = max_output_tokens;
        }
    }

    if params.subchat_n_ctx > 1 {
        params.subchat_max_new_tokens = params.subchat_max_new_tokens.min(params.subchat_n_ctx - 1);
    }

    let available_for_rag = params
        .subchat_n_ctx
        .saturating_sub(params.subchat_max_new_tokens)
        .saturating_sub(1);
    if params.subchat_tokens_for_rag > available_for_rag {
        params.subchat_tokens_for_rag = available_for_rag;
    }
}

pub async fn resolve_subchat_params(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
) -> Result<SubchatParameters, String> {
    let mut params = load_subchat_params(gcx.clone(), tool_name).await?;
    let model = resolve_subchat_model_for_tool(gcx.clone(), tool_name, &params).await?;
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;
    let model_rec = resolve_chat_model(caps, &model)?;
    normalize_subchat_params_for_model(tool_name, &mut params, &model_rec);

    Ok(params)
}

/// Subagent parameters as configured, before any model is resolved or normalized against.
pub async fn load_subchat_params(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
) -> Result<SubchatParameters, String> {
    use crate::yaml_configs::customization_registry::get_subagent_config;

    let subagent_config = get_subagent_config(gcx.clone(), tool_name, None)
        .await
        .ok_or_else(|| {
            format!(
                "subchat params for '{}' not found in subagents registry",
                tool_name
            )
        })?;

    let subchat = &subagent_config.subchat;

    let model_type = match subchat.model_type.as_deref() {
        Some(mt) if mt.eq_ignore_ascii_case("light") => ChatModelType::Light,
        Some(mt) if mt.eq_ignore_ascii_case("thinking") => ChatModelType::Thinking,
        Some(mt) if mt.eq_ignore_ascii_case("default") => ChatModelType::Default,
        Some(mt) if mt.eq_ignore_ascii_case("buddy") => ChatModelType::Buddy,
        Some(mt) => {
            return Err(format!(
                "invalid model_type '{}' for '{}', expected: light, default, thinking, buddy",
                mt, tool_name
            ))
        }
        None => ChatModelType::Default,
    };

    let reasoning_effort = match subchat.reasoning_effort.as_deref() {
        Some(re) => match ReasoningEffort::from_str_opt(re) {
            Some(effort) => Some(effort),
            None => return Err(format!(
                "invalid reasoning_effort '{}' for '{}', expected: none, minimal, low, medium, high, xhigh, max",
                re, tool_name
            )),
        },
        None => None,
    };
    let cache_control = parse_subchat_cache_control(tool_name, subchat.cache_control.as_deref())?;

    let params = SubchatParameters {
        subchat_model_type: model_type,
        subchat_model: subchat.model.clone().unwrap_or_default(),
        subchat_n_ctx: subchat.n_ctx.unwrap_or(0),
        subchat_max_new_tokens: subchat.max_new_tokens.unwrap_or(0),
        subchat_temperature: subchat.temperature,
        subchat_tokens_for_rag: subchat.tokens_for_rag.unwrap_or(0),
        subchat_reasoning_effort: reasoning_effort,
        subchat_cache_control: cache_control,
    };

    if params.subchat_n_ctx == 0 {
        return Err(format!(
            "subchat_n_ctx must be > 0 for tool '{}'",
            tool_name
        ));
    }
    if params.subchat_max_new_tokens == 0 {
        return Err(format!(
            "subchat_max_new_tokens must be > 0 for tool '{}'",
            tool_name
        ));
    }

    Ok(params)
}

pub async fn resolve_subchat_model(
    gcx: Arc<GlobalContext>,
    params: &SubchatParameters,
) -> Result<String, String> {
    resolve_subchat_model_inner(gcx, params, None).await
}

async fn resolve_subchat_model_for_tool(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
    params: &SubchatParameters,
) -> Result<String, String> {
    resolve_subchat_model_inner(gcx, params, Some(tool_name)).await
}

async fn resolve_subchat_model_inner(
    gcx: Arc<GlobalContext>,
    params: &SubchatParameters,
    tool_name: Option<&str>,
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;

    if !params.subchat_model.is_empty() {
        let model_rec = resolve_chat_model(caps, &params.subchat_model).map_err(|e| {
            subchat_explicit_model_error(tool_name, &params.subchat_model, "is not available", &e)
        })?;
        if let Some(reason) = llm_endpoint_unusable_reason(&model_rec.base.endpoint) {
            return Err(subchat_explicit_model_error(
                tool_name,
                &params.subchat_model,
                "is misconfigured",
                &reason,
            ));
        }
        return Ok(model_rec.base.id.clone());
    }

    let model_id = match params.subchat_model_type {
        ChatModelType::Light => &caps.defaults.chat_light_model,
        ChatModelType::Default => &caps.defaults.chat_default_model,
        ChatModelType::Thinking => &caps.defaults.chat_thinking_model,
        ChatModelType::Buddy => &caps.defaults.chat_buddy_model,
    };
    let model_label = subchat_model_type_label(params.subchat_model_type);

    if model_id.trim().is_empty() {
        return Err(format!(
            "{} is not set up. Go to Default model settings and configure {}.",
            subchat_model_requirement_label(tool_name, params.subchat_model_type),
            model_label
        ));
    }

    let model_rec = resolve_model(&caps.chat_models, model_id).map_err(|e| {
        format!(
            "{} '{}' is not available: {}. Go to Default model settings and configure {}.",
            subchat_model_requirement_label(tool_name, params.subchat_model_type),
            model_id,
            e,
            model_label
        )
    })?;

    if let Some(reason) = llm_endpoint_unusable_reason(&model_rec.base.endpoint) {
        return Err(format!(
            "{} '{}' is misconfigured: {}. Go to Default model settings and configure {}.",
            subchat_model_requirement_label(tool_name, params.subchat_model_type),
            model_id,
            reason,
            model_label
        ));
    }

    Ok(model_rec.base.id.clone())
}

fn subchat_explicit_model_error(
    tool_name: Option<&str>,
    model: &str,
    state: &str,
    reason: &str,
) -> String {
    match tool_name {
        Some(tool_name) => format!(
            "Subagent '{}' is pinned to model '{}', but it {}: {}. Go to Default model settings and configure this model or update the subagent config.",
            tool_name, model, state, reason
        ),
        None => format!(
            "Subchat model '{}' {}: {}. Go to Default model settings and configure this model or update the subagent config.",
            model, state, reason
        ),
    }
}

fn subchat_model_requirement_label(tool_name: Option<&str>, model_type: ChatModelType) -> String {
    let model_label = subchat_model_type_label(model_type);
    match tool_name {
        Some(tool_name) => format!(
            "{} required by subagent '{}' (model_type: {})",
            model_label,
            tool_name,
            subchat_model_type_config_value(model_type)
        ),
        None => model_label.to_string(),
    }
}

fn subchat_model_type_config_value(model_type: ChatModelType) -> &'static str {
    match model_type {
        ChatModelType::Light => "light",
        ChatModelType::Default => "default",
        ChatModelType::Thinking => "thinking",
        ChatModelType::Buddy => "buddy",
    }
}

fn subchat_model_type_label(model_type: ChatModelType) -> &'static str {
    match model_type {
        ChatModelType::Light => "Light model",
        ChatModelType::Default => "Default model",
        ChatModelType::Thinking => "Thinking model",
        ChatModelType::Buddy => "Buddy model",
    }
}

fn llm_endpoint_unusable_reason(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Some("an empty LLM endpoint URL".to_string());
    }
    match url::Url::parse(endpoint) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => None,
        Ok(url) => Some(format!(
            "an unsupported LLM endpoint URL scheme '{}': {}",
            url.scheme(),
            endpoint
        )),
        Err(e) => Some(format!("an invalid LLM endpoint URL '{}': {}", endpoint, e)),
    }
}

pub async fn resolve_subchat_config(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
    stateful: bool,
    chat_id: Option<String>,
    title: Option<String>,
    parent_id: Option<String>,
    link_type: Option<String>,
    root_chat_id: Option<String>,
    tools: Option<Vec<String>>,
    max_steps: usize,
    prepend_system_prompt: bool,
    wrap_up: Option<WrapUpConfig>,
    mode: String,
) -> Result<SubchatConfig, String> {
    resolve_subchat_config_with_parent(
        gcx,
        tool_name,
        stateful,
        chat_id,
        title,
        parent_id,
        link_type,
        root_chat_id,
        tools,
        max_steps,
        prepend_system_prompt,
        wrap_up,
        mode,
        None,
        None,
        None,
        None,
        None,
        0,
    )
    .await
}

async fn parent_thread_worktree(gcx: Arc<GlobalContext>, parent_id: &str) -> Option<WorktreeMeta> {
    let sessions = { gcx.chat_sessions.clone() };
    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(parent_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        return session_arc.lock().await.thread.worktree.clone();
    }

    crate::chat::trajectories::validate_trajectory_id(parent_id).ok()?;
    crate::chat::trajectories::load_trajectory_for_chat(gcx, parent_id)
        .await
        .and_then(|loaded| loaded.thread.worktree)
}

async fn resolve_subchat_worktree(
    gcx: Arc<GlobalContext>,
    parent_id: Option<&str>,
    parent_worktree: Option<WorktreeMeta>,
) -> Option<WorktreeMeta> {
    match parent_worktree {
        Some(parent_worktree) => Some(parent_worktree),
        None => match parent_id {
            Some(parent_id) => parent_thread_worktree(gcx, parent_id).await,
            None => None,
        },
    }
}

fn worktree_reference_for_thread(
    chat_id: &str,
    thread: &ThreadParams,
) -> Option<WorktreeReference> {
    let worktree = thread.worktree.as_ref()?;
    let task_meta = thread.task_meta.as_ref();
    Some(WorktreeReference {
        kind: worktree.kind.clone(),
        chat_id: Some(chat_id.to_string()),
        task_id: task_meta.map(|meta| meta.task_id.clone()),
        card_id: task_meta.and_then(|meta| meta.card_id.clone()),
        agent_id: task_meta.and_then(|meta| meta.agent_id.clone()),
    })
}

async fn register_stateful_subchat_worktree(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    thread: &ThreadParams,
) -> Result<(), String> {
    let Some(worktree) = thread.worktree.clone() else {
        return Ok(());
    };
    let Some(reference) = worktree_reference_for_thread(chat_id, thread) else {
        return Ok(());
    };

    let cache_dir = { gcx.cache_dir.clone() };
    let service =
        match WorktreeService::new_async(cache_dir, worktree.source_workspace_root.clone()).await {
            Ok(service) => service,
            Err(e) => {
                warn!(
                    "Failed to resolve worktree service while registering subchat '{}': {}",
                    chat_id, e
                );
                return Err(e);
            }
        };

    if service.validate_worktree_meta_strict(&worktree).await? != worktree {
        return Err("Runner checkout identity changed; retain current scope".into());
    }
    match service.add_reference(&worktree.id, reference).await {
        Ok(view) if view.meta == worktree => Ok(()),
        Ok(_) => Err("Runner checkout identity changed while registering reference".into()),
        Err(e) => Err(e),
    }
}

pub async fn resolve_subchat_config_with_parent(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
    stateful: bool,
    chat_id: Option<String>,
    title: Option<String>,
    parent_id: Option<String>,
    link_type: Option<String>,
    root_chat_id: Option<String>,
    tools: Option<Vec<String>>,
    max_steps: usize,
    prepend_system_prompt: bool,
    wrap_up: Option<WrapUpConfig>,
    mode: String,
    task_meta: Option<TaskMeta>,
    worktree: Option<WorktreeMeta>,
    parent_tool_call_id: Option<String>,
    parent_subchat_tx: Option<Arc<AMutex<mpsc::UnboundedSender<Value>>>>,
    abort_flag: Option<Arc<AtomicBool>>,
    subchat_depth: usize,
) -> Result<SubchatConfig, String> {
    let max_depth = crate::runtime_settings::current()
        .coordination
        .max_subchat_depth;
    if max_steps == 0 {
        return Err("max_steps must be > 0".to_string());
    }
    if subchat_depth >= max_depth {
        return Err(format!("subchat depth limit ({max_depth}) exceeded"));
    }

    let params = resolve_subchat_params(gcx.clone(), tool_name).await?;
    let model = resolve_subchat_model_for_tool(gcx.clone(), tool_name, &params).await?;
    let cache_control = params.subchat_cache_control;
    let (autonomous_no_confirm, auto_approve_editing_tools, auto_approve_dangerous_commands) =
        resolve_subagent_confirmation_defaults(gcx.clone(), tool_name).await;

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;

    let model_rec = resolve_chat_model(caps, &model)?;
    if params.subchat_n_ctx > model_rec.base.n_ctx && model_rec.base.n_ctx > 0 {
        return Err(format!(
            "subchat_n_ctx ({}) exceeds model '{}' n_ctx ({})",
            params.subchat_n_ctx, model, model_rec.base.n_ctx
        ));
    }

    let worktree = resolve_subchat_worktree(gcx.clone(), parent_id.as_deref(), worktree).await;

    if let Some(requested_tools) = tools.as_ref().filter(|list| !list.is_empty()) {
        let known_names: std::collections::HashSet<String> = get_available_tools(gcx.clone())
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect();
        let unknown: Vec<&String> = requested_tools
            .iter()
            .filter(|name| !known_names.contains(*name))
            .collect();
        if !unknown.is_empty() {
            warn!(
                "subchat '{}' requested tools not present in the registry (check config tool names): {:?}",
                tool_name, unknown,
            );
        }
    }

    let trace_parent = TraceParent::from_parts(parent_id.as_deref(), root_chat_id.as_deref());

    Ok(SubchatConfig {
        history_source_model: None,
        tool_name: tool_name.to_string(),
        stateful,
        autonomous_no_confirm,
        auto_approve_editing_tools,
        auto_approve_dangerous_commands,
        chat_id,
        title,
        parent_id,
        link_type,
        root_chat_id,
        tools: ToolsPolicy::from_option(tools),
        max_steps,
        prepend_system_prompt,
        wrap_up,
        task_meta,
        worktree,
        model,
        mode,
        n_ctx: params.subchat_n_ctx,
        implicit_context_window: false,
        max_new_tokens: params.subchat_max_new_tokens,
        temperature: params.subchat_temperature,
        reasoning_effort: params.subchat_reasoning_effort,
        cache_control,
        parent_tool_call_id,
        parent_subchat_tx,
        abort_flag,
        soft_abort: false,
        activity_stamp: None,
        background_agent_id: None,
        subchat_depth,
        final_step_force_answer: false,
        buddy_meta: None,
        step_progress: None,
        trace_parent,
    })
}

#[derive(Debug, Clone)]
pub struct ExplicitSubchatSpec {
    pub params: SubchatParameters,
    pub model: String,
    pub autonomous_no_confirm: bool,
}

pub async fn resolve_subchat_config_with_explicit_params(
    gcx: Arc<GlobalContext>,
    attribution_id: &str,
    spec: &ExplicitSubchatSpec,
    stateful: bool,
    chat_id: Option<String>,
    title: Option<String>,
    parent_id: Option<String>,
    link_type: Option<String>,
    root_chat_id: Option<String>,
    tools: Option<Vec<String>>,
    max_steps: usize,
    prepend_system_prompt: bool,
    mode: String,
    task_meta: Option<TaskMeta>,
    worktree: Option<WorktreeMeta>,
    parent_tool_call_id: Option<String>,
    parent_subchat_tx: Option<Arc<AMutex<mpsc::UnboundedSender<Value>>>>,
    abort_flag: Option<Arc<AtomicBool>>,
    subchat_depth: usize,
) -> Result<SubchatConfig, String> {
    let max_depth = crate::runtime_settings::current()
        .coordination
        .max_subchat_depth;
    if max_steps == 0 {
        return Err("max_steps must be > 0".to_string());
    }
    if subchat_depth >= max_depth {
        return Err(format!("subchat depth limit ({max_depth}) exceeded"));
    }
    if spec.model.trim().is_empty() {
        return Err(format!(
            "explicit subchat '{}' requires a resolved model id",
            attribution_id
        ));
    }
    if spec.params.subchat_n_ctx == 0 {
        return Err(format!(
            "subchat_n_ctx must be > 0 for '{}'",
            attribution_id
        ));
    }
    if spec.params.subchat_max_new_tokens == 0 {
        return Err(format!(
            "subchat_max_new_tokens must be > 0 for '{}'",
            attribution_id
        ));
    }

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;
    let model_rec = resolve_chat_model(caps, &spec.model)?;
    if let Some(reason) = llm_endpoint_unusable_reason(&model_rec.base.endpoint) {
        return Err(format!(
            "model '{}' for '{}' is misconfigured: {}",
            spec.model, attribution_id, reason
        ));
    }
    let mut params = spec.params.clone();
    normalize_subchat_params_for_model(attribution_id, &mut params, &model_rec);

    let worktree = resolve_subchat_worktree(gcx.clone(), parent_id.as_deref(), worktree).await;

    if let Some(requested_tools) = tools.as_ref().filter(|list| !list.is_empty()) {
        let known_names: std::collections::HashSet<String> = get_available_tools(gcx.clone())
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect();
        let unknown: Vec<&String> = requested_tools
            .iter()
            .filter(|name| !known_names.contains(*name))
            .collect();
        if !unknown.is_empty() {
            warn!(
                "subchat '{}' requested tools not present in the registry (check config tool names): {:?}",
                attribution_id, unknown,
            );
        }
    }

    let trace_parent = TraceParent::from_parts(parent_id.as_deref(), root_chat_id.as_deref());

    Ok(SubchatConfig {
        history_source_model: None,
        tool_name: attribution_id.to_string(),
        stateful,
        autonomous_no_confirm: spec.autonomous_no_confirm,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        chat_id,
        title,
        parent_id,
        link_type,
        root_chat_id,
        tools: ToolsPolicy::from_option(tools),
        max_steps,
        prepend_system_prompt,
        wrap_up: None,
        task_meta,
        worktree,
        model: model_rec.base.id.clone(),
        mode,
        n_ctx: params.subchat_n_ctx,
        implicit_context_window: false,
        max_new_tokens: params.subchat_max_new_tokens,
        temperature: params.subchat_temperature,
        reasoning_effort: params.subchat_reasoning_effort,
        cache_control: params.subchat_cache_control,
        parent_tool_call_id,
        parent_subchat_tx,
        abort_flag,
        soft_abort: false,
        activity_stamp: None,
        background_agent_id: None,
        subchat_depth,
        final_step_force_answer: false,
        buddy_meta: None,
        step_progress: None,
        trace_parent,
    })
}

pub async fn run_subchat_once_with_explicit_params(
    gcx: Arc<GlobalContext>,
    attribution_id: &str,
    spec: &ExplicitSubchatSpec,
    messages: Vec<ChatMessage>,
    parent_tool_call_id: String,
    parent_subchat_tx: Arc<AMutex<mpsc::UnboundedSender<Value>>>,
    parent_abort_flag: Arc<AtomicBool>,
    parent_depth: usize,
    parent_task_meta: Option<TaskMeta>,
    parent_worktree: Option<WorktreeMeta>,
    trace_parent: TraceParent,
) -> Result<SubchatResult, String> {
    let mut config = resolve_subchat_config_with_explicit_params(
        gcx.clone(),
        attribution_id,
        spec,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        "agent".to_string(),
        parent_task_meta,
        parent_worktree,
        Some(parent_tool_call_id),
        Some(parent_subchat_tx),
        Some(parent_abort_flag),
        parent_depth + 1,
    )
    .await?;
    config.trace_parent = trace_parent;

    run_subchat(gcx, messages, config).await
}

fn has_final_answer(messages: &[ChatMessage]) -> bool {
    messages
        .last()
        .filter(|m| m.role == "assistant")
        .map(|m| m.tool_calls.as_ref().map_or(true, |tc| tc.is_empty()))
        .unwrap_or(false)
}

fn stateful_thread_from_config(chat_id: &str, config: &SubchatConfig) -> ThreadParams {
    let tool_use = match &config.tools {
        ToolsPolicy::All => "agent".to_string(),
        ToolsPolicy::None => "none".to_string(),
        ToolsPolicy::Only(v) => v.join(","),
        ToolsPolicy::Supervised { tools, .. } => {
            if tools.is_empty() {
                "none".into()
            } else {
                tools.join(",")
            }
        }
    };
    let task_meta = task_meta_for_stateful_subchat(config);

    let mut thread = ThreadParams {
        id: chat_id.to_string(),
        title: config
            .title
            .clone()
            .unwrap_or_else(|| "Subchat".to_string()),
        model: config.model.clone(),
        mode: config.mode.clone(),
        tool_use,
        task_meta,
        worktree: config.worktree.clone(),
        parent_id: config.parent_id.clone(),
        link_type: config.link_type.clone(),
        root_chat_id: config.root_chat_id.clone(),
        autonomous_no_confirm: config.autonomous_no_confirm,
        auto_approve_editing_tools: config.auto_approve_editing_tools,
        auto_approve_dangerous_commands: config.auto_approve_dangerous_commands,
        buddy_meta: config.buddy_meta.clone(),
        context_tokens_cap: config.context_tokens_cap(),
        ..Default::default()
    };
    // SubchatConfig carries the already model-normalized request window.
    thread.resolve_pending_compression_cap(Some(config.n_ctx));
    thread
}

async fn resolve_subagent_confirmation_defaults(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
) -> (bool, bool, bool) {
    crate::yaml_configs::customization_registry::get_subagent_config(gcx, tool_name, None)
        .await
        .map(|config| {
            (
                config.subchat.autonomous_no_confirm.unwrap_or(false),
                config.subchat.auto_approve_editing_tools.unwrap_or(false),
                config
                    .subchat
                    .auto_approve_dangerous_commands
                    .unwrap_or(false),
            )
        })
        .unwrap_or_default()
}

fn is_subagentic_link_type(link_type: &str) -> bool {
    !matches!(link_type, "handoff" | "mode_transition" | "branch")
}

fn trace_thread_from_config(chat_id: &str, config: &SubchatConfig) -> ThreadParams {
    let mut thread = stateful_thread_from_config(chat_id, config);
    if !config.stateful {
        thread.link_type = Some(crate::chat::trajectories::internal_trace_link_type(
            &config.tool_name,
        ));
        thread.root_chat_id = config
            .trace_parent
            .trace_folder_owner()
            .or_else(|| thread.root_chat_id.clone())
            .or_else(|| config.parent_id.clone())
            .or_else(|| Some(crate::chat::trajectories::UNATTRIBUTED_TRACES_DIR.to_string()));
    }
    thread
}

fn should_persist_subchat_trajectory(config: &SubchatConfig) -> bool {
    config.stateful || config.tool_name != "mode_transition"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubchatTrajectoryCommitPhase {
    Seed,
    Progress,
    Failed,
    Final,
}

fn subchat_trajectory_commit_intent(phase: SubchatTrajectoryCommitPhase) -> TrajectoryCommitIntent {
    match phase {
        SubchatTrajectoryCommitPhase::Seed | SubchatTrajectoryCommitPhase::Progress => {
            TrajectoryCommitIntent::Checkpoint
        }
        SubchatTrajectoryCommitPhase::Failed | SubchatTrajectoryCommitPhase::Final => {
            TrajectoryCommitIntent::Required
        }
    }
}

type SubchatProgressMessages = Arc<StdMutex<Vec<ChatMessage>>>;

fn record_subchat_progress(progress: &SubchatProgressMessages, messages: &[ChatMessage]) {
    let mut slot = match progress.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    *slot = messages.to_vec();
}

async fn runner_owned_session(
    app: &AppState,
    config: &SubchatConfig,
    chat_id: &str,
) -> Option<Arc<AMutex<crate::chat::types::ChatSession>>> {
    if !config.stateful {
        return None;
    }
    let session = app.chat.sessions.read().await.get(chat_id).cloned()?;
    let owned = {
        let session = session.lock().await;
        !session.closed && session.is_runner_owned_subagent_view()
    };
    owned.then_some(session)
}

async fn save_through_runner_session(
    app: &AppState,
    config: &SubchatConfig,
    thread: &ThreadParams,
    messages: &[ChatMessage],
    intent: TrajectoryCommitIntent,
) -> Option<Result<(), String>> {
    if !config.stateful {
        return None;
    }
    let session = app.chat.sessions.read().await.get(&thread.id).cloned();
    let Some(session) = session else {
        return Some(Err(
            "Runner session is unavailable; refusing to save a stale checkout".into(),
        ));
    };
    let _transition = match crate::chat::worktree_lifecycle::try_transition_lock(&session) {
        Ok(transition) => transition,
        Err(error) => return Some(Err(error)),
    };
    let (version, source, requests, aborted, interrupted) = {
        let guarded = session.lock().await;
        (
            guarded.trajectory_version,
            guarded.work_project_root.clone(),
            guarded.recent_request_ids.clone(),
            guarded.abort_flag.load(Ordering::Acquire),
            guarded.user_interrupt_flag.load(Ordering::Acquire),
        )
    };
    {
        let guarded = session.lock().await;
        let sessions = app.chat.sessions.try_read();
        if !sessions.as_ref().is_ok_and(|sessions| {
            sessions
                .get(&thread.id)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
        }) {
            return Some(Err(
                "Runner session changed before reference authorization".into()
            ));
        }
        if let Err(error) =
            crate::chat::worktree_lifecycle::check_unchanged_attachment(&guarded, thread)
        {
            return Some(Err(error));
        }
    }
    if let Err(error) =
        register_stateful_subchat_worktree(app.gcx.clone(), &thread.id, thread).await
    {
        return Some(Err(error));
    }
    {
        let sessions = match app.chat.sessions.try_read() {
            Ok(sessions) => sessions,
            Err(_) => return Some(Err("Runner session registry busy; retain progress".into())),
        };
        if !sessions
            .get(&thread.id)
            .is_some_and(|current| Arc::ptr_eq(current, &session))
        {
            return Some(Err("Runner session changed; refusing stale progress".into()));
        }
        let mut session = match session.try_lock() {
            Ok(session) => session,
            Err(_) => return Some(Err("Runner session busy; retain progress".into())),
        };
        if let Err(error) =
            crate::chat::worktree_lifecycle::check_unchanged_attachment(&session, thread)
        {
            return Some(Err(error));
        }
        let retains_current_history =
            messages
                .get(..session.messages.len())
                .is_some_and(|prefix| {
                    serde_json::to_value(prefix)
                        .ok()
                        .zip(serde_json::to_value(&session.messages).ok())
                        .is_some_and(|(proposed, current)| proposed == current)
                });
        if (!session.is_runner_owned_subagent_view()
            && session.trajectory_dirty
            && !retains_current_history)
            || session.trajectory_version != version
            || session.work_project_root != source
            || session.recent_request_ids != requests
            || session.abort_flag.load(Ordering::Acquire) != aborted
            || session.user_interrupt_flag.load(Ordering::Acquire) != interrupted
            || !session.command_queue.is_empty()
        {
            return Some(Err(
                "Runner no longer owns the current session; retain its history and checkout".into(),
            ));
        }
        session.mirror_runner_messages(messages.to_vec());
        session.increment_version();
    }
    Some(
        crate::chat::trajectories::try_save_trajectory_with_intent(app.clone(), session, intent)
            .await
            .and_then(|saved| {
                if saved || !intent.requires_durability() {
                    Ok(())
                } else {
                    Err("child trajectory changed before its save completed".into())
                }
            }),
    )
}

async fn save_subchat_trajectory(
    app: &AppState,
    config: &SubchatConfig,
    thread: &ThreadParams,
    messages: &[ChatMessage],
    intent: TrajectoryCommitIntent,
) -> Result<(), String> {
    match save_through_runner_session(app, config, thread, messages, intent).await {
        Some(result) => result,
        None => {
            crate::chat::trajectories::save_trajectory_as_with_intent_checked(
                app.gcx.clone(),
                thread,
                messages,
                intent,
            )
            .await
        }
    }
}

async fn refresh_live_context_window(ccx: &Arc<AMutex<AtCommandsContext>>, config: &SubchatConfig) {
    if matches!(config.tools, ToolsPolicy::Supervised { .. }) {
        return;
    }
    let (app, chat_id) = {
        let context = ccx.lock().await;
        (context.app.clone(), context.chat_id.clone())
    };
    let Some(session) = runner_owned_session(&app, config, &chat_id).await else {
        return;
    };
    let live_cap = session.lock().await.thread.context_tokens_cap;
    let model_n_ctx = match try_load_caps_quickly_if_not_present(app.gcx.clone(), 0).await {
        Ok(caps) => resolve_chat_model(caps, &config.model)
            .map(|model| model.base.n_ctx)
            .unwrap_or(0),
        Err(_) => 0,
    };
    let window = match (live_cap.filter(|cap| *cap > 0), model_n_ctx) {
        (Some(cap), 0) => cap,
        (Some(cap), model) => cap.min(model),
        (None, 0) => return,
        (None, model) => model,
    };
    ccx.lock().await.n_ctx = window;
}

async fn persist_subchat_progress(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    progress: &SubchatProgressMessages,
    messages: &[ChatMessage],
) {
    record_subchat_progress(progress, messages);
    if messages.is_empty() || !should_persist_subchat_trajectory(config) {
        return;
    }

    let (gcx, chat_id) = {
        let cgcx = ccx.lock().await;
        (cgcx.global_context.clone(), cgcx.chat_id.clone())
    };

    let thread = trace_thread_from_config(&chat_id, config);
    let app = AppState::from_gcx(gcx.clone()).await;
    let acknowledgements = if let Some(agent_id) = config.background_agent_id.as_deref() {
        app.agents
            .pending_deliveries(agent_id)
            .await
            .into_iter()
            .filter(|delivery| {
                messages.iter().any(|message| {
                    refact_core::chat_types::delivery_id_of_message(message)
                        == Some(delivery.id.as_str())
                })
            })
            .map(|delivery| delivery.id)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    // A checkpoint may merely schedule a write. A delivery may only be removed
    // from the durable registry after a required trajectory commit has completed.
    let intent = if acknowledgements.is_empty() {
        subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Progress)
    } else {
        TrajectoryCommitIntent::Required
    };
    match save_subchat_trajectory(&app, config, &thread, messages, intent).await {
        Ok(()) => {
            if let Some(agent_id) = config.background_agent_id.as_deref() {
                for id in acknowledgements {
                    if let Err(error) = app.agents.consume_delivery(agent_id, &id).await {
                        warn!(%agent_id, %id, %error, "Failed to mark durable runner delivery consumed");
                    }
                }
                crate::agents::delivery::publish_runner_queue(&app, agent_id).await;
            }
        }
        Err(error) => warn!(%error, "Failed to save runner progress; retaining delivery payloads"),
    }
}

#[cfg(test)]
pub(crate) async fn mirror_subchat_messages_into_session(
    app: &AppState,
    chat_id: &str,
    messages: &[ChatMessage],
) {
    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    let mut session = session_arc.lock().await;
    if session.closed {
        return;
    }
    if !session.is_runner_owned_subagent_view() && session.trajectory_dirty {
        return;
    }
    session.mirror_runner_messages(messages.to_vec());
}

fn take_subchat_progress(progress: &SubchatProgressMessages) -> Vec<ChatMessage> {
    match progress.lock() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

async fn save_failed_subchat_trajectory(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    config: &SubchatConfig,
    progress: &SubchatProgressMessages,
    error: &str,
) {
    if !should_persist_subchat_trajectory(config) {
        return;
    }
    let mut messages = take_subchat_progress(progress);
    if messages.is_empty() {
        return;
    }

    let error = safe_context_limit_error_for_log(error);
    messages.push(crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        "subchat.run",
        json!({"error": error, "tool_name": config.tool_name}),
        format!("Subchat run failed: {error}"),
    ));

    let thread = trace_thread_from_config(chat_id, config);
    let app = AppState::from_gcx(gcx).await;
    if let Err(error) = save_subchat_trajectory(
        &app,
        config,
        &thread,
        &messages,
        subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Failed),
    )
    .await
    {
        warn!("Failed to save trajectory: {}", error);
    }
}

fn task_meta_for_stateful_subchat(config: &SubchatConfig) -> Option<TaskMeta> {
    let mut task_meta = config.task_meta.clone()?;
    if task_meta.role == "planner"
        && config
            .link_type
            .as_deref()
            .map(is_subagentic_link_type)
            .unwrap_or(false)
    {
        task_meta.role = "subchats".to_string();
    }
    Some(task_meta)
}

pub(crate) fn stable_subchat_chat_id(
    config: &SubchatConfig,
    fresh_id: impl FnOnce() -> String,
) -> String {
    if let Some(chat_id) = config
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|chat_id| !chat_id.is_empty())
    {
        return chat_id.to_string();
    }
    if let Some(agent_id) = config
        .background_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|agent_id| !agent_id.is_empty())
    {
        return format!("subchat-{agent_id}");
    }
    fresh_id()
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{SubchatConfig, ToolsPolicy, TraceParent};

    pub(crate) fn minimal_config(chat_id: &str, parent: &str) -> SubchatConfig {
        SubchatConfig {
            history_source_model: None,
            tool_name: "subagent".into(),
            stateful: true,
            autonomous_no_confirm: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            chat_id: Some(chat_id.into()),
            title: Some("child".into()),
            parent_id: Some(parent.into()),
            link_type: Some("subagent".into()),
            root_chat_id: Some(parent.into()),
            tools: ToolsPolicy::None,
            max_steps: 1,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: None,
            worktree: None,
            model: "test/model".into(),
            mode: "agent".into(),
            n_ctx: 4096,
            implicit_context_window: false,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            soft_abort: false,
            background_agent_id: None,
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            activity_stamp: None,
            trace_parent: TraceParent::rooted(parent, parent),
        }
    }
}

pub(crate) fn prepare_child_history(
    messages: &[ChatMessage],
    config: &SubchatConfig,
) -> Vec<ChatMessage> {
    let mut selected = sanitize_messages_for_new_thread(messages);
    if config.history_source_model.as_deref() == Some(config.model.as_str()) {
        for (target, source) in
            selected.iter_mut().zip(messages.iter().filter(|message| {
                !refact_chat_history::trajectory_ops::is_ui_only_message(message)
            }))
        {
            target.thinking_blocks = source.thinking_blocks.clone();
            target.reasoning_content = source.reasoning_content.clone();
            target.server_content_blocks = source.server_content_blocks.clone();
        }
    }
    selected
}

pub async fn run_subchat(
    gcx: Arc<GlobalContext>,
    messages: Vec<ChatMessage>,
    mut config: SubchatConfig,
) -> Result<SubchatResult, String> {
    if !subchat_retries_allowed(&config) {
        config.wrap_up = None;
        config.final_step_force_answer = false;
    }
    info!(
        "run_subchat tool={} model={} stateful={}",
        config.tool_name, config.model, config.stateful
    );

    let chat_id = stable_subchat_chat_id(&config, || format!("subchat-{}", Uuid::new_v4()));

    let messages = refact_core::active_context::active_context(&messages)
        .map_err(|e| e.to_string())?
        .messages;
    let messages = sanitize_messages_for_new_thread(&messages);
    let resuming = config.stateful
        && (agent_resume_boundary(&messages, config.background_agent_id.as_deref()).is_some()
            || previous_agent_cap(&messages, config.background_agent_id.as_deref()).is_some());
    let messages = if resuming {
        messages
    } else {
        prepare_child_history(&messages, &config)
    };
    let messages = prepare_subchat_messages(&gcx, messages, &config.model)?;
    if should_persist_subchat_trajectory(&config) && !resuming && !config.stateful {
        let thread = trace_thread_from_config(&chat_id, &config);
        save_trajectory_as_with_intent(
            gcx.clone(),
            &thread,
            &messages,
            subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Seed),
        )
        .await;
    }
    let app = AppState::from_gcx(gcx.clone()).await;
    if config.stateful {
        let source = config
            .background_agent_id
            .as_deref()
            .and_then(|agent_id| app.agent_executions.saved_scope(agent_id));
        install_stateful_subchat_session(&app, &chat_id, &config, &messages, source.as_deref())
            .await?;
        if !resuming {
            let thread = trace_thread_from_config(&chat_id, &config);
            save_subchat_trajectory(
                &app,
                &config,
                &thread,
                &messages,
                subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Seed),
            )
            .await?;
        }
    }
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_with_abort(
            app.clone(),
            config.n_ctx,
            1,
            false,
            messages.clone(),
            chat_id.clone(),
            config.root_chat_id.clone(),
            config.model.clone(),
            config.task_meta.clone(),
            config.worktree.clone(),
            config.abort_flag.clone(),
        )
        .await,
    ));

    ccx.lock().await.subchat_depth = config.subchat_depth;
    ccx.lock().await.background_agent_id = config.background_agent_id.clone();
    ccx.lock().await.activity_stamp = config.activity_stamp.clone();
    ccx.lock().await.child_tool_ceiling = Some(config.tools.clone());

    if let Some(ref parent_tx) = config.parent_subchat_tx {
        ccx.lock().await.subchat_tx = parent_tx.clone();
    }

    let mut _usage = ChatUsage::default();

    let progress_messages: SubchatProgressMessages = Arc::new(StdMutex::new(messages.clone()));

    let current_messages_result: Result<(Vec<ChatMessage>, bool), String> = async {
        let mut messages = messages;
        loop {
            let (mut completed, aborted) = if let Some(ref wrap_up) = config.wrap_up {
                Box::pin(run_subchat_with_wrap_up(
                    ccx.clone(),
                    &config,
                    messages,
                    &config.tools,
                    wrap_up,
                    &mut _usage,
                    &progress_messages,
                ))
                .await?
            } else {
                Box::pin(run_subchat_loop(
                    ccx.clone(),
                    &config,
                    messages,
                    &config.tools,
                    &mut _usage,
                    &progress_messages,
                ))
                .await?
            };
            if aborted {
                break Ok((completed, true));
            }
            // Outside interruptible-wait boundaries, C becomes eligible after
            // forced finals and wrap-up. The registry closes acceptance atomically
            // with this drain so a successful enqueue cannot be stranded on exit.
            let wake = if let Some(agent_id) = config.background_agent_id.as_deref() {
                import_runner_local_deliveries(&app, agent_id, &chat_id, &completed).await;
                let deliveries =
                    crate::agents::delivery::finish_authorized_turn(&app, agent_id).await?;
                let wake = append_runner_deliveries(&mut completed, deliveries);
                crate::agents::delivery::publish_runner_queue(&app, agent_id).await;
                wake
            } else {
                false
            };
            persist_subchat_progress(&ccx, &config, &progress_messages, &completed).await;
            if !wake {
                break Ok((completed, false));
            }
            messages = completed;
        }
    }
    .await;
    let (mut current_messages, aborted) = match current_messages_result {
        Ok((messages, aborted)) => (messages, aborted),
        Err(e) => {
            save_failed_subchat_trajectory(gcx.clone(), &chat_id, &config, &progress_messages, &e)
                .await;
            finish_stateful_subchat_session(&app, &chat_id, &config).await;
            clear_unbound_openai_codex_websocket_session(&chat_id).await;
            return Err(e);
        }
    };

    let subagent_policy = gcx
        .privacy_policy_load
        .read()
        .unwrap()
        .policy
        .subagents
        .clone();
    if let Err(error) = apply_subchat_report_policy(&subagent_policy, &mut current_messages) {
        let error = error.to_string();
        save_failed_subchat_trajectory(gcx.clone(), &chat_id, &config, &progress_messages, &error)
            .await;
        finish_stateful_subchat_session(&app, &chat_id, &config).await;
        clear_unbound_openai_codex_websocket_session(&chat_id).await;
        return Err(error);
    }

    if config.stateful {
        let thread = stateful_thread_from_config(&chat_id, &config);
        if let Err(error) = save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &current_messages,
            subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Final),
        )
        .await
        {
            finish_stateful_subchat_session(&app, &chat_id, &config).await;
            clear_unbound_openai_codex_websocket_session(&chat_id).await;
            return Err(error);
        }
    } else if should_persist_subchat_trajectory(&config) {
        let thread = trace_thread_from_config(&chat_id, &config);
        save_trajectory_as_with_intent(
            gcx.clone(),
            &thread,
            &current_messages,
            subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Final),
        )
        .await;
    }

    let metering = aggregate_metering_from_messages(&current_messages);
    finish_stateful_subchat_session(&app, &chat_id, &config).await;
    clear_unbound_openai_codex_websocket_session(&chat_id).await;

    Ok(SubchatResult {
        messages: current_messages,
        metering,
        chat_id: if config.stateful { Some(chat_id) } else { None },
        aborted,
    })
}

pub(crate) async fn install_stateful_subchat_session(
    app: &AppState,
    chat_id: &str,
    config: &SubchatConfig,
    messages: &[ChatMessage],
    accepted_source: Option<&std::path::Path>,
) -> Result<(), String> {
    let source = match accepted_source {
        Some(source) => Some(
            crate::coordination::roots::resolve_source_workspace_root(
                app.gcx.clone(),
                Some(source.to_str().ok_or("accepted project scope is invalid")?),
            )
            .await?,
        ),
        None => None,
    };
    if let (Some(source), Some(worktree)) = (&source, &config.worktree) {
        if worktree.source_workspace_root.canonicalize().ok().as_ref() != Some(source) {
            return Err("child worktree does not match accepted project scope".into());
        }
    }
    let thread = stateful_thread_from_config(chat_id, config);
    let existing = app.chat.sessions.read().await.get(chat_id).cloned();
    let session_arc = match existing {
        Some(session) => session,
        None => {
            let mut session = crate::chat::types::ChatSession::new_with_trajectory(
                chat_id.to_string(),
                messages.to_vec(),
                thread.clone(),
                chrono::Utc::now().to_rfc3339(),
                None,
                Vec::new(),
                None,
            );
            session.work_project_root = source.clone();
            let session = Arc::new(AMutex::new(session));
            let _transition = crate::chat::worktree_lifecycle::try_transition_lock(&session)?;
            let mut sessions = app.chat.sessions.write().await;
            if sessions.contains_key(chat_id) {
                return Err("Child session appeared during setup; retain its current scope".into());
            }
            sessions.insert(chat_id.to_string(), session.clone());
            session
        }
    };
    let _transition = crate::chat::worktree_lifecycle::try_transition_lock(&session_arc)?;
    let saved_source = {
        let session = session_arc.lock().await;
        crate::coordination::scope::session_source_root(&session)
    };
    if let (Some(saved), Some(source)) = (&saved_source, &source) {
        if saved.canonicalize().ok().as_ref() != Some(source) {
            return Err("saved child session does not match accepted project scope".into());
        }
    }
    let sessions = app
        .chat
        .sessions
        .try_read()
        .map_err(|_| "Child session registry busy; retry setup")?;
    if !sessions
        .get(chat_id)
        .is_some_and(|current| Arc::ptr_eq(current, &session_arc))
    {
        return Err("Child session changed during setup".into());
    }
    let mut session = session_arc
        .try_lock()
        .map_err(|_| "Child session busy; retry setup")?;
    crate::chat::worktree_lifecycle::check_unchanged_attachment(&session, &thread)?;
    if crate::coordination::scope::session_source_root(&session) != saved_source {
        return Err("Child source scope changed during setup".into());
    }
    if let Some(source) = source {
        session.work_project_root = Some(source);
    }
    if config.background_agent_id.is_some() {
        session.set_runtime_state(crate::chat::types::SessionState::Generating, None);
    }
    Ok(())
}

pub(crate) async fn finish_stateful_subchat_session(
    app: &AppState,
    chat_id: &str,
    config: &SubchatConfig,
) {
    if !config.stateful || config.background_agent_id.is_none() {
        return;
    }
    let session = app.chat.sessions.read().await.get(chat_id).cloned();
    if let Some(session) = session {
        let Ok(_transition) = crate::chat::worktree_lifecycle::try_transition_lock(&session) else {
            warn!("Runner completion deferred during checkout transition");
            return;
        };
        let Ok(sessions) = app.chat.sessions.try_read() else {
            return;
        };
        if !sessions
            .get(chat_id)
            .is_some_and(|current| Arc::ptr_eq(current, &session))
        {
            return;
        }
        let Ok(mut session) = session.try_lock() else {
            return;
        };
        let thread = stateful_thread_from_config(chat_id, config);
        if crate::chat::worktree_lifecycle::check_unchanged_attachment(&session, &thread).is_err()
            || !session.is_runner_owned_subagent_view()
            || session.runtime.paused
            || !session.runtime.pause_reasons.is_empty()
        {
            return;
        }
        session.set_runtime_state(crate::chat::types::SessionState::Idle, None);
    }
}

pub async fn run_subchat_once(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
    messages: Vec<ChatMessage>,
    trace_parent: TraceParent,
) -> Result<SubchatResult, String> {
    run_subchat_once_with_abort(gcx, tool_name, messages, None, trace_parent).await
}

pub async fn run_subchat_once_with_abort(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
    messages: Vec<ChatMessage>,
    abort_flag: Option<Arc<AtomicBool>>,
    trace_parent: TraceParent,
) -> Result<SubchatResult, String> {
    let mut config = resolve_subchat_config_with_parent(
        gcx.clone(),
        tool_name,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        None,
        "agent".to_string(),
        None,
        None,
        None,
        None,
        abort_flag,
        0,
    )
    .await?;
    config.trace_parent = trace_parent;

    run_subchat(gcx, messages, config).await
}

pub async fn run_subchat_once_with_parent(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
    messages: Vec<ChatMessage>,
    parent_tool_call_id: String,
    parent_subchat_tx: Arc<AMutex<mpsc::UnboundedSender<Value>>>,
    parent_abort_flag: Arc<AtomicBool>,
    parent_depth: usize,
    parent_task_meta: Option<TaskMeta>,
    parent_worktree: Option<WorktreeMeta>,
    trace_parent: TraceParent,
) -> Result<SubchatResult, String> {
    let mut config = resolve_subchat_config_with_parent(
        gcx.clone(),
        tool_name,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        None,
        "agent".to_string(),
        parent_task_meta,
        parent_worktree,
        Some(parent_tool_call_id),
        Some(parent_subchat_tx),
        Some(parent_abort_flag),
        parent_depth + 1,
    )
    .await?;
    config.trace_parent = trace_parent;

    run_subchat(gcx, messages, config).await
}

#[cfg(test)]
mod progress_collector_tests {
    use super::SubchatProgressCollector;
    use serde_json::json;

    #[test]
    fn test_append_tail_unicode_no_panic() {
        let mut s = String::new();
        // Force truncation in the middle of multibyte chars.
        for _ in 0..50 {
            SubchatProgressCollector::append_tail(&mut s, "✅", 10);
        }
        assert!(s.is_char_boundary(s.len()));
    }

    #[test]
    fn tool_arg_preview_uses_first_safe_scalar_and_sanitizes_whitespace() {
        assert_eq!(
            super::tool_arg_preview(r#"{"token":"hidden","path":"a\n b"}"#),
            Some("a b".to_string())
        );
        assert_eq!(super::tool_arg_preview(r#"{"password":"hidden"}"#), None);
    }
}

#[cfg(test)]
mod convert_results_tests {
    use super::*;

    #[test]
    fn empty_results_error_instead_of_masking_failure() {
        let messages = vec![ChatMessage::new(
            "user".to_string(),
            "review this".to_string(),
        )];
        let result = convert_results_to_messages(vec![], messages);
        assert!(result.is_err());
    }

    #[test]
    fn empty_content_without_tool_calls_errors() {
        let messages = vec![ChatMessage::new(
            "user".to_string(),
            "review this".to_string(),
        )];
        let choice = ChoiceFinal {
            content: "  ".to_string(),
            finish_reason: Some("length".to_string()),
            ..Default::default()
        };
        let error = convert_results_to_messages(vec![choice], messages).unwrap_err();
        assert!(error.contains("no visible content"));
        assert!(error.contains("length"));
    }

    #[test]
    fn empty_choice_is_skipped_when_another_has_content() {
        let messages = vec![ChatMessage::new(
            "user".to_string(),
            "review this".to_string(),
        )];
        let empty = ChoiceFinal {
            content: String::new(),
            ..Default::default()
        };
        let full = ChoiceFinal {
            content: "all good".to_string(),
            ..Default::default()
        };
        let result = convert_results_to_messages(vec![empty, full], messages).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0][1].content.content_text_only(), "all good");
    }
}

fn is_abort_error(err: &str) -> bool {
    err.eq_ignore_ascii_case("aborted")
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn stamp_activity(stamp: &Option<Arc<AtomicU64>>) {
    if let Some(stamp) = stamp {
        stamp.store(epoch_ms(), Ordering::Relaxed);
    }
}

fn is_aborted(abort_flag: &Option<Arc<AtomicBool>>) -> bool {
    abort_flag
        .as_ref()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
}

fn final_step_wrap_up_message(max_steps: usize) -> ChatMessage {
    ChatMessage::new(
        "user".to_string(),
        format!(
            "⚠️ Step budget reached ({max_steps}/{max_steps}): no more tool calls are available. Stop investigating now and write your final response using everything you have gathered so far, following the exact final report / Status format defined in your system prompt. Do not call any tools. If something is still unverified, give your best partial findings and flag the gaps — never return an empty result."
        ),
    )
}

fn needs_forced_final_answer(
    final_step_force_answer: bool,
    aborted: bool,
    has_answer: bool,
) -> bool {
    final_step_force_answer && !aborted && !has_answer
}

/// Race only the uncommitted model draft, never tool execution. Dropping this
/// future discards partial output without setting the runner's permanent abort flag.
async fn wait_for_runner_preempt(app: &AppState, agent_id: &str) {
    let Some(notify) = app.agents.interrupt_notify(agent_id).await else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if app.agents.has_preempt(agent_id).await {
            return;
        }
        notified.await;
    }
}

async fn runner_model_turn(
    ccx: Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    messages: &mut Vec<ChatMessage>,
    tools_subset: Option<Vec<String>>,
    prepend_system_prompt: bool,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    let projected =
        refact_core::active_context::active_context(messages).map_err(|error| error.to_string())?;
    if !runner_tool_window_closed(&projected.messages) {
        return Err(
            "Cannot generate a runner draft before all tool calls have results".to_string(),
        );
    }
    let (app, agent_id, chat_id) = {
        let context = ccx.lock().await;
        (
            context.app.clone(),
            context.background_agent_id.clone(),
            context.chat_id.clone(),
        )
    };
    loop {
        if hold_native_pause(&ccx, config).await {
            return Err("Aborted".to_string());
        }
        refresh_live_context_window(&ccx, config).await;
        let execution = app.agent_executions.get(&chat_id);
        let _operation = if let Some(execution) = &execution {
            Some(
                execution
                    .begin_operation()
                    .await
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let result = {
            let generation = subchat_single_internal(
                ccx.clone(),
                &config.model,
                &config.mode,
                refact_core::active_context::active_context(messages)
                    .map_err(|e| e.to_string())?
                    .messages,
                tools_subset.clone(),
                false,
                config.temperature,
                config.max_new_tokens,
                config.reasoning_effort.clone(),
                config.cache_control,
                prepend_system_prompt,
                None,
                subchat_retries_allowed(config),
            );
            tokio::pin!(generation);
            if let Some(agent_id) = agent_id.as_deref() {
                tokio::select! {
                    biased;
                    _ = wait_for_runner_preempt(&app, agent_id) => None,
                    result = &mut generation => Some(result),
                }
            } else {
                Some(generation.await)
            }
        };
        if let Some(result) = result {
            let projected_len = refact_core::active_context::active_context(messages)
                .map_err(|e| e.to_string())?
                .messages
                .len();
            return result.map(|choices| {
                choices
                    .into_iter()
                    .map(|choice| {
                        let mut raw = messages.clone();
                        raw.extend(choice.into_iter().skip(projected_len));
                        raw
                    })
                    .collect()
            });
        }
        // The cancelled generation has been dropped before touching its context.
        clear_unbound_openai_codex_websocket_session(&chat_id).await;
        drain_background_agent_inbox(&ccx, messages).await;
        if let Some(agent_id) = agent_id.as_deref() {
            if app.agents.has_preempt(agent_id).await {
                // A concurrent new preemption can retry, but never busy-spin on
                // a failed durable drain: let the registry error reach the caller.
                let deliveries =
                    crate::agents::delivery::drain_authorized(&app, agent_id, false).await?;
                append_runner_deliveries(messages, deliveries);
                crate::agents::delivery::publish_runner_queue(&app, agent_id).await;
            }
        }
        if is_aborted(&config.abort_flag) {
            return Err("Aborted".to_string());
        }
    }
}

fn agent_resume_boundary(messages: &[ChatMessage], agent_id: Option<&str>) -> Option<(usize, u64)> {
    let agent_id = agent_id?;
    messages.iter().rev().find_map(|message| {
        if message.role != "event" {
            return None;
        }
        let event = message.extra.get("event")?;
        if !matches!(
            event.get("source")?.as_str()?,
            "agents.resume" | "agents.continued_turn"
        ) {
            return None;
        }
        let payload = event.get("payload")?;
        if payload.get("agent_id")?.as_str()? != agent_id {
            return None;
        }
        Some((
            usize::try_from(payload.get("steps_before")?.as_u64()?).ok()?,
            payload.get("run_generation")?.as_u64()?,
        ))
    })
}

pub(crate) fn previous_agent_cap(
    messages: &[ChatMessage],
    agent_id: Option<&str>,
) -> Option<crate::agents::types::CapStop> {
    let agent_id = agent_id?;
    messages.iter().rev().find_map(|message| {
        if message.role != "event" {
            return None;
        }
        let event = message.extra.get("event")?;
        if event.get("source").and_then(Value::as_str) != Some("agents.cap_stop") {
            return None;
        }
        let payload = event.get("payload")?;
        if payload.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
            return None;
        }
        serde_json::from_value(payload.get("cap_stop")?.clone()).ok()
    })
}

pub(crate) fn agent_step_cap(
    messages: &[ChatMessage],
    agent_id: Option<&str>,
    remaining: usize,
) -> crate::agents::types::CapStop {
    let previous = previous_agent_cap(messages, agent_id);
    let boundary = agent_resume_boundary(messages, agent_id);
    let steps_before = boundary.map_or_else(
        || previous.as_ref().map_or(0, |stop| stop.used),
        |(steps, _)| steps,
    );
    let total = steps_before.saturating_add(remaining);
    let goal_limit = crate::chat::goal_snapshot_from_messages(messages, None)
        .and_then(|goal| goal.budget.max_turns)
        .filter(|limit| *limit > 0);
    let goal_cap = goal_limit.is_some_and(|limit| total as u64 >= u64::from(limit));
    crate::agents::types::CapStop {
        key: if goal_cap {
            refact_work_core::CapKind::GoalTurns
        } else {
            refact_work_core::CapKind::Steps
        },
        scope: refact_work_core::CapScope::Child,
        limit: total,
        used: total,
        resumable: !goal_cap,
        run_generation: boundary.map_or_else(
            || {
                previous
                    .as_ref()
                    .map_or(1, |stop| stop.run_generation.saturating_add(1))
            },
            |(_, generation)| generation,
        ),
    }
}

#[cfg(test)]
mod cap_history_tests {
    use super::*;

    #[test]
    fn cap_accounting_selects_only_this_agents_bound_event() {
        let stop = crate::agents::types::CapStop {
            key: refact_work_core::CapKind::Steps,
            scope: refact_work_core::CapScope::Child,
            limit: 12,
            used: 12,
            resumable: true,
            run_generation: 1,
        };
        let message = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "agents.cap_stop",
            serde_json::json!({"agent_id": "child", "cap_stop": stop}),
            "cap reached",
        );
        assert_eq!(
            previous_agent_cap(&[message.clone()], Some("child")),
            Some(stop)
        );
        assert!(previous_agent_cap(&[message.clone()], Some("parent")).is_none());
        assert!(previous_agent_cap(&[message.clone()], None).is_none());
        let mut historical_text = message;
        historical_text.role = "user".into();
        assert!(previous_agent_cap(&[historical_text], Some("child")).is_none());
        for source in ["agents.resume", "agents.continued_turn"] {
            let boundary = |agent: &str, steps: usize, generation: u64| {
                event(
                    EventSubkind::SystemNotice,
                    source,
                    json!({"agent_id": agent, "steps_before": steps, "run_generation": generation}),
                    "continue",
                )
            };
            let mut messages = vec![
                boundary("child", 2, 1),
                boundary("child", 5, 2),
                boundary("other", 99, 9),
            ];
            assert_eq!(
                agent_resume_boundary(&messages, Some("child")),
                Some((5, 2))
            );
            assert!(agent_resume_boundary(&messages, None).is_none());
            let step_stop = agent_step_cap(&messages, Some("child"), 1);
            assert_eq!(
                (step_stop.limit, step_stop.used, step_stop.run_generation),
                (6, 6, 2)
            );
            assert!(step_stop.parent_can_raise_steps());
            messages.push(crate::chat::internal_roles::goal(
                "agent",
                1,
                "finite",
                None,
                true,
                crate::chat::types::GoalBudget {
                    max_turns: Some(6),
                    explicit: true,
                    ..Default::default()
                },
            ));
            let goal_stop = agent_step_cap(&messages, Some("child"), 1);
            assert_eq!(goal_stop.key, refact_work_core::CapKind::GoalTurns);
            assert!(!goal_stop.resumable);
            assert!(!goal_stop.parent_can_raise_steps());
        }
    }
}

async fn run_subchat_loop(
    ccx: Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    mut messages: Vec<ChatMessage>,
    tools_policy: &ToolsPolicy,
    usage: &mut ChatUsage,
    progress: &SubchatProgressMessages,
) -> Result<(Vec<ChatMessage>, bool), String> {
    let mut context_limit_compact_count = 0usize;
    let mut empty_choice_retry_count = 0usize;
    let mut supervised_cost = 0.0_f64;
    for step in 0..config.max_steps {
        if is_aborted(&config.abort_flag) {
            if config.soft_abort {
                return Ok((messages, true));
            }
            return Err("Aborted".to_string());
        }
        // Model boundary: honour a native pause before any new dispatch.
        if hold_native_pause(&ccx, config).await {
            if config.soft_abort {
                return Ok((messages, true));
            }
            return Err("Aborted".to_string());
        }
        stamp_activity(&config.activity_stamp);
        emit_subchat_progress(config, SubchatProgress::Step(step + 1));
        drain_background_agent_inbox(&ccx, &mut messages).await;
        persist_subchat_progress(&ccx, config, progress, &messages).await;

        // Reserve the entire possible input/output envelope before a provider
        // call. Unknown pricing/usage fails closed; retries and wrap-up calls
        // are disabled for supervision, and budgets are never increased.
        let mut supervised_pricing = None;
        if let ToolsPolicy::Supervised { budget, .. } = tools_policy {
            let envelope = config.n_ctx.saturating_add(config.max_new_tokens) as u64;
            if budget.max_turns.is_none_or(|limit| step >= limit as usize)
                || budget.max_tokens.is_none_or(|limit| {
                    (usage.total_tokens as u64).saturating_add(envelope) > limit
                })
            {
                return Err("Aborted: supervised token/turn budget exhausted".into());
            }
            if let Some(cents) = budget.max_cost_cents {
                let gcx = ccx.lock().await.global_context.clone();
                let pricing = crate::providers::pricing::lookup_model_pricing(&gcx, &config.model)
                    .await
                    .ok_or("Aborted: supervised cost pricing unavailable")?;
                let reserve = ChatUsage {
                    prompt_tokens: config.n_ctx as _,
                    completion_tokens: config.max_new_tokens as _,
                    total_tokens: envelope as _,
                    ..Default::default()
                };
                let cost = crate::providers::pricing::compute_cost(&reserve, &pricing)
                    .ok_or("Aborted: supervised cost bound unavailable")?
                    .total_usd;
                if !cost.is_finite() || cost < 0.0 || supervised_cost + cost > cents as f64 / 100.0
                {
                    return Err("Aborted: supervised cost budget exhausted".into());
                }
                supervised_pricing = Some(pricing);
            }
        }

        let results = loop {
            match runner_model_turn(
                ccx.clone(),
                config,
                &mut messages,
                tools_policy.to_subset_for_llm(),
                config.prepend_system_prompt && step == 0,
            )
            .await
            {
                Ok(r) => break r,
                Err(ref err)
                    if subchat_retries_allowed(config)
                        && should_compact_context_limit_error(
                            err,
                            context_limit_compact_count,
                            &config.abort_flag,
                        ) =>
                {
                    let original_error = err.clone();
                    let log_error = safe_context_limit_error_for_log(&original_error);
                    context_limit_compact_count += 1;
                    warn!(
                        "Subchat context limit, rebuilding context attempt {}/{}: {}",
                        context_limit_compact_count, MAX_CONTEXT_LIMIT_COMPACT_ATTEMPTS, log_error,
                    );
                    apply_subchat_reactive_compaction(
                        ccx.lock().await.global_context.clone(),
                        config,
                        &mut messages,
                        &original_error,
                        context_limit_compact_count,
                        false,
                    )
                    .await;
                }
                Err(ref err)
                    if err.starts_with(EMPTY_CHOICE_ERROR_PREFIX)
                        && subchat_retries_allowed(config)
                        && empty_choice_retry_count < MAX_EMPTY_CHOICE_RETRIES
                        && !is_aborted(&config.abort_flag) =>
                {
                    empty_choice_retry_count += 1;
                    warn!(
                        "Subchat returned no visible content, retrying step ({}/{}): {}",
                        empty_choice_retry_count, MAX_EMPTY_CHOICE_RETRIES, err,
                    );
                }
                Err(err) => {
                    if config.soft_abort && is_abort_error(&err) {
                        return Ok((messages, true));
                    }
                    return Err(err);
                }
            }
        };

        if let ToolsPolicy::Supervised { budget, .. } = tools_policy {
            let actual = results
                .first()
                .and_then(|messages| messages.last())
                .and_then(|m| m.usage.as_ref())
                .ok_or("Aborted: supervised provider usage unavailable")?;
            if let Some(pricing) = supervised_pricing.as_ref() {
                let cost = crate::providers::pricing::compute_cost(actual, pricing)
                    .ok_or("Aborted: supervised provider cost unavailable")?
                    .total_usd;
                if !cost.is_finite() || cost < 0.0 {
                    return Err("Aborted: invalid provider cost".into());
                }
                supervised_cost += cost;
            }
            if budget.max_tokens.is_none_or(|limit| {
                (usage.total_tokens as u64).saturating_add(actual.total_tokens as u64) > limit
            }) || budget
                .max_cost_cents
                .is_some_and(|limit| supervised_cost > limit as f64 / 100.0)
            {
                return Err("Aborted: supervised provider budget exhausted".into());
            }
        }
        update_usage_from_messages(usage, &results);
        let gcx = ccx.lock().await.global_context.clone();
        emit_usage_progress(&gcx, config, &results).await;
        messages = results.into_iter().next().unwrap_or(messages);
        persist_subchat_progress(&ccx, config, progress, &messages).await;

        if has_final_answer(&messages) {
            break;
        }

        // Tool boundary: the provider outcome above is retained; a pause
        // parks here and cancellation exits before any tool starts.
        if hold_native_pause(&ccx, config).await {
            if config.soft_abort {
                return Ok((messages, true));
            }
            return Err("Aborted".to_string());
        }
        messages = execute_pending_tool_calls(
            ccx.clone(),
            &config.model,
            &config.mode,
            messages,
            tools_policy,
            step,
            config.max_steps,
            config.parent_tool_call_id.clone(),
            config.autonomous_no_confirm,
            config.auto_approve_editing_tools,
            config.auto_approve_dangerous_commands,
            config.step_progress.clone(),
        )
        .await?;
        persist_subchat_progress(&ccx, config, progress, &messages).await;

        if is_aborted(&config.abort_flag) {
            if config.soft_abort {
                return Ok((messages, true));
            }
            return Err("Aborted".to_string());
        }
    }

    if config.background_agent_id.is_some()
        && !has_final_answer(&messages)
        && !is_aborted(&config.abort_flag)
    {
        let stop = agent_step_cap(
            &messages,
            config.background_agent_id.as_deref(),
            config.max_steps,
        );
        let is_goal_cap = stop.key == refact_work_core::CapKind::GoalTurns;
        let error = stop.as_error().map_err(|error| error.to_string())?;
        messages.push(crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "agents.cap_stop", serde_json::json!({"agent_id": config.background_agent_id, "cap_stop": stop}),
            if is_goal_cap {
                "Explicit goal turn budget reached; a step-cap override cannot widen the goal budget."
            } else {
                "Step limit reached. Parent may use agent_lifecycle action inspect, then raise_and_recover with the current change sequence to continue this child's task."
            },
        ));
        persist_subchat_progress(&ccx, config, progress, &messages).await;
        return Err(error);
    }

    if needs_forced_final_answer(
        config.final_step_force_answer && subchat_retries_allowed(config),
        is_aborted(&config.abort_flag),
        has_final_answer(&messages),
    ) {
        drain_background_agent_inbox(&ccx, &mut messages).await;
        messages = run_forced_final_answer_turn(
            ccx.clone(),
            config,
            messages,
            &mut context_limit_compact_count,
            usage,
        )
        .await?;
        persist_subchat_progress(&ccx, config, progress, &messages).await;
    }

    Ok((messages, is_aborted(&config.abort_flag)))
}

async fn run_forced_final_answer_turn(
    ccx: Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    mut messages: Vec<ChatMessage>,
    context_limit_compact_count: &mut usize,
    usage: &mut ChatUsage,
) -> Result<Vec<ChatMessage>, String> {
    messages.push(final_step_wrap_up_message(config.max_steps));
    let mut empty_choice_retry_count = 0usize;

    let results = loop {
        match runner_model_turn(ccx.clone(), config, &mut messages, Some(vec![]), false).await {
            Ok(r) => break r,
            Err(ref err)
                if subchat_retries_allowed(config)
                    && should_compact_context_limit_error(
                        err,
                        *context_limit_compact_count,
                        &config.abort_flag,
                    ) =>
            {
                let original_error = err.clone();
                let log_error = safe_context_limit_error_for_log(&original_error);
                *context_limit_compact_count += 1;
                warn!(
                    "Subchat forced final answer context limit, rebuilding context attempt {}/{}: {}",
                    *context_limit_compact_count, MAX_CONTEXT_LIMIT_COMPACT_ATTEMPTS, log_error,
                );
                apply_subchat_reactive_compaction(
                    ccx.lock().await.global_context.clone(),
                    config,
                    &mut messages,
                    &original_error,
                    *context_limit_compact_count,
                    true,
                )
                .await;
            }
            Err(ref err)
                if err.starts_with(EMPTY_CHOICE_ERROR_PREFIX)
                    && subchat_retries_allowed(config)
                    && empty_choice_retry_count < MAX_EMPTY_CHOICE_RETRIES
                    && !is_aborted(&config.abort_flag) =>
            {
                empty_choice_retry_count += 1;
                warn!(
                    "Subchat forced final answer returned no visible content, retrying ({}/{}): {}",
                    empty_choice_retry_count, MAX_EMPTY_CHOICE_RETRIES, err,
                );
            }
            Err(err) => {
                if config.soft_abort && is_abort_error(&err) {
                    return Ok(messages);
                }
                return Err(err);
            }
        }
    };

    update_usage_from_messages(usage, &results);
    let gcx = ccx.lock().await.global_context.clone();
    emit_usage_progress(&gcx, config, &results).await;
    Ok(results.into_iter().next().unwrap_or(messages))
}

async fn run_subchat_with_wrap_up(
    ccx: Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    mut messages: Vec<ChatMessage>,
    tools_policy: &ToolsPolicy,
    wrap_up: &WrapUpConfig,
    usage: &mut ChatUsage,
    progress: &SubchatProgressMessages,
) -> Result<(Vec<ChatMessage>, bool), String> {
    let mut step_n = 0;
    let mut context_limit_compact_count = 0usize;
    let mut empty_choice_retry_count = 0usize;

    loop {
        if hold_native_pause(&ccx, config).await {
            if config.soft_abort {
                return Ok((messages, true));
            }
            return Err("Aborted".to_string());
        }

        stamp_activity(&config.activity_stamp);
        emit_subchat_progress(config, SubchatProgress::Step(step_n + 1));
        drain_background_agent_inbox(&ccx, &mut messages).await;
        persist_subchat_progress(&ccx, config, progress, &messages).await;

        if has_final_answer(&messages) {
            break;
        }

        let last_message = match messages.last() {
            Some(m) => m,
            None => break,
        };

        if last_message.role == "assistant"
            && last_message
                .tool_calls
                .as_ref()
                .map_or(false, |tc| !tc.is_empty())
        {
            if step_n >= wrap_up.depth {
                break;
            }
            if let Some(msg_usage) = &last_message.usage {
                if msg_usage.prompt_tokens + msg_usage.completion_tokens > wrap_up.tokens_cnt {
                    break;
                }
            }
        }

        let results = loop {
            match runner_model_turn(
                ccx.clone(),
                config,
                &mut messages,
                tools_policy.to_subset_for_llm(),
                config.prepend_system_prompt && step_n == 0,
            )
            .await
            {
                Ok(r) => break r,
                Err(ref err)
                    if subchat_retries_allowed(config)
                        && should_compact_context_limit_error(
                            err,
                            context_limit_compact_count,
                            &config.abort_flag,
                        ) =>
                {
                    let original_error = err.clone();
                    let log_error = safe_context_limit_error_for_log(&original_error);
                    context_limit_compact_count += 1;
                    warn!(
                        "Subchat wrap-up context limit, rebuilding context attempt {}/{}: {}",
                        context_limit_compact_count, MAX_CONTEXT_LIMIT_COMPACT_ATTEMPTS, log_error,
                    );
                    apply_subchat_reactive_compaction(
                        ccx.lock().await.global_context.clone(),
                        config,
                        &mut messages,
                        &original_error,
                        context_limit_compact_count,
                        false,
                    )
                    .await;
                }
                Err(ref err)
                    if err.starts_with(EMPTY_CHOICE_ERROR_PREFIX)
                        && subchat_retries_allowed(config)
                        && empty_choice_retry_count < MAX_EMPTY_CHOICE_RETRIES
                        && !is_aborted(&config.abort_flag) =>
                {
                    empty_choice_retry_count += 1;
                    warn!(
                        "Subchat returned no visible content, retrying step ({}/{}): {}",
                        empty_choice_retry_count, MAX_EMPTY_CHOICE_RETRIES, err,
                    );
                }
                Err(err) => {
                    if config.soft_abort && is_abort_error(&err) {
                        return Ok((messages, true));
                    }
                    return Err(err);
                }
            }
        };

        update_usage_from_messages(usage, &results);
        let gcx = ccx.lock().await.global_context.clone();
        emit_usage_progress(&gcx, config, &results).await;
        messages = results.into_iter().next().unwrap_or(messages);
        persist_subchat_progress(&ccx, config, progress, &messages).await;

        messages = execute_pending_tool_calls(
            ccx.clone(),
            &config.model,
            &config.mode,
            messages,
            tools_policy,
            step_n,
            config.max_steps,
            config.parent_tool_call_id.clone(),
            config.autonomous_no_confirm,
            config.auto_approve_editing_tools,
            config.auto_approve_dangerous_commands,
            config.step_progress.clone(),
        )
        .await?;
        persist_subchat_progress(&ccx, config, progress, &messages).await;

        step_n += 1;

        if is_aborted(&config.abort_flag) {
            if config.soft_abort {
                return Ok((messages, true));
            }
            return Err("Aborted".to_string());
        }
    }

    if is_aborted(&config.abort_flag) {
        if config.soft_abort {
            return Ok((messages, true));
        }
        return Err("Aborted".to_string());
    }

    messages = execute_pending_tool_calls(
        ccx.clone(),
        &config.model,
        &config.mode,
        messages,
        tools_policy,
        step_n,
        config.max_steps,
        config.parent_tool_call_id.clone(),
        config.autonomous_no_confirm,
        config.auto_approve_editing_tools,
        config.auto_approve_dangerous_commands,
        config.step_progress.clone(),
    )
    .await?;
    persist_subchat_progress(&ccx, config, progress, &messages).await;

    messages.push(ChatMessage::new("user".to_string(), wrap_up.prompt.clone()));
    drain_background_agent_inbox(&ccx, &mut messages).await;
    record_subchat_progress(progress, &messages);

    let final_results = loop {
        match runner_model_turn(ccx.clone(), config, &mut messages, Some(vec![]), false).await {
            Ok(r) => break r,
            Err(ref err)
                if subchat_retries_allowed(config)
                    && should_compact_context_limit_error(
                        err,
                        context_limit_compact_count,
                        &config.abort_flag,
                    ) =>
            {
                let original_error = err.clone();
                let log_error = safe_context_limit_error_for_log(&original_error);
                context_limit_compact_count += 1;
                warn!(
                    "Subchat wrap-up final context limit, rebuilding context attempt {}/{}: {}",
                    context_limit_compact_count, MAX_CONTEXT_LIMIT_COMPACT_ATTEMPTS, log_error,
                );
                apply_subchat_reactive_compaction(
                    ccx.lock().await.global_context.clone(),
                    config,
                    &mut messages,
                    &original_error,
                    context_limit_compact_count,
                    true,
                )
                .await;
            }
            Err(err) => {
                if config.soft_abort && is_abort_error(&err) {
                    return Ok((messages, true));
                }
                return Err(err);
            }
        }
    };
    update_usage_from_messages(usage, &final_results);
    let gcx = ccx.lock().await.global_context.clone();
    emit_usage_progress(&gcx, config, &final_results).await;

    Ok((
        final_results.into_iter().next().unwrap_or_default(),
        is_aborted(&config.abort_flag),
    ))
}

fn truncate_args(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let boundary = s
        .char_indices()
        .take_while(|(i, _)| *i < max)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    format!("{}…", &s[..boundary])
}

fn emit_subchat_progress(config: &SubchatConfig, progress: SubchatProgress) {
    if let Some(callback) = &config.step_progress {
        callback(progress);
    }
}

async fn emit_usage_progress(
    gcx: &Arc<GlobalContext>,
    config: &SubchatConfig,
    results: &[Vec<ChatMessage>],
) {
    let usage = results
        .first()
        .and_then(|messages| messages.last())
        .and_then(|message| message.usage.as_ref());
    let Some(usage) = usage else {
        return;
    };
    let tokens_delta = usage.total_tokens as u64;
    if tokens_delta > 0 {
        let cost_delta = match usage.metering_usd.as_ref() {
            Some(metering) => Some(metering.total_usd),
            None => crate::providers::pricing::lookup_model_pricing(gcx, &config.model)
                .await
                .and_then(|pricing| {
                    crate::providers::pricing::compute_cost(usage, &pricing)
                        .map(|metering| metering.total_usd)
                }),
        };
        emit_subchat_progress(
            config,
            SubchatProgress::Usage {
                tokens_delta,
                cost_delta,
            },
        );
    }
}

fn tool_arg_preview(arguments: &str) -> Option<String> {
    static SECRET_ARG_NAME: OnceLock<regex::Regex> = OnceLock::new();
    let value: Value = serde_json::from_str(arguments).ok()?;
    let object = value.as_object()?;
    for (key, value) in object {
        if SECRET_ARG_NAME
            .get_or_init(|| regex::Regex::new("(?i)key|token|secret|password").unwrap())
            .is_match(key)
        {
            continue;
        }
        let scalar = match value {
            Value::String(value) => value.clone(),
            Value::Number(value) => value.to_string(),
            Value::Bool(value) => value.to_string(),
            _ => continue,
        };
        let preview = scalar.split_whitespace().collect::<Vec<_>>().join(" ");
        if !preview.is_empty() {
            return Some(truncate_args(&preview, 40));
        }
    }
    None
}

async fn execute_pending_tool_calls(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model_id: &str,
    mode_id: &str,
    mut messages: Vec<ChatMessage>,
    tools_policy: &ToolsPolicy,
    step_idx: usize,
    max_steps: usize,
    tx_toolid_mb: Option<String>,
    autonomous_no_confirm: bool,
    auto_approve_editing_tools: bool,
    auto_approve_dangerous_commands: bool,
    step_progress: Option<Arc<dyn Fn(SubchatProgress) + Send + Sync>>,
) -> Result<Vec<ChatMessage>, String> {
    let (n_ctx, task_meta, worktree, chat_id, root_chat_id, app, agent_id, abort_flag) = {
        let cgcx = ccx.lock().await;
        (
            cgcx.n_ctx,
            cgcx.task_meta.clone(),
            cgcx.execution_scope_worktree(),
            cgcx.chat_id.clone(),
            cgcx.root_chat_id.clone(),
            cgcx.app.clone(),
            cgcx.background_agent_id.clone(),
            cgcx.abort_flag.clone(),
        )
    };
    let native_pause = if let Some(agent_id) = agent_id {
        app.agents.pause_gate(&agent_id).await
    } else {
        None
    };
    if matches!(tools_policy, ToolsPolicy::Supervised { .. }) && native_pause.is_none() {
        return Err(
            "supervised native tool dispatch missing background agent identity".to_string(),
        );
    }
    let last = match messages.last() {
        Some(m) => m,
        None => return Ok(messages),
    };
    let tool_calls = match &last.tool_calls {
        Some(tc) if !tc.is_empty() => tc.clone(),
        _ => return Ok(messages),
    };
    let tool_calls =
        resolve_tool_call_aliases(app.clone(), tool_calls, mode_id, Some(model_id)).await;

    for tool_call in &tool_calls {
        if let Some(callback) = &step_progress {
            callback(SubchatProgress::ToolStarted {
                name: tool_call.function.name.clone(),
                arg_preview: tool_arg_preview(&tool_call.function.arguments),
            });
        }
    }

    let mut allowed: Vec<ChatToolCall> = vec![];
    let mut denied_msgs: Vec<ChatMessage> = vec![];

    for tc in tool_calls.iter() {
        if !crate::agents::tool_authority::allows_dispatch(tools_policy, &tc.function.name) {
            denied_msgs.push(ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                tool_call_id: tc.id.clone(),
                tool_failed: Some(true),
                content: ChatContent::SimpleText(format!(
                    "Tool '{}' not allowed in this subchat",
                    tc.function.name
                )),
                ..Default::default()
            });
        } else {
            allowed.push(tc.clone());
        }
    }

    let thread = ThreadParams {
        id: chat_id,
        model: model_id.to_string(),
        mode: mode_id.to_string(),
        context_tokens_cap: Some(n_ctx),
        task_meta,
        worktree,
        root_chat_id: Some(root_chat_id),
        autonomous_no_confirm,
        auto_approve_editing_tools,
        auto_approve_dangerous_commands,
        ..Default::default()
    };

    let (confirmations, denials) = crate::chat::tools::check_tools_confirmation(
        app.clone(),
        &allowed,
        &messages,
        &thread,
        None,
        mode_id,
        Some(model_id),
        &[],
        "subagent",
    )
    .await;
    for denial in denials {
        allowed.retain(|call| call.id != denial.tool_call_id);
        denied_msgs.push(ChatMessage {
            role: "tool".into(),
            tool_call_id: denial.tool_call_id,
            tool_failed: Some(true),
            content: ChatContent::SimpleText(format!("Denied by policy: {}", denial.rule)),
            ..Default::default()
        });
    }
    for confirmation in confirmations {
        if let Some(call) = allowed
            .iter()
            .find(|call| call.id == confirmation.tool_call_id)
            .cloned()
        {
            let result =
                crate::agents::approval::request(&app, &ccx, &call, &confirmation.rule).await;
            if !matches!(result, Ok(true)) {
                allowed.retain(|candidate| candidate.id != call.id);
                denied_msgs.push(ChatMessage {
                    role: "tool".into(),
                    tool_call_id: call.id,
                    tool_failed: Some(true),
                    content: ChatContent::SimpleText(
                        result
                            .err()
                            .unwrap_or_else(|| "Command approval rejected or expired".into()),
                    ),
                    ..Default::default()
                });
            }
        }
    }

    if let Some(tx_toolid) = &tx_toolid_mb {
        let subchat_tx = ccx.lock().await.subchat_tx.clone();
        let context_files = get_context_files_from_messages(&messages);
        for tc in &allowed {
            let args_truncated = truncate_args(&tc.function.arguments, 200);
            let progress_msg = format!(
                "{}/{}: {}({})",
                step_idx + 1,
                max_steps,
                tc.function.name,
                args_truncated
            );
            let tool_msg = json!({
                "tool_call_id": tx_toolid,
                "subchat_id": progress_msg,
                "attached_files": context_files
            });
            let _ = subchat_tx.lock().await.send(tool_msg);
        }
    }

    let (mut tool_results, _) = execute_tools(
        app,
        &allowed,
        &messages,
        &thread,
        &thread.mode,
        Some(&thread.model),
        ExecuteToolsOptions {
            native_pause,
            native_abort: Some(abort_flag.clone()),
            execution_context: Some(ccx.clone()),
            ..Default::default()
        },
    )
    .await;

    if let Some(callback) = &step_progress {
        callback(SubchatProgress::ToolsFinished);
    }

    // A cancelled dispatch deliberately leaves unstarted calls without a
    // fabricated success result; the enclosing subchat observes the same
    // abort flag and exits rather than continuing this turn.
    let dispatch_aborted = abort_flag.load(Ordering::Relaxed);
    for tc in &tool_calls {
        let answered = denied_msgs
            .iter()
            .chain(tool_results.iter())
            .any(|m| m.tool_call_id == tc.id);
        if !answered && !dispatch_aborted {
            tool_results.push(ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                tool_call_id: tc.id.clone(),
                tool_failed: Some(false),
                content: ChatContent::SimpleText("Tool executed with no output.".to_string()),
                ..Default::default()
            });
        }
    }

    messages.extend(denied_msgs);
    messages.extend(tool_results);
    drain_background_agent_inbox(&ccx, &mut messages).await;

    if let Some(tx_toolid) = &tx_toolid_mb {
        let subchat_tx = ccx.lock().await.subchat_tx.clone();
        let context_files = get_context_files_from_messages(&messages);
        if !context_files.is_empty() {
            let tool_msg = json!({
                "tool_call_id": tx_toolid,
                "subchat_id": "/tool:files",
                "attached_files": context_files
            });
            let _ = subchat_tx.lock().await.send(tool_msg);
        }
    }

    Ok(messages)
}

pub(crate) fn stable_stream_chat_id(
    context_chat_id: &str,
    fresh_id: impl FnOnce() -> String,
) -> String {
    let context_chat_id = context_chat_id.trim();
    if context_chat_id.is_empty() {
        fresh_id()
    } else {
        context_chat_id.to_string()
    }
}

pub(crate) async fn subchat_stream(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model_id: &str,
    mode_id: &str,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolDesc>,
    prepend_system_prompt: bool,
    temperature: Option<f32>,
    max_new_tokens: usize,
    reasoning_effort: Option<ReasoningEffort>,
    cache_control: CacheControl,
    only_deterministic_messages: bool,
    progress_tool_call_id: Option<&str>,
    allow_provider_retries: bool,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    let (
        gcx,
        effective_n_ctx,
        abort_flag,
        activity_stamp,
        task_meta,
        worktree,
        context_chat_id,
        context_root_id,
    ) = {
        let cgcx = ccx.lock().await;
        (
            cgcx.global_context.clone(),
            cgcx.n_ctx,
            cgcx.abort_flag.clone(),
            cgcx.activity_stamp.clone(),
            cgcx.task_meta.clone(),
            cgcx.execution_scope_worktree(),
            cgcx.chat_id.clone(),
            cgcx.root_chat_id.clone(),
        )
    };

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("no caps: {:?}", e))?;
    let model_rec = resolve_chat_model(caps, model_id)?;

    let tokenizer_arc = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let t = HasTokenizerAndEot::new(tokenizer_arc);

    let capped_n_ctx = if model_rec.base.n_ctx > 0 {
        effective_n_ctx.min(model_rec.base.n_ctx)
    } else {
        effective_n_ctx
    };

    let stream_chat_id = stable_stream_chat_id(&context_chat_id, || Uuid::new_v4().to_string());
    let stream_root_chat_id = Some(context_root_id).filter(|id| !id.trim().is_empty());

    let meta = ChatMeta {
        chat_id: stream_chat_id.clone(),
        chat_mode: mode_id.to_string(),
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: Some(capped_n_ctx),
        include_project_info: true,
        request_attempt_id: Uuid::new_v4().to_string(),
        worktree: worktree.clone(),
    };

    let thread = ThreadParams {
        id: stream_chat_id,
        model: model_id.to_string(),
        mode: mode_id.to_string(),
        context_tokens_cap: Some(capped_n_ctx),
        task_meta,
        worktree,
        root_chat_id: stream_root_chat_id,
        ..Default::default()
    };

    let mut parameters = SamplingParameters {
        max_new_tokens,
        temperature,
        n: Some(1),
        reasoning_effort,
        ..Default::default()
    };

    let options = ChatPrepareOptions {
        prepend_system_prompt,
        allow_at_commands: false,
        allow_tool_prerun: false,
        supports_tools: model_rec.supports_tools,
        cache_control,
        ..Default::default()
    };

    if only_deterministic_messages {
        return Ok(vec![messages]);
    }

    let messages_count = messages.len();
    let tools_count = tools.len();
    let mode_for_stats = canonicalize_mode_for_stats(mode_id);

    let (
        stats_chat_id,
        stats_root_chat_id,
        stats_task_id,
        stats_task_role,
        stats_agent_id,
        stats_card_id,
    ) = {
        let cgcx = ccx.lock().await;
        let tm = cgcx.task_meta.as_ref();
        (
            cgcx.chat_id.clone(),
            cgcx.root_chat_id.clone(),
            tm.map(|t| t.task_id.clone()),
            tm.map(|t| t.role.clone()),
            tm.and_then(|t| t.agent_id.clone()),
            tm.and_then(|t| t.card_id.clone()),
        )
    };

    let prepared = prepare_chat_passthrough(
        gcx.clone(),
        ccx.clone(),
        &t,
        messages.clone(),
        &thread,
        model_id,
        mode_id,
        tools,
        &meta,
        &mut parameters,
        &options,
    )
    .await?;

    let t1 = std::time::Instant::now();
    let llm_request = prepared.llm_request;

    let progress_sender: Option<mpsc::UnboundedSender<Value>> = if progress_tool_call_id.is_some() {
        let subchat_tx_arc = ccx.lock().await.subchat_tx.clone();
        let x = Some(subchat_tx_arc.lock().await.clone());
        x
    } else {
        None
    };
    let progress_tool_call_id = progress_tool_call_id.map(|s| s.to_string());

    let mut attempt = 0usize;
    let mut last_retry_reason: Option<String> = None;
    let results = loop {
        attempt += 1;
        let params = StreamRunParams {
            llm_request: llm_request.clone(),
            model_rec: model_rec.base.clone(),
            chat_id: Some(stats_chat_id.clone()),
            allow_websocket: true,
            abort_flag: Some(abort_flag.clone()),
            abort_notify: None,
            supports_tools: model_rec.supports_tools,
            supports_reasoning: model_rec.has_reasoning_support(),
            reasoning_type: model_rec.reasoning_type_string(),
            supports_temperature: model_rec.supports_temperature,
            priority: refact_providers::pressure::RequestPriority::Background,
        };

        let mut collector = SubchatProgressCollector::new(
            progress_sender.clone(),
            progress_tool_call_id.clone(),
            activity_stamp.clone(),
        );

        let call_ts_start = chrono::Utc::now().to_rfc3339();
        let call_start = std::time::Instant::now();
        let mut attempt_result: Result<Vec<_>, _> = run_llm_stream(
            AppState::from_gcx(gcx.clone()).await,
            params,
            &mut collector,
        )
        .await
        .and_then(|o| match o {
            crate::chat::stream_core::LlmStreamOutcome::Choices(c) => Ok(c),
            crate::chat::stream_core::LlmStreamOutcome::PausedForCacheGuard => {
                Err(crate::chat::stream_core::LlmStreamError {
                    message: "subchat generation paused by cache guard; retry".to_string(),
                    partial_output_emitted: false,
                    pressure_failure: None,
                })
            }
        });
        let duration_ms = call_start.elapsed().as_millis() as u64;
        let call_ts_end = chrono::Utc::now().to_rfc3339();

        let (provider, model_short) = split_model_provider(model_id);
        let mut monitor_error: Option<String> = None;

        match &mut attempt_result {
            Err(error) => {
                let retry_decision = error.retry_decision();
                let retry_reason = retry_decision
                    .is_retryable_transient()
                    .then(|| retry_decision.reason().to_string());
                let retry_attempt = attempt.saturating_sub(1);
                let pressure_retry = if allow_provider_retries
                    && !retry_decision.is_user_cancelled()
                    && !retry_decision.is_context_limit()
                    && !abort_flag.load(Ordering::SeqCst)
                {
                    crate::llm::pressure::pressure_retry(
                        error,
                        u32::try_from(retry_attempt).unwrap_or(u32::MAX),
                    )
                } else {
                    None
                };
                let should_retry = allow_provider_retries
                    && error.should_retry(retry_attempt, &abort_flag)
                    && pressure_retry.is_none();
                let will_retry = should_retry
                    || matches!(
                        pressure_retry.as_ref(),
                        Some(crate::llm::pressure::PressureRetry::Retry { .. })
                    );
                if error.partial_output_emitted
                    && !will_retry
                    && !retry_decision.is_context_limit()
                    && !abort_flag.load(Ordering::SeqCst)
                {
                    let original = error.message.clone();
                    let safe_error = partial_output_stream_error_message(&original);
                    warn!("{}", safe_error);
                    error.message = safe_error;
                    monitor_error = Some(error.message.clone());
                }
                let event = LlmCallEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    ts_start: call_ts_start,
                    ts_end: call_ts_end,
                    duration_ms,
                    chat_id: stats_chat_id.clone(),
                    root_chat_id: Some(stats_root_chat_id.clone()),
                    mode: mode_for_stats.clone(),
                    task_id: stats_task_id.clone(),
                    task_role: stats_task_role.clone(),
                    agent_id: stats_agent_id.clone(),
                    card_id: stats_card_id.clone(),
                    model_id: model_id.to_string(),
                    provider,
                    model: model_short,
                    messages_count,
                    tools_count,
                    max_tokens: max_new_tokens,
                    temperature,
                    success: false,
                    error_message: Some(error.message.chars().take(200).collect()),
                    finish_reason: None,
                    attempt_n: attempt,
                    retry_reason: retry_reason.clone(),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    total_tokens: 0,
                    cost_usd: None,
                };
                if let Some(sender) = &*gcx.llm_stats_sender.lock().unwrap() {
                    if sender.try_send(event).is_err() {
                        tracing::warn!("stats: channel full, dropping LLM call event");
                    }
                }

                if should_retry {
                    let delay = retry_delay_for_attempt(retry_attempt);
                    let retry_reason_for_log = retry_reason.as_deref().unwrap_or("retryable_error");
                    last_retry_reason = Some(retry_reason_for_log.to_string());
                    warn!(
                        "Retrying subchat generation after retryable LLM error in {}s (attempt {}/{}, reason={})",
                        delay.as_secs(),
                        attempt,
                        MAX_LLM_RETRY_ATTEMPTS,
                        retry_reason_for_log,
                    );
                    if sleep_or_abort(delay, abort_flag.clone()).await {
                        break Err(LlmStreamError::from("aborted".to_string()));
                    }
                    continue;
                }
                if let Some(crate::llm::pressure::PressureRetry::Retry { delay_ms, .. }) =
                    pressure_retry.as_ref()
                {
                    last_retry_reason = Some(
                        retry_reason
                            .as_deref()
                            .unwrap_or("provider_pressure_retry")
                            .to_string(),
                    );
                    if sleep_or_abort(
                        std::time::Duration::from_millis(*delay_ms),
                        abort_flag.clone(),
                    )
                    .await
                    {
                        break Err(LlmStreamError::from("aborted".to_string()));
                    }
                    continue;
                }
                if let Some(crate::llm::pressure::PressureRetry::GiveUp { cause }) =
                    pressure_retry.as_ref()
                {
                    error.message = format!("{} (not retried: {})", error.message, cause);
                }
            }
            Ok(ref results_ok) => {
                let usage = results_ok.first().and_then(|r| r.usage.as_ref());
                let event = LlmCallEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    ts_start: call_ts_start,
                    ts_end: call_ts_end,
                    duration_ms,
                    chat_id: stats_chat_id.clone(),
                    root_chat_id: Some(stats_root_chat_id.clone()),
                    mode: mode_for_stats.clone(),
                    task_id: stats_task_id.clone(),
                    task_role: stats_task_role.clone(),
                    agent_id: stats_agent_id.clone(),
                    card_id: stats_card_id.clone(),
                    model_id: model_id.to_string(),
                    provider,
                    model: model_short,
                    messages_count,
                    tools_count,
                    max_tokens: max_new_tokens,
                    temperature,
                    success: true,
                    error_message: None,
                    finish_reason: results_ok.first().and_then(|r| r.finish_reason.clone()),
                    attempt_n: attempt,
                    retry_reason: last_retry_reason.clone(),
                    prompt_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                    completion_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
                    cache_read_tokens: usage.and_then(|u| u.cache_read_tokens),
                    cache_creation_tokens: usage.and_then(|u| u.cache_creation_tokens),
                    total_tokens: usage.map(|u| u.total_tokens).unwrap_or(0),
                    cost_usd: usage
                        .and_then(|u| u.metering_usd.as_ref())
                        .map(|m| m.total_usd),
                };
                if let Some(sender) = &*gcx.llm_stats_sender.lock().unwrap() {
                    if sender.try_send(event).is_err() {
                        tracing::warn!("stats: channel full, dropping LLM call event");
                    }
                }
            }
        }

        if let Some(error_message) = monitor_error {
            if let Some(task_meta) = thread.task_meta.clone() {
                crate::chat::task_agent_monitor::handle_agent_streaming_error(
                    AppState::from_gcx(gcx.clone()).await,
                    &task_meta,
                    &error_message,
                )
                .await;
            }
        }

        break attempt_result;
    }?;

    info!(
        "stream generation took {:?}ms",
        t1.elapsed().as_millis() as i32
    );

    convert_results_to_messages(results, messages)
}

fn convert_results_to_messages(
    results: Vec<ChoiceFinal>,
    original_messages: Vec<ChatMessage>,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    if results.is_empty() {
        return Err("subchat model returned no output (empty choices)".to_string());
    }

    let mut all_choices = vec![];
    let mut skipped_empty_finish_reason: Option<String> = None;
    for result in results {
        let tool_calls: Option<Vec<_>> = if result.tool_calls_raw.is_empty() {
            None
        } else {
            let parsed: Vec<_> = result
                .tool_calls_raw
                .iter()
                .filter_map(|tc| normalize_tool_call(tc))
                .collect();
            if parsed.is_empty() {
                None
            } else {
                Some(parsed)
            }
        };

        if tool_calls.is_none() && result.content.trim().is_empty() {
            skipped_empty_finish_reason = Some(
                result
                    .finish_reason
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            continue;
        }

        let msg = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(result.content),
            tool_calls,
            reasoning_content: if result.reasoning.is_empty() {
                None
            } else {
                Some(result.reasoning)
            },
            thinking_blocks: if result.thinking_blocks.is_empty() {
                None
            } else {
                Some(result.thinking_blocks)
            },
            citations: result.citations,
            finish_reason: result.finish_reason,
            usage: result.usage,
            extra: result.extra,
            ..Default::default()
        };

        let mut extended = original_messages.clone();
        extended.push(msg);
        all_choices.push(extended);
    }

    if all_choices.is_empty() {
        return Err(format!(
            "{EMPTY_CHOICE_ERROR_PREFIX} (finish_reason: {})",
            skipped_empty_finish_reason.unwrap_or_else(|| "unknown".to_string())
        ));
    }

    Ok(all_choices)
}

fn update_usage_from_messages(usage: &mut ChatUsage, messages: &[Vec<ChatMessage>]) {
    if let Some(message_0) = messages.first() {
        if let Some(last_message) = message_0.last() {
            if let Some(u) = last_message.usage.as_ref() {
                usage.total_tokens += u.total_tokens;
                usage.completion_tokens += u.completion_tokens;
                usage.prompt_tokens += u.prompt_tokens;
                if let Some(cache_creation) = u.cache_creation_tokens {
                    *usage.cache_creation_tokens.get_or_insert(0) += cache_creation;
                }
                if let Some(cache_read) = u.cache_read_tokens {
                    *usage.cache_read_tokens.get_or_insert(0) += cache_read;
                }
            }
        }
    }
}

fn aggregate_metering_from_messages(
    _messages: &[ChatMessage],
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::new()
}

async fn subchat_single_internal(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model_id: &str,
    mode_id: &str,
    messages: Vec<ChatMessage>,
    tools_subset: Option<Vec<String>>,
    only_deterministic_messages: bool,
    temperature: Option<f32>,
    max_new_tokens: usize,
    reasoning_effort: Option<ReasoningEffort>,
    cache_control: CacheControl,
    prepend_system_prompt: bool,
    progress_tool_call_id: Option<&str>,
    allow_provider_retries: bool,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    let gcx = {
        let cgcx = ccx.lock().await;
        cgcx.global_context.clone()
    };

    let tools: Vec<ToolDesc> = if tools_subset
        .as_ref()
        .is_some_and(|subset| subset.is_empty())
    {
        vec![]
    } else {
        let mut tools_turned_on_by_cmdline = get_available_tools(gcx.clone())
            .await
            .into_iter()
            .map(|tool| tool.tool_description())
            .collect::<Vec<_>>();
        let (app, chat_id) = {
            let context = ccx.lock().await;
            (context.app.clone(), context.chat_id.clone())
        };
        let has_parent = crate::tools::tools_list::chat_has_parent_agent(&app, &chat_id).await;
        crate::tools::tools_list::filter_chat_tools(&mut tools_turned_on_by_cmdline, has_parent);
        let policy = crate::tools::tools_list::tool_access_policy(gcx.clone()).await;
        if !policy.tool_access.providers.is_empty() {
            let provider = crate::tools::tools_list::provider_of_model(model_id);
            tools_turned_on_by_cmdline
                .retain(|desc| crate::tools::tools_list::mcp_tool_allowed(&policy, provider, desc));
        }

        match tools_subset.as_ref() {
            Some(subset) => tools_turned_on_by_cmdline
                .into_iter()
                .filter(|tool| subset.contains(&tool.name))
                .collect(),
            None => tools_turned_on_by_cmdline,
        }
    };

    subchat_stream(
        ccx.clone(),
        model_id,
        mode_id,
        messages,
        tools,
        prepend_system_prompt,
        temperature,
        max_new_tokens,
        reasoning_effort,
        cache_control,
        only_deterministic_messages,
        progress_tool_call_id,
        allow_provider_retries,
    )
    .await
}
#[cfg(test)]
mod subchat_tests {
    use super::{
        apply_subchat_reactive_compaction, apply_subchat_report_policy,
        emit_parent_compaction_diagnostics, gate_subchat_boundary,
        parent_compaction_diagnostic_status, parent_thread_worktree, parse_subchat_cache_control,
        partial_output_stream_error_message, prepare_subchat_messages,
        resolve_subchat_config_with_parent, resolve_subagent_confirmation_defaults,
        resolve_subchat_model, resolve_subchat_params, resolve_subchat_worktree,
        safe_context_limit_error_for_log, stable_stream_chat_id, stable_subchat_chat_id,
        should_compact_context_limit_error, should_persist_subchat_trajectory,
        stateful_thread_from_config, subchat_retries_allowed, subchat_trajectory_commit_intent,
        trace_thread_from_config, save_failed_subchat_trajectory, SubchatConfig, SubchatProgress,
        SubchatProgressCollector, SubchatTrajectoryCommitPhase, ToolsPolicy, TraceParent,
        GUARDED_REPORT_INSTRUCTION, PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS,
        PARENT_COMPACTION_DIAGNOSTIC_REDACTION_LOOKAHEAD_CHARS,
        PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED, PARTIAL_OUTPUT_STREAM_ERROR,
    };
    use super::{final_step_wrap_up_message, needs_forced_final_answer};
    use crate::chat::diagnostics::{
        SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS, SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED,
    };

    use crate::chat::trajectory_ops::sanitize_messages_for_new_thread;
    use crate::chat::trajectories::save_trajectory_as_with_intent;
    use crate::call_validation::{
        ChatContent, ChatMessage, ChatModelType, ReasoningEffort, SubchatParameters,
    };
    use crate::chat::types::{TaskMeta, ThreadParams, TrajectoryCommitIntent};
    use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
    use crate::global_context::tests::make_test_gcx;
    use crate::llm::params::CacheControl;
    use crate::worktrees::types::WorktreeMeta;
    use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;
    use refact_privacy::{
        Attribution, FileRecord, PolicyLoad, PrivacyPolicy, PrivacyRecord, ShellBehavior,
        SubagentPolicy, Zone,
    };
    use crate::chat::stream_core::StreamCollector;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn chat_model_record(id: &str, n_ctx: usize, endpoint: &str) -> Arc<ChatModelRecord> {
        Arc::new(ChatModelRecord {
            base: BaseModelRecord {
                id: id.to_string(),
                name: id.to_string(),
                n_ctx,
                endpoint: endpoint.to_string(),
                ..Default::default()
            },
            max_output_tokens: Some(128_000),
            ..Default::default()
        })
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    #[tokio::test]
    async fn runner_preempt_wait_observes_queued_a_without_aborting_runner() {
        let ccx = abort_test_ccx().await;
        let app = ccx.lock().await.app.clone();
        let request = crate::agents::types::CreateAgentRequest {
            parent_chat_id: "parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: crate::agents::types::BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "delivery test".into(),
            prompt: "test".into(),
            target_files: vec![],
            model: "test".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        };
        let (agent, _, _) = app.agents.create(request).await.unwrap();
        ccx.lock().await.background_agent_id = Some(agent.agent_id.clone());
        for push in [
            refact_chat_api::PushMode::WhenIdle,
            refact_chat_api::PushMode::Append,
            refact_chat_api::PushMode::Preempt,
        ] {
            app.agents
                .enqueue_delivery(
                    &agent.agent_id,
                    refact_chat_api::PendingDelivery::new(
                        vec![ChatMessage::new("user".into(), push.as_str().into())],
                        push,
                        "test",
                        true,
                    ),
                )
                .await
                .unwrap();
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            super::wait_for_runner_preempt(&app, &agent.agent_id),
        )
        .await
        .unwrap();
        assert!(!ccx
            .lock()
            .await
            .abort_flag
            .load(std::sync::atomic::Ordering::SeqCst));
        let mut messages = vec![];
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        let pending = app.agents.pending_deliveries(&agent.agent_id).await;
        assert_eq!(pending.len(), 3); // Claimed A/B remain durable until trajectory ack.
        assert_eq!(pending[0].push, refact_chat_api::PushMode::WhenIdle);
        assert!(!app.agents.has_preempt(&agent.agent_id).await);
        assert_eq!(messages.len(), 3);
        let mut config = test_subchat_config();
        config.stateful = true;
        config.background_agent_id = Some(agent.agent_id.clone());
        let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gcx = ccx.lock().await.global_context.clone();
        assert_eq!(
            crate::chat::trajectories::get_trajectories_dir(gcx.clone())
                .await
                .unwrap_err(),
            "No workspace folder found"
        );
        super::persist_subchat_progress(&ccx, &config, &progress, &messages).await;
        assert_eq!(
            app.agents.pending_deliveries(&agent.agent_id).await.len(),
            3
        );
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        super::persist_subchat_progress(&ccx, &config, &progress, &messages).await;
        let pending = app.agents.pending_deliveries(&agent.agent_id).await;
        assert_eq!(
            pending.len(),
            1,
            "required trajectory commit acknowledges A/B only"
        );
        assert_eq!(pending[0].push, refact_chat_api::PushMode::WhenIdle);
        let (gcx, chat_id) = {
            let context = ccx.lock().await;
            (context.global_context.clone(), context.chat_id.clone())
        };
        let saved = crate::chat::trajectories::load_trajectory_for_chat(gcx, &chat_id)
            .await
            .unwrap();
        assert_eq!(saved.messages.len(), messages.len());
    }

    #[tokio::test]
    async fn runner_wait_boundary_releases_idle_after_all_results_and_local_ticks() {
        use refact_chat_api::{PendingDelivery, PushMode};
        let ccx = abort_test_ccx().await;
        let (app, chat_id) = {
            let ccx = ccx.lock().await;
            (ccx.app.clone(), ccx.chat_id.clone())
        };
        let (agent, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "parent".into(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: crate::agents::types::BgAgentKind::Subagent,
                config_name: "subagent".into(),
                title: "wait boundary test".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "test".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        ccx.lock().await.background_agent_id = Some(agent.agent_id.clone());
        let delivery = PendingDelivery::with_id(
            "runner-idle",
            vec![ChatMessage::new("user".into(), "continue".into())],
            PushMode::WhenIdle,
            "parent",
            true,
        );
        app.agents
            .enqueue_delivery(&agent.agent_id, delivery.clone())
            .await
            .unwrap();
        let mut session = crate::chat::types::ChatSession::new(chat_id.clone());
        session.set_runner_pending_deliveries(vec![delivery]);
        let session = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id, session.clone());
        let mut messages = vec![];
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert!(messages.is_empty(), "ordinary runner boundaries retain C");

        let assistant: ChatMessage = serde_json::from_value(serde_json::json!({
            "role": "assistant", "content": "", "tool_calls": [
                {"id":"sleep", "type":"function", "function":{"name":"wait", "arguments":"{\"target\":{\"kind\":\"duration\",\"duration_ms\":30000},\"description\":\"Wait briefly\"}"}},
                {"id":"other", "type":"function", "function":{"name":"wait", "arguments":"{\"target\":{\"kind\":\"duration\",\"duration_ms\":30000},\"description\":\"Wait briefly\"}"}}
            ]
        }))
        .unwrap();
        messages.push(assistant);
        {
            let mut session = session.lock().await;
            session.messages = messages.clone();
            session.turn_depth = 1;
            session.interruptible_waits = 1;
            session.interrupt_wait_for_delivery();
            assert!(session.wait_delivery_boundary);
            assert!(!session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
            session.interruptible_waits = 0;
            session
                .queue_post_tool_delivery(PendingDelivery::with_id(
                    "local-tick",
                    vec![ChatMessage::new("event".into(), "tick".into())],
                    PushMode::WhenIdle,
                    "sleep",
                    false,
                ))
                .unwrap();
        }
        for id in ["sleep", "other"] {
            super::drain_background_agent_inbox(&ccx, &mut messages).await;
            assert!(session.lock().await.wait_delivery_boundary);
            assert!(!messages.iter().any(|message| message.role == "user"));
            messages.push(ChatMessage {
                role: "tool".into(),
                tool_call_id: id.into(),
                content: ChatContent::SimpleText("yielded".into()),
                ..Default::default()
            });
        }
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert!(!session.lock().await.wait_delivery_boundary);
        assert!(session.lock().await.pending_deliveries.is_empty());
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[3].extra["delivery"]["id"], "runner-idle");
        assert_eq!(messages[4].extra["delivery"]["id"], "local-tick");
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert_eq!(
            messages.len(),
            5,
            "claimed deliveries must not be duplicated"
        );
        app.agents
            .enqueue_delivery(
                &agent.agent_id,
                PendingDelivery::with_id(
                    "later-idle",
                    vec![ChatMessage::new("user".into(), "later".into())],
                    PushMode::WhenIdle,
                    "parent",
                    true,
                ),
            )
            .await
            .unwrap();
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert_eq!(messages.len(), 5, "wait boundary is consumed once");

        session.lock().await.wait_delivery_boundary = true;
        ccx.lock().await.background_agent_id = Some("missing-agent".into());
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert!(
            session.lock().await.wait_delivery_boundary,
            "failed drains retain the boundary"
        );
    }

    #[tokio::test]
    async fn runner_local_sleep_deliveries_follow_result_and_survive_ack() {
        use refact_chat_api::{PendingDelivery, PushMode};
        let ccx = abort_test_ccx().await;
        let app = ccx.lock().await.app.clone();
        let request = crate::agents::types::CreateAgentRequest {
            parent_chat_id: "parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: crate::agents::types::BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "delivery test".into(),
            prompt: "test".into(),
            target_files: vec![],
            model: "test".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        };
        let (agent, _, _) = app.agents.create(request).await.unwrap();
        ccx.lock().await.background_agent_id = Some(agent.agent_id.clone());
        let chat_id = ccx.lock().await.chat_id.clone();
        let assistant: ChatMessage = serde_json::from_value(serde_json::json!({
            "role": "assistant", "content": "", "tool_calls": [
                {"id":"sleep", "type":"function", "function":{"name":"wait", "arguments":"{\"target\":{\"kind\":\"duration\",\"duration_ms\":30000},\"description\":\"Wait briefly\"}"}}
            ]
        }))
        .unwrap();
        let mut messages = vec![assistant.clone()];
        let mut session = crate::chat::types::ChatSession::new(chat_id.clone());
        session.messages = messages.clone();
        session.turn_depth = 1;
        let mut originals = Vec::new();
        for push in [PushMode::Preempt, PushMode::Append, PushMode::WhenIdle] {
            let delivery = PendingDelivery::with_id(
                push.as_str(),
                vec![ChatMessage::new("event".into(), "sleep tick".into())],
                push,
                "sleep",
                false,
            );
            assert_eq!(
                session.queue_post_tool_delivery(delivery).unwrap(),
                refact_chat_api::DeliveryOutcome::Queued
            );
            if push == PushMode::Append {
                session
                    .pending_deliveries
                    .back_mut()
                    .unwrap()
                    .after_tool_call_id = Some(String::new());
            }
            originals.push(session.pending_deliveries.back().unwrap().clone());
        }
        let serialized = serde_json::to_value(session.pending_deliveries_for_snapshot()).unwrap();
        session.pending_deliveries.clear();
        session.restore_pending_deliveries(serde_json::from_value(serialized).unwrap());
        session.set_runner_pending_deliveries(vec![PendingDelivery::with_id(
            "mirror-only",
            vec![ChatMessage::new("event".into(), "not input".into())],
            PushMode::Append,
            "display",
            false,
        )]);
        let session = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert_eq!(messages.len(), 1);
        assert!(!app.agents.has_preempt(&agent.agent_id).await);
        assert_eq!(session.lock().await.pending_deliveries.len(), 3);
        messages.push(ChatMessage {
            role: "tool".into(),
            tool_call_id: "sleep".into(),
            content: ChatContent::SimpleText("slept".into()),
            ..Default::default()
        });
        super::import_runner_local_deliveries(&app, "missing-agent", &chat_id, &messages).await;
        assert_eq!(session.lock().await.pending_deliveries.len(), 3);
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert!(session.lock().await.pending_deliveries.is_empty());
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[1].role, "tool");
        assert_eq!(messages.len(), 5);
        assert_eq!(
            messages[3].extra["delivery"]["id"],
            PushMode::Preempt.as_str()
        );
        assert_eq!(
            messages[4].extra["delivery"]["id"],
            PushMode::Append.as_str()
        );
        assert_eq!(
            app.agents.pending_deliveries(&agent.agent_id).await.len(),
            3
        );
        session
            .lock()
            .await
            .pending_deliveries
            .extend(originals.clone());
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert_eq!(messages.len(), 5);
        let mut config = test_subchat_config();
        config.stateful = true;
        config.background_agent_id = Some(agent.agent_id.clone());
        let workspace = tempfile::tempdir().unwrap();
        let gcx = ccx.lock().await.global_context.clone();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let progress = Arc::new(StdMutex::new(Vec::new()));
        super::persist_subchat_progress(&ccx, &config, &progress, &messages).await;
        assert_eq!(
            app.agents.pending_deliveries(&agent.agent_id).await.len(),
            1
        );
        session.lock().await.pending_deliveries.extend(originals);
        super::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert_eq!(messages.len(), 5);
        messages.push(ChatMessage::new("assistant".into(), "final".into()));
        let deliveries = app
            .agents
            .finish_delivery_turn(&agent.agent_id)
            .await
            .unwrap();
        assert!(!super::append_runner_deliveries(&mut messages, deliveries));
        assert_eq!(messages[5].content.content_text_only(), "final");
        assert_eq!(
            messages[6].extra["delivery"]["id"],
            PushMode::WhenIdle.as_str()
        );
        for (index, message) in messages.iter_mut().enumerate() {
            if message.message_id.is_empty() {
                message.message_id = format!("runner-sleep-message-{index}");
            }
            if let Some(calls) = message.tool_calls.as_mut() {
                for (call_index, call) in calls.iter_mut().enumerate() {
                    call.index = Some(call_index);
                }
            }
        }
        super::persist_subchat_progress(&ccx, &config, &progress, &messages).await;
        assert!(app
            .agents
            .pending_deliveries(&agent.agent_id)
            .await
            .is_empty());
        let gcx = ccx.lock().await.global_context.clone();
        let saved = crate::chat::trajectories::load_trajectory_for_chat(gcx, &chat_id)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(saved.messages).unwrap(),
            serde_json::to_value(messages).unwrap()
        );
    }

    #[test]
    fn runner_delivery_preserves_prefix_and_puts_cancellation_before_preempt() {
        let original = ChatMessage::new("assistant".into(), "completed answer".into());
        let mut messages = vec![original.clone()];
        let delivery = refact_chat_api::PendingDelivery::with_id(
            "urgent",
            vec![ChatMessage::new("user".into(), "new instruction".into())],
            refact_chat_api::PushMode::Preempt,
            "user",
            true,
        );
        assert!(super::append_runner_deliveries(
            &mut messages,
            vec![delivery]
        ));
        assert_eq!(
            serde_json::to_value(&messages[0]).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(messages[1].extra["event"]["source"], "runner");
        assert_eq!(messages[1].extra["event"]["subkind"], "cancellation_note");
        assert_eq!(messages[1].extra["event"]["payload"]["source"], "user");
        assert_eq!(messages[2].extra["delivery"]["id"], "urgent");
        assert!(!super::has_final_answer(&messages));
    }

    #[test]
    fn runner_idle_delivery_wakes_only_when_requested() {
        for wake in [false, true] {
            let mut messages = vec![ChatMessage::new("assistant".into(), "done".into())];
            let delivery = refact_chat_api::PendingDelivery::with_id(
                "idle",
                vec![ChatMessage::new("event".into(), "notice".into())],
                refact_chat_api::PushMode::WhenIdle,
                "test",
                wake,
            );
            assert_eq!(
                super::append_runner_deliveries(&mut messages, vec![delivery]),
                wake
            );
            assert_eq!(messages.len(), 2);
            assert!(!super::has_final_answer(&messages));
        }
    }

    #[test]
    fn runner_boundary_requires_every_parallel_tool_result() {
        let mut assistant = ChatMessage::new("assistant".into(), String::new());
        assistant.tool_calls = Some(vec![
            serde_json::from_value(serde_json::json!({"id":"one", "type":"function", "function":{"name":"cat", "arguments":"{}"}})).unwrap(),
            serde_json::from_value(serde_json::json!({"id":"two", "type":"function", "function":{"name":"cat", "arguments":"{}"}})).unwrap(),
        ]);
        let mut messages = vec![assistant];
        assert!(!super::runner_tool_window_closed(&messages));
        for id in ["one", "two"] {
            let mut result = ChatMessage::new("tool".into(), "result".into());
            result.tool_call_id = id.into();
            messages.push(result);
            assert_eq!(super::runner_tool_window_closed(&messages), id == "two");
        }
    }

    #[test]
    fn runner_boundary_accepts_diff_results_for_edit_tools() {
        let mut assistant = ChatMessage::new("assistant".into(), String::new());
        assistant.tool_calls = Some(vec![serde_json::from_value(serde_json::json!({
            "id":"edit", "type":"function", "function":{"name":"update_textdoc", "arguments":"{}"}
        }))
        .unwrap()]);
        let mut diff = ChatMessage::new("diff".into(), "[]".into());
        diff.tool_call_id = "edit".into();
        let messages = vec![assistant, diff];
        assert!(super::runner_tool_window_closed(&messages));
    }

    #[test]
    fn needs_forced_final_answer_gating() {
        assert!(needs_forced_final_answer(true, false, false));
        assert!(!needs_forced_final_answer(false, false, false));
        assert!(!needs_forced_final_answer(true, true, false));
        assert!(!needs_forced_final_answer(true, false, true));
    }

    fn test_subchat_config() -> SubchatConfig {
        SubchatConfig {
            history_source_model: None,
            tool_name: "subagent".to_string(),
            stateful: false,
            autonomous_no_confirm: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            chat_id: None,
            title: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            tools: ToolsPolicy::None,
            max_steps: 1,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: None,
            worktree: None,
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            implicit_context_window: false,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            soft_abort: false,
            activity_stamp: None,
            background_agent_id: None,
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: TraceParent::unattributed(),
        }
    }

    #[test]
    fn completed_child_history_preserves_signatures_only_for_the_same_model() {
        let mut message = ChatMessage::new("assistant".into(), "completed answer".into());
        message.thinking_blocks = Some(vec![
            serde_json::json!({"type":"thinking", "thinking":"reasoning", "signature":"opaque-unaltered"}),
        ]);
        message.extra.insert(
            "launch_context_source".into(),
            serde_json::json!({"type":"completed_prefix","digest":"original"}),
        );
        let mut config = test_subchat_config();
        config.history_source_model = Some(config.model.clone());
        let same = super::prepare_child_history(&[message.clone()], &config);
        assert_eq!(same[0].thinking_blocks, message.thinking_blocks);
        config.model = "different/model".into();
        let changed = super::prepare_child_history(&[message.clone()], &config);
        assert!(changed[0].thinking_blocks.is_none());
        assert_eq!(
            changed[0].extra["launch_context_source"],
            message.extra["launch_context_source"]
        );
    }

    async fn abort_test_ccx(
    ) -> Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>> {
        let gcx = make_test_gcx().await;
        Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new(
                gcx,
                4096,
                1,
                false,
                vec![],
                "subchat-abort-test".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn aborted_config(soft_abort: bool) -> SubchatConfig {
        let mut config = test_subchat_config();
        config.abort_flag = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        config.soft_abort = soft_abort;
        config
    }

    #[test]
    fn abort_error_detection_matches_stream_and_loop_variants() {
        assert!(super::is_abort_error("Aborted"));
        assert!(super::is_abort_error("aborted"));
        assert!(!super::is_abort_error("aborted by user"));
        assert!(!super::is_abort_error("context length exceeded"));
    }

    #[tokio::test]
    async fn soft_abort_returns_accumulated_messages_with_aborted_flag() {
        let ccx = abort_test_ccx().await;
        let config = aborted_config(true);
        let messages = vec![ChatMessage::new("user".to_string(), "hi".to_string())];
        let mut usage = crate::call_validation::ChatUsage::default();
        let progress = Arc::new(StdMutex::new(Vec::new()));
        let result = super::run_subchat_loop(
            ccx,
            &config,
            messages,
            &ToolsPolicy::None,
            &mut usage,
            &progress,
        )
        .await;
        let (out, aborted) = result.expect("soft abort must return Ok");
        assert!(aborted);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "user");
        assert_eq!(out[0].content.content_text_only(), "hi");
    }

    #[tokio::test]
    async fn hard_abort_still_returns_aborted_error() {
        let ccx = abort_test_ccx().await;
        let config = aborted_config(false);
        let messages = vec![ChatMessage::new("user".to_string(), "hi".to_string())];
        let mut usage = crate::call_validation::ChatUsage::default();
        let progress = Arc::new(StdMutex::new(Vec::new()));
        let result = super::run_subchat_loop(
            ccx,
            &config,
            messages,
            &ToolsPolicy::None,
            &mut usage,
            &progress,
        )
        .await;
        assert_eq!(result.err(), Some("Aborted".to_string()));
    }

    #[tokio::test]
    async fn wrap_up_soft_abort_returns_accumulated_messages() {
        let ccx = abort_test_ccx().await;
        let config = aborted_config(true);
        let wrap_up = super::WrapUpConfig {
            depth: 1,
            tokens_cnt: 1000,
            prompt: "wrap up".to_string(),
        };
        let messages = vec![ChatMessage::new("user".to_string(), "hi".to_string())];
        let mut usage = crate::call_validation::ChatUsage::default();
        let progress = Arc::new(StdMutex::new(Vec::new()));
        let result = super::run_subchat_with_wrap_up(
            ccx,
            &config,
            messages,
            &ToolsPolicy::None,
            &wrap_up,
            &mut usage,
            &progress,
        )
        .await;
        let (out, aborted) = result.expect("soft abort must return Ok");
        assert!(aborted);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content.content_text_only(), "hi");
    }

    #[tokio::test]
    async fn wrap_up_hard_abort_still_returns_aborted_error() {
        let ccx = abort_test_ccx().await;
        let config = aborted_config(false);
        let wrap_up = super::WrapUpConfig {
            depth: 1,
            tokens_cnt: 1000,
            prompt: "wrap up".to_string(),
        };
        let messages = vec![ChatMessage::new("user".to_string(), "hi".to_string())];
        let mut usage = crate::call_validation::ChatUsage::default();
        let progress = Arc::new(StdMutex::new(Vec::new()));
        let result = super::run_subchat_with_wrap_up(
            ccx,
            &config,
            messages,
            &ToolsPolicy::None,
            &wrap_up,
            &mut usage,
            &progress,
        )
        .await;
        assert_eq!(result.err(), Some("Aborted".to_string()));
    }

    /// Real registry runtime bound to a loop context: the loop parks on the
    /// agent's own pause gate and observes the agent's own abort flag.
    async fn paused_agent_loop(
        app: &crate::app_state::AppState,
        tools_policy: ToolsPolicy,
    ) -> (
        String,
        Arc<crate::agents::registry::PauseGate>,
        tokio::sync::mpsc::UnboundedReceiver<SubchatProgress>,
        tokio::task::JoinHandle<Result<(Vec<ChatMessage>, bool), String>>,
    ) {
        let (record, abort_flag, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "parent".into(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: crate::agents::types::BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "paused".into(),
                prompt: "prompt".into(),
                target_files: vec![],
                model: "model".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        let gate = app.agents.pause_gate(&record.agent_id).await.unwrap();
        app.agents.request_pause(&record.agent_id).await.unwrap();
        let mut ccx = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            1,
            false,
            vec![],
            format!("subchat-{}", record.agent_id),
            None,
            "model".to_string(),
            None,
            None,
        )
        .await;
        ccx.background_agent_id = Some(record.agent_id.clone());
        let ccx = Arc::new(tokio::sync::Mutex::new(ccx));
        let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut config = test_subchat_config();
        config.abort_flag = Some(abort_flag);
        config.background_agent_id = Some(record.agent_id.clone());
        config.max_steps = 3;
        config.tools = tools_policy.clone();
        config.step_progress = Some(Arc::new(move |update| {
            let _ = progress_tx.send(update);
        }));
        let task = tokio::spawn(async move {
            let mut usage = crate::call_validation::ChatUsage::default();
            let progress = Arc::new(StdMutex::new(Vec::new()));
            super::run_subchat_loop(
                ccx,
                &config,
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
                &tools_policy,
                &mut usage,
                &progress,
            )
            .await
        });
        (record.agent_id, gate, progress_rx, task)
    }

    #[tokio::test]
    async fn paused_loop_parks_before_any_dispatch_and_cancel_wakes_it_without_effect() {
        let app = crate::app_state::AppState::from_gcx(make_test_gcx().await).await;
        let (agent_id, gate, mut progress, task) = paused_agent_loop(&app, ToolsPolicy::None).await;
        gate.quiesced().await;
        let status = app.agents.pause_status(&agent_id).await.unwrap();
        assert!(status.requested && status.quiescent);
        assert!(!status.running_tool_cancelable);
        assert!(
            progress.try_recv().is_err(),
            "no step may start while parked"
        );
        app.agents
            .cancel("parent", &agent_id, Some("owner cancel".into()))
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().err(), Some("Aborted".to_string()));
        assert!(
            progress.try_recv().is_err(),
            "cancel must not release a dispatch"
        );
        assert!(app.agents.pause_status(&agent_id).await.is_none());
    }

    #[tokio::test]
    async fn resume_wakes_parked_loop_and_next_boundary_is_reached() {
        let app = crate::app_state::AppState::from_gcx(make_test_gcx().await).await;
        // A zero-turn supervised budget makes the first post-barrier check
        // fail closed deterministically, proving the loop moved past the gate
        // without needing a provider.
        let policy = ToolsPolicy::Supervised {
            tools: vec![],
            budget: crate::chat::types::GoalBudget {
                max_turns: Some(0),
                max_tokens: Some(1),
                explicit: true,
                ..Default::default()
            },
        };
        let (agent_id, gate, mut progress, task) = paused_agent_loop(&app, policy).await;
        gate.quiesced().await;
        assert!(progress.try_recv().is_err());
        let status = app.agents.release_pause(&agent_id).await.unwrap();
        assert!(!status.requested);
        assert_eq!(progress.recv().await, Some(SubchatProgress::Step(1)));
        assert_eq!(
            task.await.unwrap().err(),
            Some("Aborted: supervised token/turn budget exhausted".to_string())
        );
        assert!(!gate.is_quiescent());
    }

    #[tokio::test]
    async fn subchat_confirmation_defaults_follow_yaml_settings() {
        let gcx = make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        global_configs_try_create_all(&config_dir).await.unwrap();

        assert_eq!(
            resolve_subagent_confirmation_defaults(gcx.clone(), "subagent").await,
            (true, true, true)
        );
        assert_eq!(
            resolve_subagent_confirmation_defaults(gcx.clone(), "title_generation").await,
            (false, false, false)
        );

        fs::write(
            config_dir.join("subagents/subagent.yaml"),
            "schema_version: 7\nid: subagent\nsubchat:\n  autonomous_no_confirm: false\n  auto_approve_editing_tools: false\n  auto_approve_dangerous_commands: false\n",
        )
        .unwrap();
        crate::yaml_configs::customization_registry::invalidate_all_registry_caches(gcx.clone())
            .await;

        assert_eq!(
            resolve_subagent_confirmation_defaults(gcx, "subagent").await,
            (false, false, false)
        );
    }

    #[test]
    fn context_reconstruction_is_the_only_ephemeral_subchat_trajectory() {
        let mut config = test_subchat_config();
        assert!(should_persist_subchat_trajectory(&config));

        config.tool_name = "title_generation".to_string();
        assert!(should_persist_subchat_trajectory(&config));

        config.tool_name = "mode_transition".to_string();
        assert!(!should_persist_subchat_trajectory(&config));

        config.stateful = true;
        assert!(should_persist_subchat_trajectory(&config));
    }

    #[test]
    fn subchat_trajectory_commit_phases_have_explicit_intents() {
        let cases = [
            (
                SubchatTrajectoryCommitPhase::Seed,
                TrajectoryCommitIntent::Checkpoint,
            ),
            (
                SubchatTrajectoryCommitPhase::Progress,
                TrajectoryCommitIntent::Checkpoint,
            ),
            (
                SubchatTrajectoryCommitPhase::Failed,
                TrajectoryCommitIntent::Required,
            ),
            (
                SubchatTrajectoryCommitPhase::Final,
                TrajectoryCommitIntent::Required,
            ),
        ];

        for (phase, expected) in cases {
            assert_eq!(subchat_trajectory_commit_intent(phase), expected);
        }
    }

    #[tokio::test]
    async fn final_and_failed_subchat_snapshots_use_required_durable_commits() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let chat_id = "subchat-required-final-failed";
        let config = test_subchat_config();
        let thread = trace_thread_from_config(chat_id, &config);
        let final_messages = vec![ChatMessage::new(
            "assistant".to_string(),
            "final".to_string(),
        )];

        save_trajectory_as_with_intent(
            gcx.clone(),
            &thread,
            &final_messages,
            subchat_trajectory_commit_intent(SubchatTrajectoryCommitPhase::Final),
        )
        .await;
        let final_saved = crate::chat::trajectories::load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .expect("final subchat snapshot should be durable");
        assert_eq!(final_saved.messages[0].content.content_text_only(), "final");

        let progress = Arc::new(StdMutex::new(final_messages));
        save_failed_subchat_trajectory(gcx.clone(), chat_id, &config, &progress, "failed").await;
        let failed_saved = crate::chat::trajectories::load_trajectory_for_chat(gcx, chat_id)
            .await
            .expect("failed subchat snapshot should be durable");
        assert!(failed_saved.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("Subchat run failed")));
    }

    #[test]
    fn context_reconstruction_never_retries_provider_calls() {
        let mut config = test_subchat_config();
        assert!(subchat_retries_allowed(&config));

        config.tool_name = "mode_transition".to_string();
        assert!(!subchat_retries_allowed(&config));
    }

    fn sample_worktree() -> (tempfile::TempDir, WorktreeMeta) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&source).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = fs::canonicalize(source).unwrap();
        (
            temp,
            WorktreeMeta {
                id: "wt-subchat".to_string(),
                kind: "task_agent".to_string(),
                root,
                source_workspace_root: source.clone(),
                repo_root: source,
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("card-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                enforce: true,
            },
        )
    }

    async fn install_caps(gcx: Arc<crate::global_context::GlobalContext>, caps: CodeAssistantCaps) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(60);
        let caps_state = gcx.caps_state.clone();
        let mut caps_state = caps_state.write().await;
        caps_state.caps = Some(Arc::new(caps));
        caps_state.last_attempted_ts = now;
    }

    fn privacy_record(path: &str, zone: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            zone: zone.to_string(),
            attribution: Attribution::Declared,
        }
    }

    fn message_with_privacy(role: &str, content: &str, records: Vec<FileRecord>) -> ChatMessage {
        let mut message = ChatMessage::new(role.to_string(), content.to_string());
        message.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord { files: records }).unwrap(),
        );
        message
    }

    fn install_privacy_policy(
        gcx: &Arc<crate::global_context::GlobalContext>,
        allowed_destination: &str,
        report_declassifies: bool,
    ) {
        *gcx.privacy_policy_load.write().unwrap() = PolicyLoad {
            policy: Arc::new(PrivacyPolicy {
                blocked: Vec::new(),
                zones: vec![
                    Zone {
                        name: "secrets".to_string(),
                        patterns: vec![".env*".to_string()],
                        send_to: vec![allowed_destination.to_string()],
                        on_shell_read: ShellBehavior::Withhold,
                    },
                    Zone {
                        name: "normal".to_string(),
                        patterns: vec!["*".to_string()],
                        send_to: vec!["*".to_string()],
                        on_shell_read: ShellBehavior::Withhold,
                    },
                ],
                subagents: SubagentPolicy {
                    report_declassifies,
                },
                ..Default::default()
            }),
            error: None,
            source_paths: Vec::new(),
        };
    }

    #[tokio::test]
    async fn subchat_projects_archived_secrets_before_privacy_gate() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        install_privacy_policy(&gcx, "trusted", true);
        let archived = message_with_privacy(
            "user",
            "archived secret",
            vec![privacy_record(".env", "secrets")],
        );
        assert!(gate_subchat_boundary(&gcx, &[archived.clone()], "untrusted/model").is_err());
        let mut current = ChatMessage::new("user".into(), "public current request".into());
        current.message_id = "public-current-request".into();
        let report = refact_core::active_context::make_reconstruction_report(
            vec![current],
            Default::default(),
        )
        .unwrap();
        let prepared =
            prepare_subchat_messages(&gcx, vec![archived, report], "untrusted/model").unwrap();
        assert_eq!(prepared.len(), 1);
        assert_eq!(
            prepared[0].content.content_text_only(),
            "public current request"
        );
    }

    #[tokio::test]
    async fn subchat_privacy_in_gate_checks_subagent_model_destination() {
        let gcx = make_test_gcx().await;
        install_privacy_policy(&gcx, "trusted", true);
        let messages = vec![message_with_privacy(
            "user",
            "guarded context",
            vec![privacy_record(".env", "secrets")],
        )];

        let error = gate_subchat_boundary(&gcx, &messages, "untrusted/model").unwrap_err();

        assert!(error.starts_with("Output withheld by user privacy policy"));
        assert!(error.contains("zone \""));
    }

    #[tokio::test]
    async fn subchat_sanitization_preserves_records_for_destination_gate() {
        let gcx = make_test_gcx().await;
        install_privacy_policy(&gcx, "trusted", true);
        let messages = vec![message_with_privacy(
            "user",
            "guarded context",
            vec![privacy_record(".env", "secrets")],
        )];

        let sanitized = sanitize_messages_for_new_thread(&messages);
        let error = prepare_subchat_messages(&gcx, sanitized, "untrusted/model").unwrap_err();

        assert!(error.starts_with("Output withheld by user privacy policy"));
        assert!(error.contains("zone \""));
    }

    #[tokio::test]
    async fn subchat_privacy_out_gate_checks_parent_model_destination() {
        let gcx = make_test_gcx().await;
        install_privacy_policy(&gcx, "child", false);
        let mut messages = vec![
            message_with_privacy(
                "tool",
                "guarded result",
                vec![privacy_record(".env", "secrets")],
            ),
            ChatMessage::new("assistant".to_string(), "safe report".to_string()),
        ];
        let policy = gcx
            .privacy_policy_load
            .read()
            .unwrap()
            .policy
            .subagents
            .clone();
        apply_subchat_report_policy(&policy, &mut messages).unwrap();

        let error = gate_subchat_boundary(&gcx, &messages[1..], "parent/model").unwrap_err();

        assert!(error.starts_with("Output withheld by user privacy policy"));
    }

    #[test]
    fn subchat_report_declassifies_by_default() {
        let mut messages = vec![
            message_with_privacy(
                "tool",
                "guarded result",
                vec![privacy_record(".env", "secrets")],
            ),
            message_with_privacy(
                "assistant",
                "final report",
                vec![privacy_record("generated.txt", "normal")],
            ),
        ];

        apply_subchat_report_policy(&SubagentPolicy::default(), &mut messages).unwrap();

        assert!(!messages[1].extra.contains_key("privacy"));
    }

    #[test]
    fn subchat_report_inherits_union_when_declassification_is_disabled() {
        let secret = privacy_record(".env", "secrets");
        let normal = privacy_record("src/lib.rs", "normal");
        let mut messages = vec![
            message_with_privacy("tool", "first", vec![secret.clone(), normal.clone()]),
            message_with_privacy("tool", "second", vec![secret.clone()]),
            ChatMessage::new("assistant".to_string(), "final report".to_string()),
        ];

        apply_subchat_report_policy(
            &SubagentPolicy {
                report_declassifies: false,
            },
            &mut messages,
        )
        .unwrap();

        let report: PrivacyRecord =
            serde_json::from_value(messages[2].extra["privacy"].clone()).unwrap();
        assert_eq!(report.files, vec![secret, normal]);
    }

    #[tokio::test]
    async fn guarded_subchat_context_gets_non_quotation_instruction() {
        let gcx = make_test_gcx().await;
        install_privacy_policy(&gcx, "trusted", true);
        let messages = vec![message_with_privacy(
            "user",
            "guarded context",
            vec![privacy_record(".env", "secrets")],
        )];

        let prepared = prepare_subchat_messages(&gcx, messages, "trusted/model").unwrap();

        assert_eq!(prepared[0].role, "system");
        assert_eq!(
            prepared[0].content.content_text_only(),
            GUARDED_REPORT_INSTRUCTION
        );
    }

    #[test]
    fn subchat_context_limit_compaction_gate_allows_wrapped_partial_output_error() {
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = format!(
            "{} Original error: context_length_exceeded",
            PARTIAL_OUTPUT_STREAM_ERROR,
        );

        assert!(should_compact_context_limit_error(&error, 0, &Some(abort)));
    }

    #[test]
    fn subchat_context_limit_compaction_gate_respects_abort() {
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

        assert!(!should_compact_context_limit_error(
            "context_length_exceeded",
            0,
            &Some(abort),
        ));
    }

    #[test]
    fn partial_output_stream_error_message_redacts_provider_secret() {
        let output = partial_output_stream_error_message(
            "provider failed: Authorization: Bearer sk-test-secret",
        );

        assert!(output.contains(PARTIAL_OUTPUT_STREAM_ERROR));
        assert!(!output.contains("sk-test-secret"));
        assert!(!output.contains("Authorization: Bearer sk-test-secret"));
        assert!(output.contains("[REDACTED"));
    }

    #[test]
    fn partial_output_stream_error_message_bounds_huge_provider_error() {
        let far_tail = "FAR_TAIL_MARKER";
        let input = format!(
            "provider failed: Authorization: Bearer sk-test-secret {} {}",
            "x".repeat(200_000),
            far_tail,
        );

        let output = partial_output_stream_error_message(&input);
        let max_len = PARTIAL_OUTPUT_STREAM_ERROR.len()
            + " Original error: ".len()
            + SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS;

        assert!(output.len() <= max_len);
        assert!(output.contains(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(!output.contains("sk-test-secret"));
        assert!(!output.contains("Authorization: Bearer sk-test-secret"));
        assert!(output.contains("[REDACTED"));
        assert!(!output.contains(far_tail));
    }

    #[tokio::test]
    async fn subchat_reactive_compaction_unavailable_model_has_no_fallback_and_preserves_archive() {
        let gcx = make_test_gcx().await;
        install_caps(gcx.clone(), CodeAssistantCaps::default()).await;
        let config = test_subchat_config();
        let mut messages = vec![
            ChatMessage::new("user".to_string(), "first".to_string()),
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("x".repeat(500)),
                tool_call_id: "tc1".to_string(),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "wrap up".to_string()),
        ];

        let original = serde_json::to_value(&messages).unwrap();
        let compacted = apply_subchat_reactive_compaction(
            gcx,
            &config,
            &mut messages,
            "context_length_exceeded",
            1,
            true,
        )
        .await;

        assert!(!compacted);
        assert_eq!(messages.len(), 4);
        let diagnostic = messages.remove(2);
        assert_eq!(diagnostic.role, "error");
        assert!(crate::chat::diagnostics::is_ui_only_message(&diagnostic));
        assert!(diagnostic
            .content
            .content_text_only()
            .contains("context_length_exceeded"));
        assert_eq!(serde_json::to_value(&messages).unwrap(), original);
        assert_eq!(
            messages.last().unwrap().content.content_text_only(),
            "wrap up"
        );
        assert!(!messages
            .iter()
            .any(|message| message.role
                == refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE));
        assert!(!messages
            .iter()
            .any(refact_core::active_context::is_legacy_summary));
        assert!(!messages.iter().any(|message| {
            message
                .content
                .content_text_only()
                .contains("Previous non-user subchat activity was summarized")
        }));
    }

    #[tokio::test]
    async fn subchat_reactive_compaction_emits_parent_diagnostics() {
        let gcx = make_test_gcx().await;
        install_caps(gcx.clone(), CodeAssistantCaps::default()).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut config = test_subchat_config();
        config.parent_tool_call_id = Some("call_1".to_string());
        config.parent_subchat_tx = Some(std::sync::Arc::new(tokio::sync::Mutex::new(tx)));
        let mut messages = vec![
            ChatMessage::new("user".to_string(), "first".to_string()),
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("x".repeat(500)),
                tool_call_id: "tc1".to_string(),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "second".to_string()),
        ];

        let original = serde_json::to_value(&messages).unwrap();
        let error = "context_length_exceeded: Authorization: Bearer sk-test-secret";
        let compacted =
            apply_subchat_reactive_compaction(gcx, &config, &mut messages, error, 1, false).await;

        assert!(!compacted);
        assert_eq!(messages.len(), 4);
        let diagnostic = messages.pop().unwrap();
        assert_eq!(diagnostic.role, "error");
        assert!(crate::chat::diagnostics::is_ui_only_message(&diagnostic));
        let diagnostic_json = serde_json::to_string(&diagnostic).unwrap();
        assert!(!diagnostic_json.contains("sk-test-secret"));
        assert!(diagnostic_json.contains("[REDACTED"));
        assert_eq!(serde_json::to_value(&messages).unwrap(), original);
        let first = rx.try_recv().unwrap();
        assert_eq!(
            first.get("tool_call_id").and_then(|v| v.as_str()),
            Some("call_1")
        );
        let status = first.get("subchat_id").and_then(|v| v.as_str()).unwrap();
        assert!(status.contains("could not rebuild active context (attempt 1)"));
        assert!(status.contains("context_length_exceeded"));
        assert!(!status.contains("sk-test-secret"));
        assert!(status.contains("[REDACTED"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn subchat_parent_compaction_diagnostic_redacts_provider_secret() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut config = test_subchat_config();
        config.parent_tool_call_id = Some("call_1".to_string());
        config.parent_subchat_tx = Some(std::sync::Arc::new(tokio::sync::Mutex::new(tx)));
        let error = "context_length_exceeded: Authorization: Bearer sk-test-secret request failed";

        emit_parent_compaction_diagnostics(&config, error, 1, true).await;

        let first = rx.try_recv().unwrap();
        let status = first.get("subchat_id").and_then(|v| v.as_str()).unwrap();
        assert!(status.contains("handled by rebuilding active context"));
        assert!(status.contains("attempt 1"));
        assert!(!status.contains("sk-test-secret"));
        assert!(!status.contains("Authorization: Bearer sk-test-secret"));
        assert!(status.contains("[REDACTED"));
    }

    #[test]
    fn subchat_context_limit_log_error_redacts_provider_secret() {
        let error = "context_length_exceeded: Authorization: Bearer sk-test-secret";

        let log_error = safe_context_limit_error_for_log(error);

        assert!(!log_error.contains("sk-test-secret"));
        assert!(!log_error.contains("Authorization: Bearer sk-test-secret"));
        assert!(log_error.contains("[REDACTED"));
        assert!(log_error.len() <= PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS);
    }

    #[test]
    fn subchat_parent_compaction_diagnostic_caps_long_provider_error() {
        let error = format!("context_length_exceeded: {}", "x".repeat(4_000));

        let status = parent_compaction_diagnostic_status(&error, 9, false);

        assert!(status.contains("could not rebuild active context"));
        assert!(status.contains("attempt 9"));
        assert!(status.len() <= PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS);
        assert!(status.ends_with(PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED));
    }

    #[test]
    fn subchat_parent_compaction_diagnostic_windows_huge_provider_error() {
        let far_tail = "FAR_TAIL_MARKER";
        let error = format!(
            "context_length_exceeded: Authorization: Bearer sk-test-secret {} {}",
            "x".repeat(200_000),
            far_tail,
        );

        let status = parent_compaction_diagnostic_status(&error, 1, true);

        assert!(status.len() <= PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS);
        assert!(status.ends_with(PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED));
        assert!(!status.contains("sk-test-secret"));
        assert!(!status.contains("Authorization: Bearer sk-test-secret"));
        assert!(status.contains("[REDACTED"));
        assert!(!status.contains(far_tail));
    }

    #[test]
    fn subchat_context_limit_log_error_redacts_secret_near_lookahead_boundary() {
        let prefix_len = PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS.saturating_sub(80);
        let error = format!(
            "context_length_exceeded: {} Authorization: Bearer sk-boundarysecretvalue {}",
            "x".repeat(prefix_len),
            "y".repeat(PARENT_COMPACTION_DIAGNOSTIC_REDACTION_LOOKAHEAD_CHARS * 2),
        );

        let log_error = safe_context_limit_error_for_log(&error);

        assert!(log_error.len() <= PARENT_COMPACTION_DIAGNOSTIC_MAX_CHARS);
        assert!(log_error.ends_with(PARENT_COMPACTION_DIAGNOSTIC_TRUNCATED));
        assert!(!log_error.contains("sk-boundarysecretvalue"));
        assert!(!log_error.contains("Authorization: Bearer sk-boundarysecretvalue"));
        assert!(log_error.contains("[REDACTED"));
    }

    #[tokio::test]
    async fn general_subagent_context_cap_is_explicit_only_and_follows_live_session() {
        let gcx = make_test_gcx().await;
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/1m".into(),
            chat_model_record("test/1m", 1_000_000, "https://example.com/v1"),
        );
        caps.chat_models.insert(
            "test/200k".into(),
            chat_model_record("test/200k", 200_000, "https://example.com/v1"),
        );
        install_caps(gcx.clone(), caps).await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let launched = |cap| {
            let mut config = super::test_support::minimal_config("child", "parent");
            config.model = "test/1m".into();
            config.n_ctx = 200_000;
            config.max_new_tokens = 16_000;
            super::apply_launch_context_window(&mut config, 1_000_000, cap, true).map(|_| config)
        };

        let implicit = launched(None).unwrap();
        let thread = stateful_thread_from_config("child", &implicit);
        assert_eq!(implicit.n_ctx, 1_000_000);
        assert_eq!(thread.context_tokens_cap, None);
        assert_eq!(thread.auto_compression_cap, Some(900_000));
        let saved: SubchatConfig =
            serde_json::from_value(serde_json::to_value(&implicit).unwrap()).unwrap();
        assert_eq!(saved.context_tokens_cap(), None);

        let explicit = launched(Some(300_000)).unwrap();
        let thread = stateful_thread_from_config("child", &explicit);
        assert_eq!(thread.context_tokens_cap, Some(300_000));
        assert_eq!(thread.auto_compression_cap, Some(270_000));
        let mut legacy = serde_json::to_value(&explicit).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("implicit_context_window");
        let legacy: SubchatConfig = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.context_tokens_cap(), Some(300_000));

        assert!(launched(Some(1_000_001)).err().unwrap().contains("exceeds"));
        assert!(launched(Some(0)).is_err());

        let workspace = tempfile::tempdir().unwrap();
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let messages = vec![ChatMessage::new("user".into(), "task".into())];
        super::install_stateful_subchat_session(&app, "child", &explicit, &messages, None)
            .await
            .unwrap();
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get("child")
            .cloned()
            .unwrap();
        session
            .lock()
            .await
            .set_runtime_state(crate::chat::types::SessionState::Generating, None);
        let ccx = Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_from_app(
                app.clone(),
                200_000,
                1,
                false,
                vec![],
                "child".into(),
                None,
                "test/1m".into(),
                None,
                None,
            )
            .await,
        ));
        super::refresh_live_context_window(&ccx, &explicit).await;
        assert_eq!(ccx.lock().await.n_ctx, 300_000);
        session.lock().await.thread.context_tokens_cap = Some(2_000_000);
        super::refresh_live_context_window(&ccx, &explicit).await;
        assert_eq!(ccx.lock().await.n_ctx, 1_000_000);
        session.lock().await.thread.context_tokens_cap = None;
        ccx.lock().await.n_ctx = 200_000;
        let mut supervised = explicit.clone();
        supervised.tools = ToolsPolicy::Supervised {
            tools: vec![],
            budget: Default::default(),
        };
        super::refresh_live_context_window(&ccx, &supervised).await;
        assert_eq!(ccx.lock().await.n_ctx, 200_000);
        super::refresh_live_context_window(&ccx, &explicit).await;
        assert_eq!(ccx.lock().await.n_ctx, 1_000_000);

        let progress = Arc::new(StdMutex::new(messages.clone()));
        super::save_failed_subchat_trajectory(
            app.gcx.clone(),
            "child",
            &explicit,
            &progress,
            "provider failed",
        )
        .await;
        let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "child")
            .await
            .unwrap();
        assert_eq!(saved.thread.context_tokens_cap, None);
        assert!(saved.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("provider failed")));
    }

    #[test]
    fn subchat_worktree_stateful_thread_from_config_carries_scope() {
        let (_temp, worktree) = sample_worktree();
        let task_meta = TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("agent-1".to_string()),
            card_id: Some("card-1".to_string()),
            planner_chat_id: Some("planner-task-1-1".to_string()),
        };
        let config = SubchatConfig {
            history_source_model: None,
            tool_name: "subagent".to_string(),
            stateful: true,
            autonomous_no_confirm: false,
            auto_approve_editing_tools: true,
            auto_approve_dangerous_commands: true,
            chat_id: None,
            title: Some("Subchat".to_string()),
            parent_id: Some("parent".to_string()),
            link_type: Some("subagent".to_string()),
            root_chat_id: Some("root".to_string()),
            tools: ToolsPolicy::Only(vec!["cat".to_string()]),
            max_steps: 3,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: Some(task_meta.clone()),
            worktree: Some(worktree.clone()),
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            implicit_context_window: false,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: Some(ReasoningEffort::Low),
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            soft_abort: false,
            activity_stamp: None,
            background_agent_id: None,
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: TraceParent::unattributed(),
        };

        let thread = stateful_thread_from_config("subchat-1", &config);

        assert_eq!(thread.id, "subchat-1");
        assert_eq!(thread.context_tokens_cap, Some(4096));
        assert_eq!(thread.auto_compression_cap, Some(3686));
        assert!(!thread.auto_compression_cap_pending);
        assert_eq!(thread.auto_compression_cap, Some(3686));
        assert!(!thread.auto_compression_cap_pending);
        assert_eq!(thread.task_meta, Some(task_meta));
        assert_eq!(thread.worktree, Some(worktree));
        assert_eq!(thread.tool_use, "cat");
        assert_eq!(thread.parent_id.as_deref(), Some("parent"));
        assert_eq!(thread.root_chat_id.as_deref(), Some("root"));
        assert!(thread.auto_approve_editing_tools);
        assert!(thread.auto_approve_dangerous_commands);
    }

    #[test]
    fn stateful_thread_from_config_preserves_disabled_approval_flags() {
        let config = test_subchat_config();

        let thread = stateful_thread_from_config("subchat-1", &config);

        assert!(!thread.auto_approve_editing_tools);
        assert!(!thread.auto_approve_dangerous_commands);
    }

    #[test]
    fn stateful_subchat_from_planner_uses_hidden_task_role() {
        let task_meta = TaskMeta {
            task_id: "task-1".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("planner-task-1-1".to_string()),
        };
        let config = SubchatConfig {
            history_source_model: None,
            tool_name: "subagent".to_string(),
            stateful: true,
            autonomous_no_confirm: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            chat_id: None,
            title: Some("Subagent: Gathering Files".to_string()),
            parent_id: Some("planner-task-1-1".to_string()),
            link_type: Some("gather_files".to_string()),
            root_chat_id: Some("planner-task-1-1".to_string()),
            tools: ToolsPolicy::Only(vec!["cat".to_string(), "tree".to_string()]),
            max_steps: 3,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: Some(task_meta),
            worktree: None,
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            implicit_context_window: false,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            soft_abort: false,
            activity_stamp: None,
            background_agent_id: None,
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: TraceParent::unattributed(),
        };

        let thread = stateful_thread_from_config("subchat-1", &config);

        assert_eq!(
            thread.task_meta.as_ref().map(|m| m.role.as_str()),
            Some("subchats")
        );
        assert_eq!(
            thread.task_meta.as_ref().map(|m| m.task_id.as_str()),
            Some("task-1")
        );
        assert_eq!(thread.parent_id.as_deref(), Some("planner-task-1-1"));
        assert_eq!(thread.link_type.as_deref(), Some("gather_files"));
        assert_eq!(thread.root_chat_id.as_deref(), Some("planner-task-1-1"));
    }

    #[tokio::test]
    async fn subchat_worktree_config_inherits_scope_from_parent_session() {
        let gcx = make_test_gcx().await;
        let (_temp, worktree) = sample_worktree();
        let parent_chat_id = "parent-session-chat".to_string();
        let sessions = gcx.chat_sessions.clone();
        {
            let mut sessions_write = sessions.write().await;
            let mut parent_session = crate::chat::types::ChatSession::new(parent_chat_id.clone());
            parent_session.thread.worktree = Some(worktree.clone());
            sessions_write.insert(
                parent_chat_id.clone(),
                Arc::new(tokio::sync::Mutex::new(parent_session)),
            );
        }

        let resolved = resolve_subchat_worktree(gcx, Some(&parent_chat_id), None).await;

        assert_eq!(resolved, Some(worktree));
    }

    #[tokio::test]
    async fn subchat_worktree_config_prefers_current_parent_scope_over_parent_session() {
        let gcx = make_test_gcx().await;
        let (_stored_temp, stored_worktree) = sample_worktree();
        let (_current_temp, mut current_worktree) = sample_worktree();
        current_worktree.id = "wt-current-parent-scope".to_string();
        let parent_chat_id = "parent-explicit-chat".to_string();
        let sessions = gcx.chat_sessions.clone();
        {
            let mut sessions_write = sessions.write().await;
            let mut parent_session = crate::chat::types::ChatSession::new(parent_chat_id.clone());
            parent_session.thread.worktree = Some(stored_worktree);
            sessions_write.insert(
                parent_chat_id.clone(),
                Arc::new(tokio::sync::Mutex::new(parent_session)),
            );
        }

        let resolved =
            resolve_subchat_worktree(gcx, Some(&parent_chat_id), Some(current_worktree.clone()))
                .await;

        assert_eq!(resolved, Some(current_worktree));
    }

    #[tokio::test]
    async fn subchat_worktree_config_does_not_invent_scope_when_parent_has_none() {
        let gcx = make_test_gcx().await;
        let parent_chat_id = "parent-without-worktree".to_string();
        let sessions = gcx.chat_sessions.clone();
        {
            let mut sessions_write = sessions.write().await;
            sessions_write.insert(
                parent_chat_id.clone(),
                Arc::new(tokio::sync::Mutex::new(
                    crate::chat::types::ChatSession::new(parent_chat_id.clone()),
                )),
            );
        }

        let resolved = resolve_subchat_worktree(gcx, Some(&parent_chat_id), None).await;

        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn subchat_worktree_lookup_falls_back_to_parent_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let service = crate::worktrees::service::WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/subchat-parent".to_string()),
                chat_id: Some("parent-trajectory-chat".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let worktree = created.worktree.meta.clone();
        let parent_chat_id = "parent-trajectory-chat".to_string();
        let thread = ThreadParams {
            id: parent_chat_id.clone(),
            title: "Parent".to_string(),
            model: "model".to_string(),
            worktree: Some(worktree.clone()),
            ..Default::default()
        };
        let messages = vec![ChatMessage::new("user".to_string(), "hello".to_string())];
        save_trajectory_as_with_intent(
            gcx.clone(),
            &thread,
            &messages,
            TrajectoryCommitIntent::Checkpoint,
        )
        .await;

        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if parent_thread_worktree(gcx.clone(), &parent_chat_id)
                    .await
                    .is_some()
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("checkpoint should persist the parent worktree");
        assert_eq!(
            parent_thread_worktree(gcx, &parent_chat_id).await,
            Some(worktree)
        );
    }

    #[tokio::test]
    async fn subchat_worktree_active_detached_session_does_not_restore_stale_trajectory_scope() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let service = crate::worktrees::service::WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/subchat-detached-parent".to_string()),
                chat_id: Some("parent-detached-chat".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let parent_chat_id = "parent-detached-chat".to_string();
        let persisted_thread = ThreadParams {
            id: parent_chat_id.clone(),
            title: "Parent".to_string(),
            model: "model".to_string(),
            worktree: Some(created.worktree.meta.clone()),
            ..Default::default()
        };
        let messages = vec![ChatMessage::new("user".to_string(), "hello".to_string())];
        save_trajectory_as_with_intent(
            gcx.clone(),
            &persisted_thread,
            &messages,
            TrajectoryCommitIntent::Checkpoint,
        )
        .await;
        let sessions = gcx.chat_sessions.clone();
        {
            let mut sessions_write = sessions.write().await;
            sessions_write.insert(
                parent_chat_id.clone(),
                Arc::new(tokio::sync::Mutex::new(
                    crate::chat::types::ChatSession::new(parent_chat_id.clone()),
                )),
            );
        }

        assert_eq!(parent_thread_worktree(gcx, &parent_chat_id).await, None);
    }

    #[tokio::test]
    async fn subchat_worktree_registers_stateful_child_reference() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let service = crate::worktrees::service::WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/subchat-child-reference".to_string()),
                chat_id: Some("parent-ref-chat".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let config = SubchatConfig {
            history_source_model: None,
            tool_name: "subagent".to_string(),
            stateful: true,
            autonomous_no_confirm: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            chat_id: Some("child-ref-chat".to_string()),
            title: Some("Subchat".to_string()),
            parent_id: Some("parent-ref-chat".to_string()),
            link_type: Some("subagent".to_string()),
            root_chat_id: Some("parent-ref-chat".to_string()),
            tools: ToolsPolicy::Only(vec!["cat".to_string()]),
            max_steps: 1,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: None,
            worktree: Some(created.worktree.meta.clone()),
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            implicit_context_window: false,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            soft_abort: false,
            activity_stamp: None,
            background_agent_id: None,
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: TraceParent::unattributed(),
        };
        let thread = stateful_thread_from_config("child-ref-chat", &config);

        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let messages = vec![ChatMessage::new(
            "user".into(),
            "retained runner history".into(),
        )];
        super::install_stateful_subchat_session(&app, "child-ref-chat", &config, &messages, None)
            .await
            .unwrap();
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get("child-ref-chat")
            .cloned()
            .unwrap();
        assert_eq!(
            session.lock().await.runtime.state,
            crate::chat::types::SessionState::Idle
        );
        let before = serde_json::to_value(&session.lock().await.thread).unwrap();
        assert!(!before["worktree"]["id"].as_str().unwrap().is_empty());
        let registry_before = serde_json::to_value(service.load_registry().await.unwrap()).unwrap();
        let transition = crate::chat::worktree_lifecycle::try_transition_lock(&session).unwrap();
        let error = super::save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &messages,
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap_err();
        assert!(error.contains("transition in progress"));
        assert_eq!(
            serde_json::to_value(&session.lock().await.thread).unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(service.load_registry().await.unwrap()).unwrap(),
            registry_before
        );
        drop(transition);
        super::save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &messages,
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap();
        let loaded = crate::chat::trajectories::load_trajectory_for_chat(gcx, "child-ref-chat")
            .await
            .unwrap();
        let mut appended = messages.clone();
        appended.push(ChatMessage::new(
            "assistant".into(),
            "checkpoint output".into(),
        ));
        super::save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &appended,
            TrajectoryCommitIntent::Checkpoint,
        )
        .await
        .unwrap();
        session.lock().await.trajectory_dirty = true;
        assert_eq!(
            session.lock().await.runtime.state,
            crate::chat::types::SessionState::Idle
        );
        let mut final_messages = appended.clone();
        final_messages.push(ChatMessage::new("assistant".into(), "final output".into()));
        super::save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &final_messages,
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&session.lock().await.messages).unwrap(),
            serde_json::to_value(&final_messages).unwrap()
        );
        let bytes = std::fs::read(&loaded.source_path).unwrap();
        session
            .lock()
            .await
            .add_message(ChatMessage::new("user".into(), "foreign live edit".into()));
        let error = super::save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &final_messages,
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap_err();
        assert!(error.contains("no longer owns"));
        assert_eq!(
            session
                .lock()
                .await
                .messages
                .last()
                .unwrap()
                .content
                .content_text_only(),
            "foreign live edit"
        );
        assert_eq!(std::fs::read(&loaded.source_path).unwrap(), bytes);
        session.lock().await.thread.worktree = None;
        let error = super::save_subchat_trajectory(
            &app,
            &config,
            &thread,
            &[ChatMessage::new(
                "user".into(),
                "stale runner replacement".into(),
            )],
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap_err();
        assert!(error.contains("scope or ownership changed"));
        assert!(session.lock().await.thread.worktree.is_none());
        assert_eq!(
            session.lock().await.messages[0].content.content_text_only(),
            "retained runner history"
        );
        assert_eq!(std::fs::read(&loaded.source_path).unwrap(), bytes);
        assert_eq!(thread.worktree, Some(created.worktree.meta.clone()));

        let view = service
            .get_worktree(&created.worktree.meta.id)
            .await
            .unwrap();
        assert_eq!(view.reference_count, 2);
        assert!(view
            .references
            .iter()
            .any(|reference| reference.chat_id.as_deref() == Some("parent-ref-chat")));
        assert!(view
            .references
            .iter()
            .any(|reference| reference.chat_id.as_deref() == Some("child-ref-chat")));
    }

    #[test]
    fn test_parse_subchat_cache_control_rejects_invalid_values() {
        let err = parse_subchat_cache_control("bad_cache_agent", Some("forever")).unwrap_err();

        assert!(err.contains("invalid cache_control 'forever'"));
        assert!(err.contains("bad_cache_agent"));
        assert!(err.contains("expected: off, ephemeral"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_model_errors_when_light_model_missing() {
        let gcx = make_test_gcx().await;
        let default_model_id = "openai/gpt-4o".to_string();

        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model_id.clone(),
            chat_model_record(
                &default_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.defaults.chat_default_model = default_model_id;

        install_caps(gcx.clone(), caps).await;

        let params = SubchatParameters {
            subchat_model_type: ChatModelType::Light,
            subchat_model: String::new(),
            subchat_n_ctx: 128_000,
            subchat_max_new_tokens: 8_192,
            subchat_temperature: None,
            subchat_tokens_for_rag: 0,
            subchat_reasoning_effort: None,
            subchat_cache_control: crate::llm::params::CacheControl::Ephemeral,
        };

        let err = resolve_subchat_model(gcx, &params).await.unwrap_err();
        assert!(err.contains("Light model is not set up"));
        assert!(err.contains("Default model settings"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_model_errors_when_endpoint_empty() {
        let gcx = make_test_gcx().await;
        let default_model_id = "openai/gpt-4o".to_string();
        let thinking_model_id = "broken/o1".to_string();

        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model_id.clone(),
            chat_model_record(
                &default_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(&thinking_model_id, 128_000, ""),
        );
        caps.defaults.chat_default_model = default_model_id;
        caps.defaults.chat_light_model = "openai/gpt-4o-mini".to_string();
        caps.defaults.chat_thinking_model = thinking_model_id.clone();

        install_caps(gcx.clone(), caps).await;

        let params = SubchatParameters {
            subchat_model_type: ChatModelType::Thinking,
            subchat_model: String::new(),
            subchat_n_ctx: 128_000,
            subchat_max_new_tokens: 8_192,
            subchat_temperature: None,
            subchat_tokens_for_rag: 0,
            subchat_reasoning_effort: None,
            subchat_cache_control: crate::llm::params::CacheControl::Ephemeral,
        };

        let err = resolve_subchat_model(gcx, &params).await.unwrap_err();
        assert!(err.contains("Thinking model 'broken/o1' is misconfigured"));
        assert!(err.contains("an empty LLM endpoint URL"));
        assert!(err.contains("Default model settings"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_model_errors_when_endpoint_relative() {
        let gcx = make_test_gcx().await;
        let default_model_id = "openai/gpt-4o".to_string();
        let thinking_model_id = "openai/o1".to_string();

        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model_id.clone(),
            chat_model_record(
                &default_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(&thinking_model_id, 128_000, "/v1/chat/completions"),
        );
        caps.defaults.chat_default_model = default_model_id;
        caps.defaults.chat_thinking_model = thinking_model_id.clone();

        install_caps(gcx.clone(), caps).await;

        let params = SubchatParameters {
            subchat_model_type: ChatModelType::Thinking,
            subchat_model: String::new(),
            subchat_n_ctx: 128_000,
            subchat_max_new_tokens: 8_192,
            subchat_temperature: None,
            subchat_tokens_for_rag: 0,
            subchat_reasoning_effort: None,
            subchat_cache_control: crate::llm::params::CacheControl::Ephemeral,
        };

        let err = resolve_subchat_model(gcx, &params).await.unwrap_err();
        assert!(err.contains("Thinking model 'openai/o1' is misconfigured"));
        assert!(err.contains("an invalid LLM endpoint URL"));
        assert!(err.contains("Default model settings"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_params_names_gather_files_light_model_requirement() {
        let gcx = make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        global_configs_try_create_all(&config_dir).await.unwrap();

        let thinking_model_id = "openai/gpt-5".to_string();
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(
                &thinking_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.defaults.chat_default_model = thinking_model_id.clone();
        caps.defaults.chat_thinking_model = thinking_model_id;

        install_caps(gcx.clone(), caps).await;

        let err = resolve_subchat_params(gcx, "title_generation")
            .await
            .unwrap_err();

        assert!(err.contains("Light model required by subagent 'title_generation'"));
        assert!(err.contains("model_type: light"));
        assert!(err.contains("Default model settings"));
    }
}
