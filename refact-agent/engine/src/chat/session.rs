use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use serde_json::json;
use tokio::sync::{broadcast, Mutex as AMutex, Notify, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_hooks};

use super::types::*;
use super::types::{session_idle_timeout, session_cleanup_interval};
use super::config::limits;
use super::trajectories::TrajectoryEvent;

pub type SessionsMap = Arc<ARwLock<HashMap<String, Arc<AMutex<ChatSession>>>>>;

pub fn create_sessions_map() -> SessionsMap {
    Arc::new(ARwLock::new(HashMap::new()))
}

impl ChatSession {
    pub fn new(chat_id: String) -> Self {
        let (event_tx, _) = broadcast::channel(limits().event_channel_capacity);
        Self {
            chat_id: chat_id.clone(),
            thread: ThreadParams {
                id: chat_id,
                ..Default::default()
            },
            messages: Vec::new(),
            runtime: RuntimeState::default(),
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx,
            trajectory_events_tx: None,
            recent_request_ids: VecDeque::with_capacity(limits().recent_request_ids_capacity),
            abort_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            trajectory_dirty: false,
            trajectory_version: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            pending_browser_message: None,
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
        }
    }

    pub fn new_with_trajectory(
        chat_id: String,
        messages: Vec<ChatMessage>,
        mut thread: ThreadParams,
        created_at: String,
    ) -> Self {
        // active_skill is runtime state — if the server restarted mid-skill, the compaction
        // anchor (started_at_index) is lost. Clear it so the session starts cleanly rather
        // than leaving the user locked into a ghost skill that can never be deactivated.
        thread.active_skill = None;
        let (event_tx, _) = broadcast::channel(limits().event_channel_capacity);
        Self {
            chat_id,
            thread,
            messages,
            runtime: RuntimeState::default(),
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx,
            trajectory_events_tx: None,
            recent_request_ids: VecDeque::with_capacity(limits().recent_request_ids_capacity),
            abort_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            external_reload_pending: false,
            trajectory_dirty: false,
            trajectory_version: 0,
            created_at,
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            last_prompt_messages: Vec::new(),
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            pending_browser_message: None,
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
        }
    }

    pub fn increment_version(&mut self) {
        self.trajectory_version += 1;
        self.trajectory_dirty = true;
    }

    pub fn set_active_skill(&mut self, name: String) {
        self.thread.active_skill = Some(name);
        self.increment_version();
    }

    pub fn clear_active_skill(&mut self) {
        self.thread.active_skill = None;
        self.increment_version();
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_idle_for_cleanup(&self) -> bool {
        let is_idle_like = matches!(
            self.runtime.state,
            SessionState::Idle | SessionState::Completed | SessionState::WaitingUserInput
        );
        is_idle_like
            && self.command_queue.is_empty()
            && self.last_activity.elapsed() > session_idle_timeout()
    }

    pub fn close_event_channel(&mut self) {
        self.closed = true;
        self.closed_flag.store(true, Ordering::Relaxed);
        let (new_tx, _) = broadcast::channel(limits().event_channel_capacity);
        self.event_tx = new_tx;
    }

    pub fn emit(&mut self, event: ChatEvent) {
        self.event_seq += 1;
        let envelope = EventEnvelope {
            chat_id: self.chat_id.clone(),
            seq: self.event_seq,
            event,
        };
        let _ = self.event_tx.send(envelope);
    }

    pub fn snapshot(&self) -> ChatEvent {
        let mut messages = self.messages.clone();
        if self.runtime.state == SessionState::Generating {
            if let Some(ref draft) = self.draft_message {
                messages.push(draft.clone());
            }
        }
        let mut runtime = self.runtime.clone();
        runtime.queue_size = self.command_queue.len();
        runtime.queued_items = self.build_queued_items();
        ChatEvent::Snapshot {
            thread: self.thread.clone(),
            runtime,
            messages,
        }
    }

    pub fn is_duplicate_request(&mut self, request_id: &str) -> bool {
        if self.recent_request_ids.contains(&request_id.to_string()) {
            return true;
        }
        if self.recent_request_ids.len() >= 100 {
            self.recent_request_ids.pop_front();
        }
        self.recent_request_ids.push_back(request_id.to_string());
        false
    }

    pub fn add_message(&mut self, mut message: ChatMessage) {
        if message.message_id.is_empty() {
            message.message_id = Uuid::new_v4().to_string();
        }
        let index = self.messages.len();
        self.messages.push(message.clone());
        self.emit(ChatEvent::MessageAdded { message, index });
        self.increment_version();
        self.touch();
    }

    pub fn insert_message(&mut self, index: usize, mut message: ChatMessage) {
        if message.message_id.is_empty() {
            message.message_id = Uuid::new_v4().to_string();
        }
        let insert_idx = index.min(self.messages.len());
        self.messages.insert(insert_idx, message.clone());
        self.emit(ChatEvent::MessageAdded { message, index: insert_idx });
        self.increment_version();
        self.touch();
    }

    pub fn update_message(&mut self, message_id: &str, message: ChatMessage) -> Option<usize> {
        if let Some(idx) = self
            .messages
            .iter()
            .position(|m| m.message_id == message_id)
        {
            self.messages[idx] = message.clone();
            self.thread.previous_response_id = None;
            self.emit(ChatEvent::MessageUpdated {
                message_id: message_id.to_string(),
                message,
            });
            self.increment_version();
            self.touch();
            return Some(idx);
        }
        None
    }

    pub fn remove_message(&mut self, message_id: &str) -> Option<usize> {
        if let Some(idx) = self
            .messages
            .iter()
            .position(|m| m.message_id == message_id)
        {
            let msg = &self.messages[idx];
            let role = msg.role.clone();
            let tool_call_ids: Vec<String> = msg.tool_calls
                .as_ref()
                .map(|tcs| tcs.iter().map(|tc| tc.id.clone()).collect())
                .unwrap_or_default();

            self.messages.remove(idx);
            self.thread.previous_response_id = None;
            self.emit(ChatEvent::MessageRemoved {
                message_id: message_id.to_string(),
            });

            if role == "assistant" && !tool_call_ids.is_empty() {
                let tool_msg_ids: Vec<String> = self.messages
                    .iter()
                    .filter(|m| m.role == "tool" && tool_call_ids.contains(&m.tool_call_id))
                    .map(|m| m.message_id.clone())
                    .collect();

                for tid in tool_msg_ids {
                    if let Some(tool_idx) = self.messages.iter().position(|m| m.message_id == tid) {
                        self.messages.remove(tool_idx);
                        self.emit(ChatEvent::MessageRemoved { message_id: tid });
                    }
                }
            }

            self.increment_version();
            self.touch();
            return Some(idx);
        }
        None
    }

    pub fn truncate_messages(&mut self, from_index: usize) {
        if from_index < self.messages.len() {
            self.messages.truncate(from_index);
            self.thread.previous_response_id = None;
            self.emit(ChatEvent::MessagesTruncated { from_index });
            self.increment_version();
            self.touch();
        }
    }

    pub fn perform_skill_deactivation_cleanup(&mut self) {
        let Some(pending) = self.pending_skill_deactivation.take() else { return };

        if pending.start_index > self.messages.len() {
            warn!(
                "Skill deactivation cleanup: start_index {} is beyond messages.len() {} for skill '{}', skipping compaction",
                pending.start_index, self.messages.len(), pending.skill_name
            );
            return;
        }

        let mut compaction_start = pending.start_index;
        if let Some(ref activation_tool_call_id) = pending.activation_tool_call_id {
            if self.messages.get(compaction_start).map_or(false, |msg| {
                msg.role == "tool" && msg.tool_call_id == *activation_tool_call_id
            }) {
                compaction_start = compaction_start.saturating_add(1);
            }
        }

        if compaction_start > self.messages.len() {
            warn!(
                "Skill deactivation cleanup: compaction_start {} is beyond messages.len() {} for skill '{}', skipping compaction",
                compaction_start, self.messages.len(), pending.skill_name
            );
            return;
        }

        info!("Skill deactivation cleanup: compacting messages from index {} for skill '{}'", compaction_start, pending.skill_name);
        self.truncate_messages(compaction_start);

        let report_content = format!(
            "## Skill Report: {}\n\n✅ Skill '{}' executed successfully.\n\nHere is the compactified result. The full skill conversation was compactified and removed from the thread.\n\n{}",
            pending.skill_name,
            pending.skill_name,
            pending.report
        );
        let report_message = ChatMessage {
            role: "plain_text".to_string(),
            content: ChatContent::SimpleText(report_content),
            ..Default::default()
        };
        self.add_message(report_message);
    }

    pub fn set_runtime_state(&mut self, state: SessionState, error: Option<String>) {
        let old_state = self.runtime.state;
        let old_error = self.runtime.error.clone();
        let was_paused = old_state == SessionState::Paused;
        let had_pause_reasons = !self.runtime.pause_reasons.is_empty();

        self.runtime.state = state;
        self.runtime.paused = state == SessionState::Paused;
        self.runtime.error = error.clone();
        self.runtime.queue_size = self.command_queue.len();
        self.runtime.queued_items = self.build_queued_items();

        if state != SessionState::Paused && (was_paused || had_pause_reasons) {
            self.runtime.pause_reasons.clear();
            self.runtime.auto_approved_tool_ids.clear();
            self.runtime.accepted_tool_ids.clear();
            self.runtime.paused_message_index = None;
            self.emit(ChatEvent::PauseCleared {});
        }

        let state_changed = old_state != state;
        let error_changed = old_error != error;
        if state_changed || error_changed {
            self.emit(ChatEvent::RuntimeUpdated {
                state,
                error: error.clone(),
            });
            self.emit_trajectory_state_change();
        }
    }

    fn emit_trajectory_state_change(&self) {
        if self.thread.task_meta.is_some() {
            return;
        }
        if let Some(ref tx) = self.trajectory_events_tx {
            let state_str = match self.runtime.state {
                SessionState::Idle => "idle",
                SessionState::Generating => "generating",
                SessionState::ExecutingTools => "executing_tools",
                SessionState::Paused => "paused",
                SessionState::WaitingIde => "waiting_ide",
                SessionState::WaitingUserInput => "waiting_user_input",
                SessionState::Completed => "completed",
                SessionState::Error => "error",
            };
            let effective_root = self.thread.root_chat_id.clone().unwrap_or_else(|| self.chat_id.clone());
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: self.chat_id.clone(),
                updated_at: None,
                title: None,
                is_title_generated: None,
                session_state: Some(state_str.to_string()),
                error: self.runtime.error.clone(),
                message_count: Some(self.messages.len()),
                parent_id: self.thread.parent_id.clone(),
                link_type: self.thread.link_type.clone(),
                root_chat_id: Some(effective_root),
                model: Some(self.thread.model.clone()),
                mode: Some(self.thread.mode.clone()),
                total_coins: None,
                total_lines_added: None,
                total_lines_removed: None,
                tasks_total: None,
                tasks_done: None,
                tasks_failed: None,
                total_prompt_tokens: None,
                total_completion_tokens: None,
                total_tokens: None,
                total_cache_read_tokens: None,
                total_cache_creation_tokens: None,
                total_cost_usd: None,
            };
            let _ = tx.send(event);
        }
    }

