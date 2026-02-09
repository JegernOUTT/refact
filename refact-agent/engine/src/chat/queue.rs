use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::warn;
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;

use super::types::*;
use super::content::parse_content_with_attachments;
use super::generation::{start_generation, prepare_session_preamble_and_knowledge};
use super::tools::execute_tools_with_session;
use super::trajectories::maybe_save_trajectory;

fn command_triggers_generation(cmd: &ChatCommand) -> bool {
    matches!(
        cmd,
        ChatCommand::UserMessage { .. }
            | ChatCommand::RetryFromIndex { .. }
            | ChatCommand::Regenerate {}
    )
}

pub async fn inject_priority_messages_if_any(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    let priority_requests = {
        let mut session = session_arc.lock().await;
        let requests = drain_priority_user_messages(&mut session.command_queue);
        if !requests.is_empty() {
            session.emit_queue_update();
        }
        requests
    };

    if priority_requests.is_empty() {
        return false;
    }

    for request in priority_requests {
        if let ChatCommand::UserMessage {
            content,
            attachments,
        } = request.command
        {
            // Extract data needed for checkpoint creation while holding the lock briefly
            let (checkpoints_enabled, chat_id, latest_checkpoint) = {
                let session = session_arc.lock().await;
                (
                    session.thread.checkpoints_enabled,
                    session.chat_id.clone(),
                    find_latest_checkpoint(&session),
                )
            };

            // Create checkpoint without holding the session lock (can be slow)
            let checkpoints = if checkpoints_enabled {
                create_checkpoint_async(gcx.clone(), latest_checkpoint.as_ref(), &chat_id).await
            } else {
                Vec::new()
            };

            // Reacquire lock to add the message
            let mut session = session_arc.lock().await;
            let parsed_content = parse_content_with_attachments(&content, &attachments);
            let user_message = ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "user".to_string(),
                content: parsed_content,
                checkpoints,
                ..Default::default()
            };
            session.add_message(user_message);
        }
    }

    maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
    true
}

pub fn find_allowed_command_while_paused(queue: &VecDeque<CommandRequest>) -> Option<usize> {
    for (i, req) in queue.iter().enumerate() {
        match &req.command {
            ChatCommand::ToolDecision { .. }
            | ChatCommand::ToolDecisions { .. }
            | ChatCommand::Abort {} => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

pub fn find_allowed_command_while_waiting_ide(queue: &VecDeque<CommandRequest>) -> Option<usize> {
    for (i, req) in queue.iter().enumerate() {
        match &req.command {
            ChatCommand::IdeToolResult { .. } | ChatCommand::Abort {} => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

pub fn drain_priority_user_messages(queue: &mut VecDeque<CommandRequest>) -> Vec<CommandRequest> {
    let mut priority_messages = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        if queue[i].priority && matches!(queue[i].command, ChatCommand::UserMessage { .. }) {
            if let Some(req) = queue.remove(i) {
                priority_messages.push(req);
            }
        } else {
            i += 1;
        }
    }
    priority_messages
}

pub fn drain_non_priority_user_messages(
    queue: &mut VecDeque<CommandRequest>,
) -> Vec<CommandRequest> {
    let mut messages = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        if !queue[i].priority && matches!(queue[i].command, ChatCommand::UserMessage { .. }) {
            if let Some(req) = queue.remove(i) {
                messages.push(req);
            }
        } else {
            i += 1;
        }
    }
    messages
}

pub fn apply_setparams_patch(
    thread: &mut ThreadParams,
    patch: &serde_json::Value,
) -> (bool, serde_json::Value) {
    let mut changed = false;

    if let Some(model) = patch.get("model").and_then(|v| v.as_str()) {
        if thread.model != model {
            thread.model = model.to_string();
            changed = true;
        }
    }
    if let Some(mode) = patch.get("mode").and_then(|v| v.as_str()) {
        if thread.mode != mode {
            thread.mode = mode.to_string();
            changed = true;
        }
    }
    if let Some(boost) = patch.get("boost_reasoning").and_then(|v| v.as_bool()) {
        if thread.boost_reasoning != Some(boost) {
            thread.boost_reasoning = Some(boost);
            changed = true;
        }
    }
    if let Some(effort_val) = patch.get("reasoning_effort") {
        let new_val = if effort_val.is_null() {
            None
        } else if let Some(effort) = effort_val.as_str() {
            if effort.is_empty() { None } else { Some(effort.to_string()) }
        } else {
            thread.reasoning_effort.clone()
        };
        if thread.reasoning_effort != new_val {
            thread.reasoning_effort = new_val;
            changed = true;
        }
    }
    if let Some(budget_val) = patch.get("thinking_budget") {
        if budget_val.is_null() {
            if thread.thinking_budget.is_some() {
                thread.thinking_budget = None;
                changed = true;
            }
        } else if let Some(b) = budget_val.as_u64() {
            let new_val = Some(b as usize);
            if thread.thinking_budget != new_val {
                thread.thinking_budget = new_val;
                changed = true;
            }
        }
    }
    if let Some(temp_val) = patch.get("temperature") {
        if temp_val.is_null() {
            if thread.temperature.is_some() {
                thread.temperature = None;
                changed = true;
            }
        } else if let Some(t) = temp_val.as_f64() {
            let new_val = Some((t as f32).clamp(0.0, 2.0));
            if thread.temperature != new_val {
                thread.temperature = new_val;
                changed = true;
            }
        }
        // Invalid type (not null, not number) - ignore, keep current value
    }
    if let Some(freq_val) = patch.get("frequency_penalty") {
        if freq_val.is_null() {
            if thread.frequency_penalty.is_some() {
                thread.frequency_penalty = None;
                changed = true;
            }
        } else if let Some(f) = freq_val.as_f64() {
            let new_val = Some((f as f32).clamp(-2.0, 2.0));
            if thread.frequency_penalty != new_val {
                thread.frequency_penalty = new_val;
                changed = true;
            }
        }
        // Invalid type - ignore
    }
    if let Some(max_val) = patch.get("max_tokens") {
        if max_val.is_null() {
            if thread.max_tokens.is_some() {
                thread.max_tokens = None;
                changed = true;
            }
        } else if let Some(m) = max_val.as_u64() {
            let new_val = Some((m as usize).min(1_000_000));
            if thread.max_tokens != new_val {
                thread.max_tokens = new_val;
                changed = true;
            }
        }
        // Invalid type - ignore
    }
    if let Some(parallel_val) = patch.get("parallel_tool_calls") {
        if parallel_val.is_null() {
            if thread.parallel_tool_calls.is_some() {
                thread.parallel_tool_calls = None;
                changed = true;
            }
        } else if let Some(p) = parallel_val.as_bool() {
            let new_val = Some(p);
            if thread.parallel_tool_calls != new_val {
                thread.parallel_tool_calls = new_val;
                changed = true;
            }
        }
        // Invalid type - ignore
    }
    if let Some(tool_use) = patch.get("tool_use").and_then(|v| v.as_str()) {
        if thread.tool_use != tool_use {
            thread.tool_use = tool_use.to_string();
            changed = true;
        }
    }
    if let Some(cap) = patch.get("context_tokens_cap") {
        if cap.is_null() {
            if thread.context_tokens_cap.is_some() {
                thread.context_tokens_cap = None;
                changed = true;
            }
        } else if let Some(n) = cap.as_u64() {
            let new_cap = Some(n as usize);
            if thread.context_tokens_cap != new_cap {
                thread.context_tokens_cap = new_cap;
                changed = true;
            }
        }
        // Invalid type (not null, not number) - ignore, keep current value
    }
    if let Some(include) = patch.get("include_project_info").and_then(|v| v.as_bool()) {
        if thread.include_project_info != include {
            thread.include_project_info = include;
            changed = true;
        }
    }
    if let Some(enabled) = patch.get("checkpoints_enabled").and_then(|v| v.as_bool()) {
        if thread.checkpoints_enabled != enabled {
            thread.checkpoints_enabled = enabled;
            changed = true;
        }
    }
    if let Some(val) = patch.get("auto_approve_editing_tools").and_then(|v| v.as_bool()) {
        if thread.auto_approve_editing_tools != val {
            thread.auto_approve_editing_tools = val;
            changed = true;
        }
    }
    if let Some(val) = patch.get("auto_approve_dangerous_commands").and_then(|v| v.as_bool()) {
        if thread.auto_approve_dangerous_commands != val {
            thread.auto_approve_dangerous_commands = val;
            changed = true;
        }
    }
    if let Some(task_meta_value) = patch.get("task_meta") {
        if !task_meta_value.is_null() {
            if let Ok(task_meta) =
                serde_json::from_value::<super::types::TaskMeta>(task_meta_value.clone())
            {
                thread.task_meta = Some(task_meta);
                changed = true;
            }
        }
    }
    if let Some(parent_id) = patch.get("parent_id").and_then(|v| v.as_str()) {
        let new_val = if parent_id.is_empty() { None } else { Some(parent_id.to_string()) };
        if thread.parent_id != new_val {
            thread.parent_id = new_val;
            changed = true;
        }
    }
    if let Some(link_type) = patch.get("link_type").and_then(|v| v.as_str()) {
        let new_val = if link_type.is_empty() { None } else { Some(link_type.to_string()) };
        if thread.link_type != new_val {
            thread.link_type = new_val;
            changed = true;
        }
    }
    if let Some(root_chat_id) = patch.get("root_chat_id").and_then(|v| v.as_str()) {
        let new_val = if root_chat_id.is_empty() { None } else { Some(root_chat_id.to_string()) };
        if thread.root_chat_id != new_val {
            thread.root_chat_id = new_val;
            changed = true;
        }
    }

    let mut sanitized_patch = patch.clone();
    if let Some(obj) = sanitized_patch.as_object_mut() {
        obj.remove("type");
        obj.remove("chat_id");
        obj.remove("seq");
    }

    (changed, sanitized_patch)
}

pub async fn process_command_queue(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    processor_running: Arc<AtomicBool>,
) {
    struct ProcessorGuard(Arc<AtomicBool>);
    impl Drop for ProcessorGuard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = ProcessorGuard(processor_running);

    loop {
        let command = {
            let mut session = session_arc.lock().await;

            if session.closed {
                return;
            }

            let state = session.runtime.state;
            let is_busy =
                state == SessionState::Generating || state == SessionState::ExecutingTools;

            let notify = session.queue_notify.clone();
            let waiter = notify.notified();

            if is_busy {
                drop(session);
                waiter.await;
                continue;
            }

            if state == SessionState::WaitingIde {
                if let Some(idx) = find_allowed_command_while_waiting_ide(&session.command_queue) {
                    let cmd = session.command_queue.remove(idx);
                    session.emit_queue_update();
                    cmd
                } else {
                    drop(session);
                    waiter.await;
                    continue;
                }
            } else if state == SessionState::Paused {
                if let Some(idx) = find_allowed_command_while_paused(&session.command_queue) {
                    let cmd = session.command_queue.remove(idx);
                    session.emit_queue_update();
                    cmd
                } else {
                    drop(session);
                    waiter.await;
                    continue;
                }
            } else if session.command_queue.is_empty() {
                let closed = session.closed;
                drop(session);

                if closed {
                    return;
                }

                maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

                let session = session_arc.lock().await;
                if session.closed {
                    return;
                }
                if session.command_queue.is_empty() {
                    let waiter2 = notify.notified();
                    drop(session);
                    waiter2.await;
                    continue;
                }
                drop(session);
                continue;
            } else {
                let cmd = session.command_queue.pop_front();
                if let Some(ref req) = cmd {
                    if command_triggers_generation(&req.command) {
                        session.runtime.state = SessionState::Generating;
                    }
                }
                session.emit_queue_update();
                cmd
            }
        };

        let Some(request) = command else {
            continue;
        };

        match request.command {
            ChatCommand::UserMessage {
                content,
                attachments,
            } => {
                let additional_messages = if !request.priority {
                    let mut session = session_arc.lock().await;
                    let msgs = drain_non_priority_user_messages(&mut session.command_queue);
                    if !msgs.is_empty() {
                        session.emit_queue_update();
                    }
                    msgs
                } else {
                    Vec::new()
                };

                // Extract data needed for checkpoint creation while holding the lock briefly
                let (checkpoints_enabled, chat_id, latest_checkpoint) = {
                    let session = session_arc.lock().await;
                    (
                        session.thread.checkpoints_enabled,
                        session.chat_id.clone(),
                        find_latest_checkpoint(&session),
                    )
                };

                // Create checkpoint without holding the session lock (can be slow)
                let checkpoints = if checkpoints_enabled {
                    create_checkpoint_async(gcx.clone(), latest_checkpoint.as_ref(), &chat_id).await
                } else {
                    Vec::new()
                };

                // Reacquire lock to add messages
                {
                    let mut session = session_arc.lock().await;
                    let parsed_content = parse_content_with_attachments(&content, &attachments);
                    let user_message = ChatMessage {
                        message_id: Uuid::new_v4().to_string(),
                        role: "user".to_string(),
                        content: parsed_content,
                        checkpoints,
                        ..Default::default()
                    };
                    session.add_message(user_message);

                    for additional in additional_messages {
                        if let ChatCommand::UserMessage {
                            content: add_content,
                            attachments: add_attachments,
                        } = additional.command
                        {
                            let add_parsed =
                                parse_content_with_attachments(&add_content, &add_attachments);
                            let add_message = ChatMessage {
                                message_id: Uuid::new_v4().to_string(),
                                role: "user".to_string(),
                                content: add_parsed,
                                ..Default::default()
                            };
                            session.add_message(add_message);
                        }
                    }
                }

                maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
                prepare_session_preamble_and_knowledge(gcx.clone(), session_arc.clone()).await;
                start_generation(gcx.clone(), session_arc.clone()).await;
            }
            ChatCommand::RetryFromIndex {
                index,
                content,
                attachments,
            } => {
                let mut session = session_arc.lock().await;
                session.truncate_messages(index);
                let parsed_content = parse_content_with_attachments(&content, &attachments);
                let user_message = ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "user".to_string(),
                    content: parsed_content,
                    ..Default::default()
                };
                session.add_message(user_message);
                drop(session);

                maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
                prepare_session_preamble_and_knowledge(gcx.clone(), session_arc.clone()).await;
                start_generation(gcx.clone(), session_arc.clone()).await;
            }
            ChatCommand::SetParams { patch } => {
                if !patch.is_object() {
                    warn!("SetParams patch must be an object, ignoring");
                    continue;
                }
                let mut session = session_arc.lock().await;
                let (mut changed, sanitized_patch) =
                    apply_setparams_patch(&mut session.thread, &patch);

                let title_in_patch = patch.get("title").and_then(|v| v.as_str());
                let is_gen_in_patch = patch.get("is_title_generated").and_then(|v| v.as_bool());
                if let Some(title) = title_in_patch {
                    let is_generated = is_gen_in_patch.unwrap_or(false);
                    session.set_title(title.to_string(), is_generated);
                } else if let Some(is_gen) = is_gen_in_patch {
                    if session.thread.is_title_generated != is_gen {
                        let title = session.thread.title.clone();
                        session.set_title(title, is_gen);
                        changed = true;
                    }
                }
                let mut patch_for_chat_sse = sanitized_patch;
                if let Some(obj) = patch_for_chat_sse.as_object_mut() {
                    obj.remove("title");
                    obj.remove("is_title_generated");
                }
                session.emit(ChatEvent::ThreadUpdated {
                    params: patch_for_chat_sse,
                });
                if changed {
                    session.increment_version();
                    session.touch();
                }
            }
            ChatCommand::Abort {} => {
                let mut session = session_arc.lock().await;
                session.abort_stream();
            }
            ChatCommand::ToolDecision {
                tool_call_id,
                accepted,
            } => {
                let decisions = vec![ToolDecisionItem {
                    tool_call_id: tool_call_id.clone(),
                    accepted,
                }];
                handle_tool_decisions(gcx.clone(), session_arc.clone(), &decisions).await;
            }
            ChatCommand::ToolDecisions { decisions } => {
                handle_tool_decisions(gcx.clone(), session_arc.clone(), &decisions).await;
            }
            ChatCommand::IdeToolResult {
                tool_call_id,
                content,
                tool_failed,
            } => {
                let mut session = session_arc.lock().await;
                let tool_message = ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(content),
                    tool_call_id,
                    tool_failed: Some(tool_failed),
                    ..Default::default()
                };
                session.add_message(tool_message);
                session.set_runtime_state(SessionState::Idle, None);
                drop(session);
                start_generation(gcx.clone(), session_arc.clone()).await;
            }
            ChatCommand::UpdateMessage {
                message_id,
                content,
                attachments,
                regenerate,
            } => {
                let mut session = session_arc.lock().await;
                if session.runtime.state == SessionState::Generating {
                    session.abort_stream();
                }
                let parsed_content = parse_content_with_attachments(&content, &attachments);
                if let Some(idx) = session
                    .messages
                    .iter()
                    .position(|m| m.message_id == message_id)
                {
                    let mut updated_msg = session.messages[idx].clone();
                    updated_msg.content = parsed_content;
                    session.update_message(&message_id, updated_msg);
                    if regenerate && idx + 1 < session.messages.len() {
                        session.truncate_messages(idx + 1);
                        drop(session);
                        maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
                        prepare_session_preamble_and_knowledge(gcx.clone(), session_arc.clone()).await;
                        start_generation(gcx.clone(), session_arc.clone()).await;
                    }
                }
            }
            ChatCommand::RemoveMessage {
                message_id,
                regenerate,
            } => {
                let mut session = session_arc.lock().await;
                if session.runtime.state == SessionState::Generating {
                    session.abort_stream();
                }
                if let Some(idx) = session.remove_message(&message_id) {
                    if regenerate && idx < session.messages.len() {
                        session.truncate_messages(idx);
                        drop(session);
                        maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
                        prepare_session_preamble_and_knowledge(gcx.clone(), session_arc.clone()).await;
                        start_generation(gcx.clone(), session_arc.clone()).await;
                    }
                }
            }
            ChatCommand::Regenerate {} => {
                prepare_session_preamble_and_knowledge(gcx.clone(), session_arc.clone()).await;
                start_generation(gcx.clone(), session_arc.clone()).await;
            }
            ChatCommand::RestoreMessages { messages } => {
                let mut session = session_arc.lock().await;
                for msg_value in messages {
                    if let Ok(msg) = serde_json::from_value::<ChatMessage>(msg_value) {
                        if !is_allowed_role_for_restore(&msg.role) {
                            continue;
                        }
                        let sanitized = sanitize_message_for_restore(&msg);
                        session.add_message(sanitized);
                    }
                }
                drop(session);
                maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
            }
            ChatCommand::BranchFromChat { source_chat_id, up_to_message_id } => {
                if let Err(e) = super::trajectories::validate_trajectory_id(&source_chat_id) {
                    warn!("BranchFromChat: invalid source_chat_id: {}", e);
                    continue;
                }

                let sessions = {
                    let gcx_locked = gcx.read().await;
                    gcx_locked.chat_sessions.clone()
                };

                let source_session_arc = super::session::get_or_create_session_with_trajectory(
                    gcx.clone(),
                    &sessions,
                    &source_chat_id,
                ).await;

                let (messages_to_copy, root_id) = {
                    let source_session = source_session_arc.lock().await;
                    let mut msgs = Vec::new();
                    let mut found = false;
                    for m in &source_session.messages {
                        if is_allowed_role_for_restore(&m.role) {
                            msgs.push(sanitize_message_for_restore(m));
                        }
                        if m.message_id == up_to_message_id {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        warn!("BranchFromChat: up_to_message_id '{}' not found in source chat", up_to_message_id);
                        continue;
                    }
                    let root = source_session.thread.root_chat_id.clone()
                        .unwrap_or_else(|| source_chat_id.clone());
                    (msgs, root)
                };

                let mut session = session_arc.lock().await;
                session.thread.parent_id = Some(source_chat_id.clone());
                session.thread.link_type = Some("branch".to_string());
                session.thread.root_chat_id = Some(root_id);

                for msg in messages_to_copy {
                    session.add_message(msg);
                }
                drop(session);
                maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
            }
        }
    }
}

fn is_allowed_role_for_restore(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "system" | "tool")
}

/// Sanitize message for branching - preserves conversation structure but strips:
/// - tool_calls from assistant messages (security: prevents prerun of injected tool calls)
/// - transient metadata (usage, checkpoints, etc.)
fn sanitize_message_for_restore(msg: &ChatMessage) -> ChatMessage {
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: msg.role.clone(),
        content: msg.content.clone(),
        tool_calls: None,  // Security: strip tool_calls to prevent prerun of restored messages
        tool_call_id: msg.tool_call_id.clone(),  // Preserve for tool result messages
        tool_failed: msg.tool_failed,  // Preserve tool execution status
        usage: None,  // Strip metering data
        checkpoints: vec![],  // Strip checkpoint data
        reasoning_content: msg.reasoning_content.clone(),
        thinking_blocks: msg.thinking_blocks.clone(),
        citations: msg.citations.clone(),  // Preserve citations (e.g., from web_search)
        finish_reason: None,  // Strip finish reason
        extra: serde_json::Map::new(),  // Strip extra provider-specific data
        output_filter: None,
    }
}

async fn handle_tool_decisions(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
) {
    let (auto_approved_ids, has_remaining_pauses, tool_calls_to_execute, messages, thread, any_rejected) = {
        let mut session = session_arc.lock().await;
        let auto_approved = session.runtime.auto_approved_tool_ids.clone();
        let paused_msg_idx = session.runtime.paused_message_index;
        let accepted = session.process_tool_decisions(decisions);
        let any_rejected = decisions.iter().any(|d| !d.accepted);

        for id in &accepted {
            if !session.runtime.accepted_tool_ids.contains(id) {
                session.runtime.accepted_tool_ids.push(id.clone());
            }
        }

        for decision in decisions {
            if !decision.accepted {
                let tool_message = ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText("Tool execution denied by user".to_string()),
                    tool_call_id: decision.tool_call_id.clone(),
                    tool_failed: Some(true),
                    ..Default::default()
                };
                session.add_message(tool_message);
            }
        }

        let remaining = !session.runtime.pause_reasons.is_empty();

        let mut ids_to_execute: std::collections::HashSet<String> = session.runtime.accepted_tool_ids.iter().cloned().collect();
        if !any_rejected && !remaining {
            for id in &auto_approved {
                ids_to_execute.insert(id.clone());
            }
        }

        let tool_calls: Vec<crate::call_validation::ChatToolCall> = if let Some(msg_idx) = paused_msg_idx {
            session.messages.get(msg_idx)
                .and_then(|m| m.tool_calls.as_ref())
                .map(|tcs| tcs.iter().filter(|tc| ids_to_execute.contains(&tc.id)).cloned().collect())
                .unwrap_or_default()
        } else {
            session.messages
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .flatten()
                .filter(|tc| ids_to_execute.contains(&tc.id))
                .cloned()
                .collect()
        };

        (
            auto_approved,
            remaining,
            tool_calls,
            session.messages.clone(),
            session.thread.clone(),
            any_rejected,
        )
    };

    if has_remaining_pauses {
        return;
    }

    {
        let mut session = session_arc.lock().await;
        session.runtime.accepted_tool_ids.clear();
        session.runtime.auto_approved_tool_ids.clear();
        session.runtime.paused_message_index = None;
    }

    if any_rejected && !auto_approved_ids.is_empty() {
        let mut session = session_arc.lock().await;
        for id in &auto_approved_ids {
            let already_handled = session.messages.iter().any(|m| m.role == "tool" && m.tool_call_id == *id);
            if already_handled {
                continue;
            }
            let tool_message = ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Tool execution skipped due to user rejection of related tools".to_string()),
                tool_call_id: id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            };
            session.add_message(tool_message);
        }
    }

    if !tool_calls_to_execute.is_empty() {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::ExecutingTools, None);
        }

        let (tool_results, _) = execute_tools_with_session(
            gcx.clone(),
            session_arc.clone(),
            &tool_calls_to_execute,
            &messages,
            &thread,
            &thread.mode,
            Some(&thread.model),
            super::tools::ExecuteToolsOptions::default(),
        )
        .await;

        {
            let mut session = session_arc.lock().await;
            for result_msg in tool_results {
                session.add_message(result_msg);
            }
            session.set_runtime_state(SessionState::Idle, None);
        }

        maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
    }

    if any_rejected {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
        }
        maybe_save_trajectory(gcx, session_arc).await;
    } else if !tool_calls_to_execute.is_empty() {
        start_generation(gcx, session_arc).await;
    } else {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
        }
        maybe_save_trajectory(gcx, session_arc).await;
    }
}