    pub fn build_queued_items(&self) -> Vec<QueuedItem> {
        self.command_queue
            .iter()
            .map(|r| r.to_queued_item())
            .collect()
    }

    pub fn emit_queue_update(&mut self) {
        self.runtime.queue_size = self.command_queue.len();
        self.runtime.queued_items = self.build_queued_items();
        self.emit(ChatEvent::QueueUpdated {
            queue_size: self.runtime.queue_size,
            queued_items: self.runtime.queued_items.clone(),
        });
    }

    pub fn set_paused_with_reasons_and_auto_approved(&mut self, reasons: Vec<PauseReason>, auto_approved_ids: Vec<String>, message_index: Option<usize>) {
        self.runtime.pause_reasons = reasons.clone();
        self.runtime.auto_approved_tool_ids = auto_approved_ids;
        self.runtime.accepted_tool_ids.clear();
        self.runtime.paused_message_index = message_index;
        self.emit(ChatEvent::PauseRequired { reasons });
        self.set_runtime_state(SessionState::Paused, None);
    }

    pub fn start_stream(&mut self) -> Option<(String, Arc<AtomicBool>)> {
        if self.runtime.state == SessionState::ExecutingTools || self.draft_message.is_some() {
            warn!("Attempted to start stream while already executing tools or draft exists");
            return None;
        }
        self.abort_flag.store(false, Ordering::SeqCst);
        let message_id = Uuid::new_v4().to_string();
        self.draft_message = Some(ChatMessage {
            message_id: message_id.clone(),
            role: "assistant".to_string(),
            ..Default::default()
        });
        self.draft_usage = None;
        self.set_runtime_state(SessionState::Generating, None);
        self.emit(ChatEvent::StreamStarted {
            message_id: message_id.clone(),
        });
        self.touch();
        Some((message_id, self.abort_flag.clone()))
    }

    pub fn emit_stream_delta(&mut self, ops: Vec<DeltaOp>) {
        let message_id = match &mut self.draft_message {
            Some(draft) => {
                for op in &ops {
                    match op {
                        DeltaOp::AppendContent { text } => match &mut draft.content {
                            ChatContent::SimpleText(s) => s.push_str(text),
                            _ => draft.content = ChatContent::SimpleText(text.clone()),
                        },
                        DeltaOp::AppendReasoning { text } => {
                            let r = draft.reasoning_content.get_or_insert_with(String::new);
                            r.push_str(text);
                        }
                        DeltaOp::SetToolCalls { tool_calls } => {
                            draft.tool_calls = serde_json::from_value(json!(tool_calls)).ok();
                        }
                        DeltaOp::SetThinkingBlocks { blocks } => {
                            draft.thinking_blocks = Some(blocks.clone());
                        }
                        DeltaOp::AddCitation { citation } => {
                            draft.citations.push(citation.clone());
                        }
                        DeltaOp::AddServerContentBlock { block } => {
                            draft.server_content_blocks.push(block.clone());
                        }
                        DeltaOp::SetUsage { usage } => {
                            if let Ok(u) = serde_json::from_value(usage.clone()) {
                                draft.usage = Some(u);
                            }
                        }
                        DeltaOp::MergeExtra { extra } => {
                            draft.extra.extend(extra.clone());
                        }
                    }
                }
                draft.message_id.clone()
            }
            None => return,
        };
        self.emit(ChatEvent::StreamDelta { message_id, ops });
    }

    pub fn finish_stream(&mut self, finish_reason: Option<String>) {
        if let Some(mut draft) = self.draft_message.take() {
            let has_text_content = match &draft.content {
                ChatContent::SimpleText(s) => !s.trim().is_empty(),
                ChatContent::Multimodal(v) => !v.is_empty(),
                ChatContent::ContextFiles(v) => !v.is_empty(),
            };
            let has_structured_data = draft.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty())
                || draft
                    .reasoning_content
                    .as_ref()
                    .map_or(false, |r| !r.trim().is_empty())
                || draft
                    .thinking_blocks
                    .as_ref()
                    .map_or(false, |tb| !tb.is_empty())
                || !draft.citations.is_empty()
                || !draft.server_content_blocks.is_empty();

            self.emit(ChatEvent::StreamFinished {
                message_id: draft.message_id.clone(),
                finish_reason: finish_reason.clone(),
            });

            if has_text_content || has_structured_data {
                draft.finish_reason = finish_reason;
                if let Some(usage) = self.draft_usage.take() {
                    draft.usage = Some(usage);
                }
                self.add_message(draft);
            } else {
                tracing::warn!("Discarding empty assistant message");
                self.emit(ChatEvent::MessageRemoved {
                    message_id: draft.message_id,
                });
            }
        }
        self.set_runtime_state(SessionState::Idle, None);
        self.touch();
    }

    pub fn finish_stream_with_error(&mut self, error: String) {
        if let Some(mut draft) = self.draft_message.take() {
            let has_text_content = match &draft.content {
                ChatContent::SimpleText(s) => !s.is_empty(),
                ChatContent::Multimodal(v) => !v.is_empty(),
                ChatContent::ContextFiles(v) => !v.is_empty(),
            };
            let has_structured_data = draft.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty())
                || draft
                    .reasoning_content
                    .as_ref()
                    .map_or(false, |r| !r.is_empty())
                || draft
                    .thinking_blocks
                    .as_ref()
                    .map_or(false, |tb| !tb.is_empty())
                || !draft.citations.is_empty()
                || !draft.server_content_blocks.is_empty()
                || draft.usage.is_some()
                || !draft.extra.is_empty();

            if has_text_content || has_structured_data {
                self.emit(ChatEvent::StreamFinished {
                    message_id: draft.message_id.clone(),
                    finish_reason: Some("error".to_string()),
                });
                draft.finish_reason = Some("error".to_string());
                if let Some(usage) = self.draft_usage.take() {
                    draft.usage = Some(usage);
                }
                self.add_message(draft);
            } else {
                self.emit(ChatEvent::MessageRemoved {
                    message_id: draft.message_id,
                });
            }
        }
        self.set_runtime_state(SessionState::Error, Some(error.clone()));
        self.touch();

        // Store task_meta for async notification (need to clone before async)
        self.task_agent_error = Some(error);
    }

    pub fn abort_stream(&mut self) {
        self.abort_flag.store(true, Ordering::SeqCst);
        if let Some(draft) = self.draft_message.take() {
            self.emit(ChatEvent::StreamFinished {
                message_id: draft.message_id.clone(),
                finish_reason: Some("abort".to_string()),
            });
            self.emit(ChatEvent::MessageRemoved {
                message_id: draft.message_id,
            });
        }
        self.draft_usage = None;
        self.set_runtime_state(SessionState::Idle, None);
        self.touch();
        self.queue_notify.notify_one();
    }

    pub fn discard_draft_for_pause(&mut self) {
        if let Some(draft) = self.draft_message.take() {
            self.emit(ChatEvent::MessageRemoved {
                message_id: draft.message_id,
            });
        }
        self.draft_usage = None;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.event_tx.subscribe()
    }

    pub fn set_title(&mut self, title: String, is_generated: bool) {
        self.thread.title = title.clone();
        self.thread.is_title_generated = is_generated;
        self.increment_version();
        self.touch();
        self.emit_trajectory_title_change(title);
    }

    fn emit_trajectory_title_change(&self, title: String) {
        if self.thread.task_meta.is_some() {
            return;
        }
        if let Some(ref tx) = self.trajectory_events_tx {
            let effective_root = self.thread.root_chat_id.clone().unwrap_or_else(|| self.chat_id.clone());
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: self.chat_id.clone(),
                updated_at: Some(chrono::Utc::now().to_rfc3339()),
                title: Some(title),
                is_title_generated: Some(self.thread.is_title_generated),
                session_state: Some(self.runtime.state.to_string()),
                error: self.runtime.error.clone(),
                message_count: Some(self.messages.len()),
                parent_id: self.thread.parent_id.clone(),
                link_type: self.thread.link_type.clone(),
                root_chat_id: Some(effective_root),
                model: Some(self.thread.model.clone()),
                mode: Some(self.thread.mode.clone()),
                total_coins: None,
                total_lines_added: None,
                total_lines_removed: None,
                tasks_total: None,
                tasks_done: None,
                tasks_failed: None,
                total_prompt_tokens: None,
                total_completion_tokens: None,
                total_tokens: None,
                total_cache_read_tokens: None,
                total_cache_creation_tokens: None,
                total_cost_usd: None,
            };
            let _ = tx.send(event);
        }
    }

    pub fn validate_tool_decision(&self, tool_call_id: &str) -> bool {
        self.runtime
            .pause_reasons
            .iter()
            .any(|r| r.tool_call_id == tool_call_id)
    }

    pub fn process_tool_decisions(&mut self, decisions: &[ToolDecisionItem]) -> Vec<String> {
        let mut accepted_ids = Vec::new();
        let mut denied_ids = Vec::new();

        for decision in decisions {
            if !self.validate_tool_decision(&decision.tool_call_id) {
                warn!(
                    "Tool decision for unknown tool_call_id: {}",
                    decision.tool_call_id
                );
                continue;
            }
            if decision.accepted {
                accepted_ids.push(decision.tool_call_id.clone());
            } else {
                denied_ids.push(decision.tool_call_id.clone());
            }
        }

        self.runtime.pause_reasons.retain(|r| {
            !accepted_ids.contains(&r.tool_call_id) && !denied_ids.contains(&r.tool_call_id)
        });

        if self.runtime.pause_reasons.is_empty() {
            self.set_runtime_state(SessionState::Idle, None);
        }

        accepted_ids
    }
}