/// Extract the latest checkpoint from session messages (call while holding lock)
fn find_latest_checkpoint(session: &ChatSession) -> Option<crate::git::checkpoints::Checkpoint> {
    session
        .messages
        .iter()
        .rev()
        .find(|msg| msg.role == "user" && !msg.checkpoints.is_empty())
        .and_then(|msg| msg.checkpoints.first().cloned())
}

/// Create checkpoint without holding session lock (async, potentially slow)
async fn create_checkpoint_async(
    gcx: Arc<ARwLock<GlobalContext>>,
    latest_checkpoint: Option<&crate::git::checkpoints::Checkpoint>,
    chat_id: &str,
) -> Vec<crate::git::checkpoints::Checkpoint> {
    use crate::git::checkpoints::create_workspace_checkpoint;

    match create_workspace_checkpoint(gcx, latest_checkpoint, chat_id).await {
        Ok((checkpoint, _)) => {
            tracing::info!(
                "Checkpoint created for chat {}: {:?}",
                chat_id,
                checkpoint
            );
            vec![checkpoint]
        }
        Err(e) => {
            warn!(
                "Failed to create checkpoint for chat {}: {}",
                chat_id, e
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_request(cmd: ChatCommand) -> CommandRequest {
        CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: cmd,
        }
    }

    #[test]
    fn test_find_allowed_command_empty_queue() {
        let queue = VecDeque::new();
        assert!(find_allowed_command_while_paused(&queue).is_none());
    }

    #[test]
    fn test_find_allowed_command_no_allowed() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::SetParams {
            patch: json!({"model": "gpt-4"}),
        }));
        assert!(find_allowed_command_while_paused(&queue).is_none());
    }

    #[test]
    fn test_find_allowed_command_finds_tool_decision() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::ToolDecision {
            tool_call_id: "tc1".into(),
            accepted: true,
        }));
        assert_eq!(find_allowed_command_while_paused(&queue), Some(1));
    }

    #[test]
    fn test_find_allowed_command_finds_tool_decisions() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::ToolDecisions {
            decisions: vec![ToolDecisionItem {
                tool_call_id: "tc1".into(),
                accepted: true,
            }],
        }));
        assert_eq!(find_allowed_command_while_paused(&queue), Some(0));
    }

    #[test]
    fn test_find_allowed_command_finds_abort() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("another"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::Abort {}));
        assert_eq!(find_allowed_command_while_paused(&queue), Some(2));
    }

    #[test]
    fn test_find_allowed_command_returns_first_match() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::Abort {}));
        queue.push_back(make_request(ChatCommand::ToolDecision {
            tool_call_id: "tc1".into(),
            accepted: true,
        }));
        assert_eq!(find_allowed_command_while_paused(&queue), Some(0));
    }

    #[test]
    fn test_apply_setparams_model() {
        let mut thread = ThreadParams::default();
        thread.model = "old-model".into();
        let patch = json!({"model": "new-model"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.model, "new-model");
    }

    #[test]
    fn test_apply_setparams_no_change_same_value() {
        let mut thread = ThreadParams::default();
        thread.model = "gpt-4".into();
        let patch = json!({"model": "gpt-4"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(!changed);
    }

    #[test]
    fn test_apply_setparams_mode() {
        let mut thread = ThreadParams::default();
        let patch = json!({"mode": "NO_TOOLS"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.mode, "NO_TOOLS");
    }

    #[test]
    fn test_apply_setparams_boost_reasoning() {
        let mut thread = ThreadParams::default();
        let patch = json!({"boost_reasoning": true});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.boost_reasoning, Some(true));
    }

    #[test]
    fn test_apply_setparams_tool_use() {
        let mut thread = ThreadParams::default();
        let patch = json!({"tool_use": "disabled"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.tool_use, "disabled");
    }

    #[test]
    fn test_apply_setparams_context_tokens_cap() {
        let mut thread = ThreadParams::default();
        let patch = json!({"context_tokens_cap": 4096});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.context_tokens_cap, Some(4096));
    }

    #[test]
    fn test_apply_setparams_context_tokens_cap_null() {
        let mut thread = ThreadParams::default();
        thread.context_tokens_cap = Some(4096);
        let patch = json!({"context_tokens_cap": null});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert!(thread.context_tokens_cap.is_none());
    }

    #[test]
    fn test_apply_setparams_context_tokens_cap_invalid_type_ignored() {
        let mut thread = ThreadParams::default();
        thread.context_tokens_cap = Some(4096);
        let patch = json!({"context_tokens_cap": "invalid"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(!changed);
        assert_eq!(thread.context_tokens_cap, Some(4096)); // Value preserved
    }

    #[test]
    fn test_apply_setparams_include_project_info() {
        let mut thread = ThreadParams::default();
        let patch = json!({"include_project_info": false});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert!(!thread.include_project_info);
    }

    #[test]
    fn test_apply_setparams_checkpoints_enabled() {
        let mut thread = ThreadParams::default();
        let patch = json!({"checkpoints_enabled": false});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert!(!thread.checkpoints_enabled);
    }

    #[test]
    fn test_apply_setparams_multiple_fields() {
        let mut thread = ThreadParams::default();
        let patch = json!({
            "model": "claude-3",
            "mode": "EXPLORE",
            "boost_reasoning": true,
        });
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.model, "claude-3");
        assert_eq!(thread.mode, "EXPLORE");
        assert_eq!(thread.boost_reasoning, Some(true));
    }

    #[test]
    fn test_apply_setparams_sanitizes_patch() {
        let mut thread = ThreadParams::default();
        let patch = json!({
            "model": "gpt-4",
            "type": "set_params",
            "chat_id": "chat-123",
            "seq": "42"
        });
        let (_, sanitized) = apply_setparams_patch(&mut thread, &patch);
        assert!(sanitized.get("type").is_none());
        assert!(sanitized.get("chat_id").is_none());
        assert!(sanitized.get("seq").is_none());
        assert!(sanitized.get("model").is_some());
    }

    #[test]
    fn test_apply_setparams_empty_patch() {
        let mut thread = ThreadParams::default();
        let original_model = thread.model.clone();
        let patch = json!({});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(!changed);
        assert_eq!(thread.model, original_model);
    }

    #[test]
    fn test_apply_setparams_invalid_types_ignored() {
        let mut thread = ThreadParams::default();
        thread.model = "original".into();
        let patch = json!({
            "model": 123,
            "boost_reasoning": "not_a_bool",
        });
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(!changed);
        assert_eq!(thread.model, "original");
    }

    #[test]
    fn test_find_allowed_command_while_waiting_ide_empty_queue() {
        let queue = VecDeque::new();
        assert!(find_allowed_command_while_waiting_ide(&queue).is_none());
    }

    #[test]
    fn test_find_allowed_command_while_waiting_ide_no_allowed() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::ToolDecision {
            tool_call_id: "tc1".into(),
            accepted: true,
        }));
        assert!(find_allowed_command_while_waiting_ide(&queue).is_none());
    }

    #[test]
    fn test_find_allowed_command_while_waiting_ide_finds_ide_tool_result() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::IdeToolResult {
            tool_call_id: "tc1".into(),
            content: "result".into(),
            tool_failed: false,
        }));
        assert_eq!(find_allowed_command_while_waiting_ide(&queue), Some(1));
    }

    #[test]
    fn test_find_allowed_command_while_waiting_ide_finds_abort() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
        }));
        queue.push_back(make_request(ChatCommand::Abort {}));
        assert_eq!(find_allowed_command_while_waiting_ide(&queue), Some(1));
    }

    #[test]
    fn test_find_allowed_command_while_waiting_ide_returns_first_match() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::Abort {}));
        queue.push_back(make_request(ChatCommand::IdeToolResult {
            tool_call_id: "tc1".into(),
            content: "result".into(),
            tool_failed: false,
        }));
        assert_eq!(find_allowed_command_while_waiting_ide(&queue), Some(0));
    }

    #[test]
    fn test_priority_insertion_before_non_priority() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("first"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-2".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("second"),
                attachments: vec![],
            },
        });
        let priority_req = CommandRequest {
            client_request_id: "req-priority".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority"),
                attachments: vec![],
            },
        };
        let insert_pos = queue
            .iter()
            .position(|r| !r.priority)
            .unwrap_or(queue.len());
        queue.insert(insert_pos, priority_req);
        assert_eq!(queue[0].client_request_id, "req-priority");
        assert_eq!(queue[1].client_request_id, "req-1");
        assert_eq!(queue[2].client_request_id, "req-2");
    }

    #[test]
    fn test_priority_insertion_after_existing_priority() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            client_request_id: "req-p1".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("p1"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("normal"),
                attachments: vec![],
            },
        });
        let priority_req = CommandRequest {
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("p2"),
                attachments: vec![],
            },
        };
        let insert_pos = queue
            .iter()
            .position(|r| !r.priority)
            .unwrap_or(queue.len());
        queue.insert(insert_pos, priority_req);
        assert_eq!(queue[0].client_request_id, "req-p1");
        assert_eq!(queue[1].client_request_id, "req-p2");
        assert_eq!(queue[2].client_request_id, "req-1");
    }

    #[test]
    fn test_priority_insertion_into_empty_queue() {
        let mut queue: VecDeque<CommandRequest> = VecDeque::new();
        let priority_req = CommandRequest {
            client_request_id: "req-p".into(),
            priority: true,
            command: ChatCommand::Abort {},
        };
        let insert_pos = queue
            .iter()
            .position(|r| !r.priority)
            .unwrap_or(queue.len());
        queue.insert(insert_pos, priority_req);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].client_request_id, "req-p");
    }

    #[test]
    fn test_priority_insertion_all_priority() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            client_request_id: "req-p1".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        let priority_req = CommandRequest {
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::Abort {},
        };
        let insert_pos = queue
            .iter()
            .position(|r| !r.priority)
            .unwrap_or(queue.len());
        queue.insert(insert_pos, priority_req);
        assert_eq!(queue[0].client_request_id, "req-p1");
        assert_eq!(queue[1].client_request_id, "req-p2");
    }

    #[test]
    fn test_drain_priority_user_messages_extracts_only_priority() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            client_request_id: "req-p1".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority 1"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("normal"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority 2"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-abort".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });

        let drained = drain_priority_user_messages(&mut queue);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].client_request_id, "req-p1");
        assert_eq!(drained[1].client_request_id, "req-p2");
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].client_request_id, "req-1");
        assert_eq!(queue[1].client_request_id, "req-abort");
    }

    #[test]
    fn test_drain_non_priority_user_messages_extracts_all_non_priority() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("first"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-p".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-2".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("second"),
                attachments: vec![],
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-3".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("third"),
                attachments: vec![],
            },
        });

        let drained = drain_non_priority_user_messages(&mut queue);
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].client_request_id, "req-1");
        assert_eq!(drained[1].client_request_id, "req-2");
        assert_eq!(drained[2].client_request_id, "req-3");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].client_request_id, "req-p");
    }

    #[test]
    fn test_drain_priority_skips_non_user_messages() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            client_request_id: "req-abort".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-params".into(),
            priority: true,
            command: ChatCommand::SetParams { patch: json!({}) },
        });

        let drained = drain_priority_user_messages(&mut queue);
        assert!(drained.is_empty());
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_drain_empty_queue() {
        let mut queue: VecDeque<CommandRequest> = VecDeque::new();
        let priority_drained = drain_priority_user_messages(&mut queue);
        let non_priority_drained = drain_non_priority_user_messages(&mut queue);
        assert!(priority_drained.is_empty());
        assert!(non_priority_drained.is_empty());
    }
}