pub async fn get_or_create_session_with_trajectory(
    gcx: Arc<ARwLock<GlobalContext>>,
    sessions: &SessionsMap,
    chat_id: &str,
) -> Arc<AMutex<ChatSession>> {
    let maybe_existing = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    if let Some(session_arc) = maybe_existing {
        let is_closed = {
            let session = session_arc.lock().await;
            session.closed
        };
        if !is_closed {
            return session_arc;
        }
        let mut sessions_write = sessions.write().await;
        if let Some(current) = sessions_write.get(chat_id) {
            if Arc::ptr_eq(current, &session_arc) {
                sessions_write.remove(chat_id);
            }
        }
    }

    let trajectory_events_tx = gcx.read().await.trajectory_events_tx.clone();

    let (mut session, is_new) = if let Some(mut loaded) =
        super::trajectories::load_trajectory_for_chat(gcx.clone(), chat_id).await
    {
        info!(
            "Loaded trajectory for chat {} with {} messages",
            chat_id,
            loaded.messages.len()
        );
        super::trajectories::apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        ).await;
        (
            ChatSession::new_with_trajectory(
                chat_id.to_string(),
                loaded.messages,
                loaded.thread,
                loaded.created_at,
            ),
            false,
        )
    } else {
        let mut s = ChatSession::new(chat_id.to_string());
        s.increment_version();
        (s, true)
    };

    if is_new {
        if let Some(mode_config) = crate::yaml_configs::customization_registry::get_mode_config(
            gcx.clone(),
            &session.thread.mode,
            None,
        ).await {
            let defaults = &mode_config.thread_defaults;
            if let Some(v) = defaults.include_project_info {
                session.thread.include_project_info = v;
            }
            if let Some(v) = defaults.checkpoints_enabled {
                session.thread.checkpoints_enabled = v;
            }
            if let Some(v) = defaults.auto_approve_editing_tools {
                session.thread.auto_approve_editing_tools = v;
            }
            if let Some(v) = defaults.auto_approve_dangerous_commands {
                session.thread.auto_approve_dangerous_commands = v;
            }
        }
    }

    session.trajectory_events_tx = trajectory_events_tx.clone();

    let (session_arc, inserted) = {
        let mut sessions_write = sessions.write().await;
        match sessions_write.entry(chat_id.to_string()) {
            std::collections::hash_map::Entry::Vacant(e) => {
                let arc = Arc::new(AMutex::new(session));
                e.insert(arc.clone());
                (arc, true)
            }
            std::collections::hash_map::Entry::Occupied(e) => {
                (e.get().clone(), false)
            }
        }
    };

    if inserted && is_new {
        let gcx_clone = gcx.clone();
        let chat_id_clone = chat_id.to_string();
        tokio::spawn(async move {
            let project_dir = get_project_dir_string(gcx_clone.clone()).await;
            let payload = HookPayload {
                hook_event_name: "SessionStart".to_string(),
                session_id: chat_id_clone,
                project_dir,
                tool_name: None,
                tool_input: None,
                tool_output: None,
                user_prompt: None,
                extra: std::collections::HashMap::new(),
            };
            run_hooks(gcx_clone, HookEvent::SessionStart, payload).await;
        });
    }

    session_arc
}

pub async fn close_all_chat_sessions(gcx: Arc<ARwLock<GlobalContext>>) {
    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };
    let session_arcs: Vec<Arc<AMutex<ChatSession>>> = {
        let sessions_read = sessions.read().await;
        sessions_read.values().cloned().collect()
    };
    for session_arc in session_arcs {
        let lock_result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            session_arc.lock(),
        ).await;
        match lock_result {
            Ok(mut session) => {
                session.abort_stream();
                session.close_event_channel(); // sets closed + closed_flag
                session.queue_notify.notify_waiters();
            }
            Err(_) => {
                // Could not acquire lock within timeout — notify_waiters best-effort
                // so the queue processor can eventually notice the shutdown flag.
                warn!("close_all_chat_sessions: session lock timeout, notifying waiters without lock");
                session_arc.try_lock().map(|s| s.queue_notify.notify_waiters()).ok();
            }
        }
    }
}

pub fn start_session_cleanup_task(gcx: Arc<ARwLock<GlobalContext>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(session_cleanup_interval());
        loop {
            let shutdown_flag = gcx.read().await.shutdown_flag.clone();
            tokio::select! {
                _ = interval.tick() => {}
                _ = async {
                    while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    }
                } => {
                    tracing::info!("Session cleanup: shutdown detected, stopping");
                    return;
                }
            }

            let sessions = {
                let gcx_locked = gcx.read().await;
                gcx_locked.chat_sessions.clone()
            };

            let candidates: Vec<(String, Arc<AMutex<ChatSession>>)> = {
                let sessions_read = sessions.read().await;
                sessions_read
                    .iter()
                    .map(|(chat_id, session_arc)| (chat_id.clone(), session_arc.clone()))
                    .collect()
            };

            let mut to_cleanup = Vec::new();
            for (chat_id, session_arc) in candidates {
                let session = session_arc.lock().await;
                if session.is_idle_for_cleanup() {
                    drop(session);
                    to_cleanup.push((chat_id, session_arc));
                }
            }

            if to_cleanup.is_empty() {
                continue;
            }

            info!("Cleaning up {} idle sessions", to_cleanup.len());

            for (chat_id, session_arc) in &to_cleanup {
                let gcx_hook = gcx.clone();
                let chat_id_hook = chat_id.clone();
                tokio::spawn(async move {
                    let project_dir = get_project_dir_string(gcx_hook.clone()).await;
                    let payload = HookPayload {
                        hook_event_name: "SessionEnd".to_string(),
                        session_id: chat_id_hook,
                        project_dir,
                        tool_name: None,
                        tool_input: None,
                        tool_output: None,
                        user_prompt: None,
                        extra: std::collections::HashMap::new(),
                    };
                    run_hooks(gcx_hook, HookEvent::SessionEnd, payload).await;
                });

                {
                    let mut session = session_arc.lock().await;
                    session.close_event_channel(); // sets closed + closed_flag
                    session.queue_notify.notify_waiters();
                }
                {
                    let mut sessions_write = sessions.write().await;
                    if let Some(current) = sessions_write.get(chat_id) {
                        if Arc::ptr_eq(current, session_arc) {
                            sessions_write.remove(chat_id);
                        }
                    }
                }
                super::trajectories::maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
                info!("Saved trajectory for closed session {}", chat_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{ChatCommand, CommandRequest};
    use serde_json::json;
    use std::time::Instant;

    fn make_session() -> ChatSession {
        ChatSession::new("test-chat".to_string())
    }

    /// Creates a session with a small broadcast channel capacity, useful for
    /// triggering `RecvError::Lagged` quickly in tests without emitting
    /// thousands of events.
    fn make_session_with_capacity(capacity: usize) -> ChatSession {
        let (event_tx, _) = broadcast::channel(capacity);
        let mut session = ChatSession::new("test-chat-small".to_string());
        session.event_tx = event_tx;
        session
    }

    #[test]
    fn test_new_session_initial_state() {
        let session = make_session();
        assert_eq!(session.chat_id, "test-chat");
        assert_eq!(session.thread.id, "test-chat");
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.messages.is_empty());
        assert!(session.draft_message.is_none());
        assert_eq!(session.event_seq, 0);
        assert!(!session.trajectory_dirty);
    }

    #[test]
    fn test_new_with_trajectory() {
        let msg = ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        };
        let thread = ThreadParams {
            id: "traj-1".into(),
            title: "Old Chat".into(),
            ..Default::default()
        };
        let session = ChatSession::new_with_trajectory(
            "traj-1".into(),
            vec![msg.clone()],
            thread,
            "2024-01-01T00:00:00Z".into(),
        );
        assert_eq!(session.chat_id, "traj-1");
        assert_eq!(session.thread.title, "Old Chat");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.created_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_emit_increments_seq() {
        let mut session = make_session();
        assert_eq!(session.event_seq, 0);
        session.emit(ChatEvent::PauseCleared {});
        assert_eq!(session.event_seq, 1);
        session.emit(ChatEvent::PauseCleared {});
        assert_eq!(session.event_seq, 2);
    }

    #[test]
    fn test_emit_sends_correct_envelope() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        session.emit(ChatEvent::PauseCleared {});
        let envelope = rx.try_recv().unwrap();
        assert_eq!(envelope.chat_id, "test-chat");
        assert_eq!(envelope.seq, 1);
        assert!(matches!(envelope.event, ChatEvent::PauseCleared {}));
    }

    #[test]
    fn test_snapshot_without_draft() {
        let mut session = make_session();
        session.messages.push(ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        });
        let snap = session.snapshot();
        match snap {
            ChatEvent::Snapshot { messages, .. } => {
                assert_eq!(messages.len(), 1);
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_snapshot_includes_draft_when_generating() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        let snap = session.snapshot();
        match snap {
            ChatEvent::Snapshot {
                messages, runtime, ..
            } => {
                assert_eq!(runtime.state, SessionState::Generating);
                assert_eq!(messages.len(), 1);
                match &messages[0].content {
                    ChatContent::SimpleText(s) => assert_eq!(s, "partial"),
                    _ => panic!("Expected SimpleText"),
                }
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_is_duplicate_request_detects_duplicates() {
        let mut session = make_session();
        assert!(!session.is_duplicate_request("req-1"));
        assert!(session.is_duplicate_request("req-1"));
        assert!(!session.is_duplicate_request("req-2"));
        assert!(session.is_duplicate_request("req-2"));
    }

    #[test]
    fn test_is_duplicate_request_caps_at_100() {
        let mut session = make_session();
        for i in 0..100 {
            session.is_duplicate_request(&format!("req-{}", i));
        }
        assert_eq!(session.recent_request_ids.len(), 100);
        session.is_duplicate_request("req-100");
        assert_eq!(session.recent_request_ids.len(), 100);
        assert!(!session.recent_request_ids.contains(&"req-0".to_string()));
        assert!(session.recent_request_ids.contains(&"req-100".to_string()));
    }

    #[test]
    fn test_add_message_generates_id_if_empty() {
        let mut session = make_session();
        let msg = ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        };
        session.add_message(msg);
        assert!(!session.messages[0].message_id.is_empty());
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_add_message_preserves_existing_id() {
        let mut session = make_session();
        let msg = ChatMessage {
            message_id: "custom-id".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        };
        session.add_message(msg);
        assert_eq!(session.messages[0].message_id, "custom-id");
    }

    #[test]
    fn test_update_message_returns_index() {
        let mut session = make_session();
        let msg = ChatMessage {
            message_id: "m1".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("original".into()),
            ..Default::default()
        };
        session.messages.push(msg);
        let updated = ChatMessage {
            message_id: "m1".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("updated".into()),
            ..Default::default()
        };
        let idx = session.update_message("m1", updated);
        assert_eq!(idx, Some(0));
        match &session.messages[0].content {
            ChatContent::SimpleText(s) => assert_eq!(s, "updated"),
            _ => panic!("Expected SimpleText"),
        }
    }

    #[test]
    fn test_update_message_unknown_id_returns_none() {
        let mut session = make_session();
        let msg = ChatMessage::default();
        assert!(session.update_message("unknown", msg).is_none());
    }

    #[test]
    fn test_remove_message_returns_index() {
        let mut session = make_session();
        session.messages.push(ChatMessage {
            message_id: "m1".into(),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            message_id: "m2".into(),
            ..Default::default()
        });
        let idx = session.remove_message("m1");
        assert_eq!(idx, Some(0));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].message_id, "m2");
    }

    #[test]
    fn test_remove_message_unknown_id_returns_none() {
        let mut session = make_session();
        assert!(session.remove_message("unknown").is_none());
    }

    #[test]
    fn test_truncate_messages() {
        let mut session = make_session();
        for i in 0..5 {
            session.messages.push(ChatMessage {
                message_id: format!("m{}", i),
                ..Default::default()
            });
        }
        session.truncate_messages(2);
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].message_id, "m1");
    }

    #[test]
    fn test_truncate_beyond_length_is_noop() {
        let mut session = make_session();
        session.messages.push(ChatMessage::default());
        let version_before = session.trajectory_version;
        session.truncate_messages(10);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.trajectory_version, version_before);
    }

    #[test]
    fn test_start_stream_returns_message_id() {
        let mut session = make_session();
        let result = session.start_stream();
        assert!(result.is_some());
        let (msg_id, abort_flag) = result.unwrap();
        assert!(!msg_id.is_empty());
        assert!(!abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Generating);
        assert!(session.draft_message.is_some());
    }

    #[test]
    fn test_start_stream_fails_if_already_generating() {
        let mut session = make_session();
        session.start_stream();
        let result = session.start_stream();
        assert!(result.is_none());
    }

    #[test]
    fn test_start_stream_fails_if_executing_tools() {
        let mut session = make_session();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let result = session.start_stream();
        assert!(result.is_none());
    }

    #[test]
    fn test_emit_stream_delta_appends_content() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "Hello".into(),
        }]);
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: " World".into(),
        }]);
        let draft = session.draft_message.as_ref().unwrap();
        match &draft.content {
            ChatContent::SimpleText(s) => assert_eq!(s, "Hello World"),
            _ => panic!("Expected SimpleText"),
        }
    }

    #[test]
    fn test_emit_stream_delta_appends_reasoning() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendReasoning {
            text: "think".into(),
        }]);
        session.emit_stream_delta(vec![DeltaOp::AppendReasoning { text: "ing".into() }]);
        let draft = session.draft_message.as_ref().unwrap();
        assert_eq!(draft.reasoning_content.as_ref().unwrap(), "thinking");
    }

    #[test]
    fn test_emit_stream_delta_sets_tool_calls() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::SetToolCalls {
            tool_calls: vec![
                json!({"id":"tc1","type":"function","function":{"name":"test","arguments":"{}"}}),
            ],
        }]);
        let draft = session.draft_message.as_ref().unwrap();
        assert!(draft.tool_calls.is_some());
        assert_eq!(draft.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_emit_stream_delta_without_draft_is_noop() {
        let mut session = make_session();
        session.emit_stream_delta(vec![DeltaOp::AppendContent { text: "x".into() }]);
        assert!(session.draft_message.is_none());
    }

    #[test]
    fn test_finish_stream_adds_message() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "done".into(),
        }]);
        session.finish_stream(Some("stop".into()));
        assert!(session.draft_message.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].finish_reason, Some("stop".into()));
        assert_eq!(session.runtime.state, SessionState::Idle);
    }

    #[test]
    fn test_finish_stream_with_error_keeps_content() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        session.finish_stream_with_error("timeout".into());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].finish_reason, Some("error".into()));
        assert_eq!(session.runtime.state, SessionState::Error);
        assert_eq!(session.runtime.error, Some("timeout".into()));
    }

    #[test]
    fn test_finish_stream_with_error_keeps_structured_data() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::SetToolCalls {
            tool_calls: vec![
                json!({"id":"tc1","type":"function","function":{"name":"test","arguments":"{}"}}),
            ],
        }]);
        session.finish_stream_with_error("error".into());
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn test_finish_stream_with_error_removes_empty_draft() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        session.start_stream();
        session.finish_stream_with_error("error".into());
        assert!(session.messages.is_empty());
        let mut found_removed = false;
        while let Ok(env) = rx.try_recv() {
            if matches!(env.event, ChatEvent::MessageRemoved { .. }) {
                found_removed = true;
            }
        }
        assert!(found_removed);
    }

    #[test]
    fn test_abort_stream() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        session.abort_stream();
        assert!(session.draft_message.is_none());
        assert!(session.messages.is_empty());
        assert!(session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
    }

    #[test]
    fn test_set_runtime_state_clears_pause_on_transition() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        assert!(!session.runtime.pause_reasons.is_empty());
        session.set_runtime_state(SessionState::Idle, None);
        assert!(session.runtime.pause_reasons.is_empty());
    }

    #[test]
    fn test_set_paused_with_reasons_and_auto_approved() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        let reasons = vec![PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "shell".into(),
            rule: "ask".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        }];
        session.set_paused_with_reasons_and_auto_approved(reasons.clone(), vec!["tc2".into()], Some(0));
        assert_eq!(session.runtime.state, SessionState::Paused);
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert_eq!(session.runtime.auto_approved_tool_ids, vec!["tc2".to_string()]);
        assert_eq!(session.runtime.paused_message_index, Some(0));
        let mut found_pause_required = false;
        while let Ok(env) = rx.try_recv() {
            if matches!(env.event, ChatEvent::PauseRequired { .. }) {
                found_pause_required = true;
            }
        }
        assert!(found_pause_required);
    }

    #[test]
    fn test_set_title() {
        let mut session = make_session();
        session.set_title("New Title".into(), true);
        assert_eq!(session.thread.title, "New Title");
        assert!(session.thread.is_title_generated);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_validate_tool_decision() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        assert!(session.validate_tool_decision("tc1"));
        assert!(!session.validate_tool_decision("unknown"));
    }

    #[test]
    fn test_process_tool_decisions_accepts() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc2".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let accepted = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: true,
        }]);
        assert_eq!(accepted, vec!["tc1"]);
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert_eq!(session.runtime.state, SessionState::Paused);
    }

    #[test]
    fn test_process_tool_decisions_denies() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let accepted = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: false,
        }]);
        assert!(accepted.is_empty());
        assert!(session.runtime.pause_reasons.is_empty());
        assert_eq!(session.runtime.state, SessionState::Idle);
    }

    #[test]
    fn test_process_tool_decisions_ignores_unknown() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let accepted = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "unknown".into(),
            accepted: true,
        }]);
        assert!(accepted.is_empty());
        assert_eq!(session.runtime.pause_reasons.len(), 1);
    }

    #[test]
    fn test_process_tool_decisions_transitions_to_idle_when_empty() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: true,
        }]);
        assert!(session.runtime.pause_reasons.is_empty());
        assert_eq!(session.runtime.state, SessionState::Idle);
    }

    #[test]
    fn test_increment_version() {
        let mut session = make_session();
        assert_eq!(session.trajectory_version, 0);
        assert!(!session.trajectory_dirty);
        session.increment_version();
        assert_eq!(session.trajectory_version, 1);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_create_sessions_map() {
        let map = create_sessions_map();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let read = map.read().await;
            assert!(read.is_empty());
        });
    }

    #[test]
    fn test_build_queued_items() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("hello"),
                attachments: vec![],
            },
        });
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-2".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        let items = session.build_queued_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].client_request_id, "req-1");
        assert!(!items[0].priority);
        assert_eq!(items[0].command_type, "user_message");
        assert_eq!(items[1].client_request_id, "req-2");
        assert!(items[1].priority);
        assert_eq!(items[1].command_type, "abort");
    }

    #[test]
    fn test_emit_queue_update_syncs_runtime() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::Abort {},
        });
        session.emit_queue_update();
        assert_eq!(session.runtime.queue_size, 1);
        assert_eq!(session.runtime.queued_items.len(), 1);
    }

    #[test]
    fn test_set_runtime_state_syncs_queued_items() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        session.set_runtime_state(SessionState::Generating, None);
        assert_eq!(session.runtime.queued_items.len(), 1);
        assert_eq!(session.runtime.queued_items[0].client_request_id, "req-1");
    }

    #[test]
    fn test_snapshot_includes_queued_items() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("test"),
                attachments: vec![],
            },
        });
        let snap = session.snapshot();
        match snap {
            ChatEvent::Snapshot { runtime, .. } => {
                assert_eq!(runtime.queue_size, 1);
                assert_eq!(runtime.queued_items.len(), 1);
                assert_eq!(runtime.queued_items[0].client_request_id, "req-1");
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_touch_updates_last_activity() {
        let mut session = make_session();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));
        session.touch();
        assert!(session.last_activity > before);
    }

    #[test]
    fn test_finish_stream_keeps_server_content_blocks_only_message() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![
            DeltaOp::AddServerContentBlock {
                block: json!({
                    "type": "server_tool_use",
                    "id": "srvtoolu_test",
                    "name": "web_search",
                    "input": {"query": "test"}
                }),
            },
            DeltaOp::AddServerContentBlock {
                block: json!({
                    "type": "web_search_tool_result",
                    "tool_use_id": "srvtoolu_test",
                    "content": [{"type": "web_search_result", "title": "Result", "url": "https://example.com"}]
                }),
            },
        ]);
        session.finish_stream(Some("stop".to_string()));

        assert_eq!(session.messages.len(), 1,
            "Server-blocks-only assistant message should be preserved");
        assert_eq!(session.messages[0].server_content_blocks.len(), 2);
        assert_eq!(session.messages[0].role, "assistant");
    }

    #[test]
    fn test_finish_stream_discards_truly_empty_message() {
        let mut session = make_session();
        session.start_stream();
        // No deltas at all
        session.finish_stream(Some("stop".to_string()));

        assert_eq!(session.messages.len(), 0,
            "Truly empty assistant message should be discarded");
    }

    /// Regression test: after a broadcast::Receiver lags, the handler must
    /// re-subscribe (`rx = session.subscribe()`) before capturing `event_seq`
    /// for the recovery snapshot.  Without re-subscribing, the old receiver
    /// resumes from the oldest ring-buffer entry whose seq is *lower* than the
    /// snapshot seq, causing the frontend to silently drop every subsequent event.
    ///
    /// This test simulates the handler's Lagged recovery path and asserts that
    /// the first event received after the snapshot has seq == snapshot_seq + 1.
    #[tokio::test]
    async fn test_lagged_recovery_seq_monotonicity() {
        use tokio::sync::broadcast::error::RecvError;

        // Use a tiny channel capacity so we only need to emit a handful of
        // events to trigger Lagged rather than the default 4096+.
        const SMALL_CAP: usize = 8;
        let mut session = make_session_with_capacity(SMALL_CAP);

        // Subscribe a "slow" receiver that we will intentionally lag.
        let mut slow_rx = session.subscribe();

        // Emit capacity+1 events so slow_rx is guaranteed to lag.
        let overflow_count = SMALL_CAP + 1;
        for _ in 0..overflow_count {
            session.emit(ChatEvent::QueueUpdated {
                queue_size: 0,
                queued_items: vec![],
            });
        }

        // Confirm that slow_rx is lagged.
        assert!(
            matches!(slow_rx.recv().await, Err(RecvError::Lagged(_))),
            "slow_rx should be lagged after overflow"
        );

        // --- Simulate the handler's recovery path ---
        // After Lagged, the handler must:
        //   1. Lock the session
        //   2. Re-subscribe to get a fresh receiver
        //   3. Capture event_seq for the recovery snapshot
        //   4. Drop the lock
        //   5. Emit one more event (from some background task)
        //   6. Assert first recv() on fresh_rx has seq == snapshot_seq + 1

        // Step 2-3: re-subscribe while holding the "lock" (single-threaded here).
        let mut fresh_rx = session.subscribe();
        let snapshot_seq = session.event_seq;

        // Step 5: emit one more event (e.g. a RuntimeUpdated broadcast).
        session.emit(ChatEvent::QueueUpdated {
            queue_size: 0,
            queued_items: vec![],
        });

        // Step 6: the first event from fresh_rx must have seq == snapshot_seq + 1.
        let envelope = fresh_rx
            .recv()
            .await
            .expect("fresh_rx should receive an event");

        assert_eq!(
            envelope.seq,
            snapshot_seq + 1,
            "First event after re-subscribe must have seq == snapshot_seq + 1, \
             got {} (snapshot_seq={}). \
             If seq < snapshot_seq the frontend drops all events forever.",
            envelope.seq,
            snapshot_seq
        );
    }

    #[test]
    #[ignore]
    fn stress_emit_and_snapshot_large_history_baseline() {
        const MESSAGE_COUNT: usize = 2_000;
        const MESSAGE_SIZE: usize = 2_048;
        const SNAPSHOT_RUNS: usize = 200;

        let mut session = make_session();

        for i in 0..MESSAGE_COUNT {
            session.add_message(ChatMessage {
                message_id: format!("m{}", i),
                role: if i % 2 == 0 { "user".to_string() } else { "assistant".to_string() },
                content: ChatContent::SimpleText("x".repeat(MESSAGE_SIZE)),
                ..Default::default()
            });
        }

        let emit_start = Instant::now();
        for _ in 0..1_500 {
            session.emit(ChatEvent::QueueUpdated {
                queue_size: 0,
                queued_items: vec![],
            });
        }
        let emit_elapsed = emit_start.elapsed();

        let snapshot_start = Instant::now();
        for _ in 0..SNAPSHOT_RUNS {
            let snapshot = session.snapshot();
            if let ChatEvent::Snapshot { messages, .. } = snapshot {
                assert_eq!(messages.len(), MESSAGE_COUNT);
            } else {
                panic!("Expected Snapshot event");
            }
        }
        let snapshot_elapsed = snapshot_start.elapsed();

        println!(
            "STRESS_BASELINE session_emit_snapshot: messages={}, msg_size={}, emits=1500, snapshots={}, emit_ms={}, snapshot_ms={}",
            MESSAGE_COUNT,
            MESSAGE_SIZE,
            SNAPSHOT_RUNS,
            emit_elapsed.as_millis(),
            snapshot_elapsed.as_millis(),
        );
    }

    #[test]
    #[ignore]
    fn stress_broadcast_lag_recovery_baseline() {
        let event_count = limits().event_channel_capacity * 3;

        let mut session = make_session();
        let mut slow_rx = session.subscribe();

        let emit_start = Instant::now();
        for i in 0..event_count {
            session.emit(ChatEvent::MessageAdded {
                message: ChatMessage {
                    message_id: format!("lag-{}", i),
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText("delta".to_string()),
                    ..Default::default()
                },
                index: i,
            });
        }
        let emit_elapsed = emit_start.elapsed();

        let recv_start = Instant::now();
        let mut received = 0usize;
        let mut lagged = 0usize;
        loop {
            match slow_rx.try_recv() {
                Ok(_envelope) => {
                    received += 1;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_skipped)) => {
                    lagged += 1;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    break;
                }
            }
        }
        let recv_elapsed = recv_start.elapsed();

        assert!(lagged > 0, "Expected lagged receiver under saturation");

        println!(
            "STRESS_BASELINE broadcast_lag: emitted={}, received={}, lagged_events={}, emit_ms={}, drain_ms={}, channel_capacity={}",
            event_count,
            received,
            lagged,
            emit_elapsed.as_millis(),
            recv_elapsed.as_millis(),
            limits().event_channel_capacity,
        );
    }

    #[test]
    fn test_active_command_initial_default() {
        let session = make_session();
        assert!(session.active_command.context_fork.is_none());
        assert!(session.active_command.model_override.is_none());
        assert!(session.active_command.allowed_tools.is_empty());
        assert!(session.active_command.name.is_empty());
    }

    #[test]
    fn test_active_command_stored_and_cleared() {
        let mut session = make_session();
        session.active_command = ActiveCommandContext {
            name: "my-agent".to_string(),
            allowed_tools: vec!["cat".to_string()],
            model_override: Some("gpt-4".to_string()),
            context_fork: Some("subagent".to_string()),
            started_at_index: None,
            activation_tool_call_id: None,
        };
        assert_eq!(session.active_command.context_fork, Some("subagent".to_string()));
        assert_eq!(session.active_command.name, "my-agent");
        session.active_command = ActiveCommandContext::default();
        assert!(session.active_command.context_fork.is_none());
        assert!(session.active_command.name.is_empty());
    }

    #[test]
    fn test_new_with_trajectory_active_command_default() {
        use crate::call_validation::{ChatContent};
        let msg = ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        };
        let thread = ThreadParams {
            id: "traj-fork".into(),
            ..Default::default()
        };
        let session = ChatSession::new_with_trajectory(
            "traj-fork".into(),
            vec![msg],
            thread,
            "2024-01-01T00:00:00Z".into(),
        );
        assert!(session.active_command.context_fork.is_none());
        assert!(session.active_command.model_override.is_none());
    }

    #[test]
    fn test_set_clear_active_skill() {
        let mut session = make_session();
        assert!(session.thread.active_skill.is_none());
        session.set_active_skill("test-skill".to_string());
        assert_eq!(session.thread.active_skill, Some("test-skill".to_string()));
        assert!(session.trajectory_dirty);
        session.clear_active_skill();
        assert!(session.thread.active_skill.is_none());
    }

    fn make_user_message(text: &str) -> ChatMessage {
        ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content: crate::call_validation::ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_skill_deactivation_cleanup_compacts_messages() {
        let mut session = make_session();

        // Add 2 pre-skill messages
        session.add_message(make_user_message("pre-skill message 1"));
        session.add_message(make_user_message("pre-skill message 2"));
        let anchor = session.messages.len(); // = 2

        // Add skill-run messages that should be removed
        session.add_message(make_user_message("skill run message A"));
        session.add_message(make_user_message("skill run message B"));
        session.add_message(make_user_message("skill run message C"));
        assert_eq!(session.messages.len(), 5);

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: anchor,
            skill_name: "my-skill".to_string(),
            report: "Did useful things.".to_string(),
            activation_tool_call_id: None,
        });

        session.perform_skill_deactivation_cleanup();

        // 2 pre-skill messages + 1 report = 3 total
        assert_eq!(session.messages.len(), 3, "Expected 2 pre-skill + 1 report");
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, "plain_text");
        if let crate::call_validation::ChatContent::SimpleText(ref text) = last.content {
            assert!(text.contains("## Skill Report: my-skill"), "Report header missing: {}", text);
            assert!(text.contains("Skill 'my-skill' executed successfully"), "Report preface missing: {}", text);
            assert!(text.contains("Did useful things."), "Report body missing: {}", text);
        } else {
            panic!("Expected SimpleText content in report message");
        }
        // pending is consumed
        assert!(session.pending_skill_deactivation.is_none());
    }

    #[test]
    fn test_skill_deactivation_cleanup_noop_when_no_pending() {
        let mut session = make_session();
        session.add_message(make_user_message("msg1"));
        session.add_message(make_user_message("msg2"));

        session.perform_skill_deactivation_cleanup();
        // Nothing changed
        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn test_skill_deactivation_keeps_activation_tool_message() {
        let mut session = make_session();

        session.add_message(make_user_message("pre-skill"));
        let anchor = session.messages.len();

        let tool_message = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("Skill activated".to_string()),
            tool_call_id: "call_activate_skill".to_string(),
            tool_failed: Some(false),
            ..Default::default()
        };
        session.add_message(tool_message);

        session.add_message(ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "context_file".to_string(),
            content: ChatContent::SimpleText("Skill body".to_string()),
            ..Default::default()
        });
        session.add_message(make_user_message("skill run"));

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: anchor,
            skill_name: "tool-skill".to_string(),
            report: "Wrapped up".to_string(),
            activation_tool_call_id: Some("call_activate_skill".to_string()),
        });

        session.perform_skill_deactivation_cleanup();

        assert_eq!(session.messages.len(), 3, "Expected pre-skill + tool + report");
        assert_eq!(session.messages[1].role, "tool", "Activation tool message must remain");
        assert_eq!(session.messages[1].tool_call_id, "call_activate_skill");
        assert_eq!(session.messages.last().unwrap().role, "plain_text");
    }

    #[test]
    fn test_skill_deactivation_skips_exact_activation_tool_call_id() {
        let mut session = make_session();

        session.add_message(make_user_message("pre-skill"));
        let anchor = session.messages.len();

        let unrelated_tool = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("Unrelated tool".to_string()),
            tool_call_id: "call_other_tool".to_string(),
            tool_failed: Some(false),
            ..Default::default()
        };
        session.add_message(unrelated_tool);

        let activation_tool = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("Skill activated".to_string()),
            tool_call_id: "call_activate_skill".to_string(),
            tool_failed: Some(false),
            ..Default::default()
        };
        session.add_message(activation_tool);

        session.add_message(ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "cd_instruction".to_string(),
            content: ChatContent::SimpleText("Skill body".to_string()),
            ..Default::default()
        });
        session.add_message(make_user_message("skill run"));

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: anchor,
            skill_name: "tool-skill".to_string(),
            report: "Wrapped up".to_string(),
            activation_tool_call_id: Some("call_activate_skill".to_string()),
        });

        session.perform_skill_deactivation_cleanup();

        assert_eq!(session.messages.len(), 3, "Expected pre-skill + activation tool + report");
        assert_eq!(session.messages[1].tool_call_id, "call_activate_skill");
    }

    #[test]
    fn test_skill_deactivation_without_anchor_still_records_report() {
        let mut session = make_session();
        session.add_message(make_user_message("pre-skill"));

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: session.messages.len(),
            skill_name: "no-anchor".to_string(),
            report: "All done".to_string(),
            activation_tool_call_id: None,
        });

        session.perform_skill_deactivation_cleanup();

        assert_eq!(session.messages.len(), 2, "Expected pre-skill + report");
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, "plain_text");
        if let crate::call_validation::ChatContent::SimpleText(ref text) = last.content {
            assert!(text.contains("## Skill Report: no-anchor"));
            assert!(text.contains("All done"));
        }
    }

    #[test]
    fn test_skill_deactivation_cleanup_rejects_out_of_range_index() {
        let mut session = make_session();
        session.add_message(make_user_message("only message"));
        assert_eq!(session.messages.len(), 1);

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: 99, // beyond messages.len()
            skill_name: "bad-skill".to_string(),
            report: "report".to_string(),
            activation_tool_call_id: None,
        });

        session.perform_skill_deactivation_cleanup();

        // No truncation, no report added — skipped with warning
        assert_eq!(session.messages.len(), 1, "Messages must not be modified on bad index");
        assert!(session.pending_skill_deactivation.is_none(), "pending must be consumed even on skip");
    }

    #[test]
    fn test_new_with_trajectory_clears_active_skill() {
        let thread = ThreadParams {
            id: "t1".into(),
            active_skill: Some("leftover-skill".to_string()),
            ..Default::default()
        };
        let session = ChatSession::new_with_trajectory(
            "t1".into(),
            vec![],
            thread,
            "2024-01-01T00:00:00Z".into(),
        );
        assert!(
            session.thread.active_skill.is_none(),
            "active_skill must be cleared on restore: compaction anchor is lost after restart"
        );
    }
}
