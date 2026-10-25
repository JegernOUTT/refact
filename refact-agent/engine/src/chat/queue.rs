use std::collections::{BTreeSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::Utc;
use tokio::sync::{Mutex as AMutex};
use tracing::warn;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::buddy::chat_reactions::{maybe_enqueue_chat_reaction, AcceptedUserMessage};
use refact_buddy_core::user_action::UserAction;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{ControlVerdict, HookPayload, get_project_dir_string, run_control_hooks};
#[cfg(test)]
use crate::ext::skills_context::SKILLS_CONTEXT_MARKER;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::files_correction::get_project_dirs;
use crate::chat::internal_roles::{self, mode_switch_event_with_diff, EventSubkind};
use crate::chat::plan_role;
use crate::yaml_configs::customization_registry::get_mode_config;
use crate::yaml_configs::customization_types::ModeConfig;

use super::types::*;
use super::content::parse_content_with_attachments;
use super::generation::{start_generation, maybe_save_trajectory_with_intent};
use super::goal_verifier::{
    should_verify_goal_on_done, verify_goal_before_completion, GoalCompletionGateOutcome,
};
use super::tools::{
    acquire_session_turn_tool_pool, execute_tools_with_session,
    resolve_tool_call_aliases_with_catalog,
};
use super::trajectories::{maybe_save_trajectory_background_with_intent};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{WorktreeMeta, WorktreeReference};

#[path = "command_handlers.rs"]
mod command_handlers;

pub fn worktree_activation_message(worktree: &WorktreeMeta) -> ChatMessage {
    let branch = worktree.branch.as_deref().unwrap_or("unknown");
    let base = worktree.base_branch.as_deref().unwrap_or("unknown");
    ChatMessage {
        role: "cd_instruction".to_string(),
        content: ChatContent::SimpleText(format!(
            "💿 WORKTREE_ENABLED\n\nActive worktree scope is now ON for this chat.\n\n- Worktree id: `{}`\n- Branch: `{}`\n- Base/target branch: `{}`\n- Worktree root: `{}`\n- Source workspace root: `{}`\n\nEffects for this thread:\n- File reads, edits, shell commands, searches, and @file resolution should operate inside the worktree root unless a tool explicitly says otherwise.\n- Treat the main workspace as the merge target and do not edit it directly for this chat.\n- Use relative paths as usual; absolute paths outside the worktree may be rejected or remapped.\n- To merge completed work, call `worktree_merge` or use the Worktrees UI merge action.\n- If you need to leave the isolated scope, ask the user to detach the worktree first.",
            worktree.id,
            branch,
            base,
            worktree.root.display(),
            worktree.source_workspace_root.display()
        )),
        tool_call_id: "worktree_enabled".to_string(),
        ..Default::default()
    }
}

pub fn worktree_disabled_message(worktree: Option<&WorktreeMeta>) -> ChatMessage {
    let previous = worktree
        .map(|worktree| {
            let branch = worktree.branch.as_deref().unwrap_or("unknown");
            format!(
                "\n\nPrevious worktree scope:\n- Worktree id: `{}`\n- Branch: `{}`\n- Worktree root: `{}`",
                worktree.id,
                branch,
                worktree.root.display()
            )
        })
        .unwrap_or_default();
    ChatMessage {
        role: "cd_instruction".to_string(),
        content: ChatContent::SimpleText(format!(
            "💿 WORKTREE_DISABLED\n\nActive worktree scope is now OFF for this chat. File reads, edits, shell commands, searches, and @file resolution should use the main workspace again.{previous}"
        )),
        tool_call_id: "worktree_disabled".to_string(),
        ..Default::default()
    }
}

async fn push_user_activity(app: AppState, action: UserAction) {
    app.activity_sink.record_user_action(action).await;
}

fn apply_manual_context_files(
    session: &mut super::types::ChatSession,
    context_files: &[serde_json::Value],
) {
    const MAX_CTX_FILES: usize = 5;
    const MAX_TOTAL_CHARS: usize = 50_000;
    let mut validated: Vec<crate::call_validation::ContextFile> = Vec::new();
    let mut total_chars = 0usize;
    for v in context_files.iter().take(MAX_CTX_FILES) {
        if let Ok(file) = serde_json::from_value::<crate::call_validation::ContextFile>(v.clone()) {
            let chars = file.file_content.chars().count();
            if total_chars + chars <= MAX_TOTAL_CHARS {
                total_chars += chars;
                validated.push(file);
            } else {
                continue;
            }
        }
    }
    if !validated.is_empty() {
        let msg = ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(validated),
            tool_call_id: "manual_memory_enrichment".to_string(),
            ..Default::default()
        };
        session.add_message(msg);
    }
}

fn user_message_with_client_message_id(
    content: ChatContent,
    checkpoints: Vec<crate::git::checkpoints::Checkpoint>,
    client_message_id: Option<String>,
) -> ChatMessage {
    let mut message = ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content,
        checkpoints,
        ..Default::default()
    };
    if let Some(client_message_id) = client_message_id {
        message.extra.insert(
            CLIENT_MESSAGE_ID_EXTRA_KEY.to_string(),
            serde_json::Value::String(client_message_id),
        );
    }
    message
}

async fn aborted_before_start_generation(
    session_arc: &Arc<AMutex<super::types::ChatSession>>,
) -> bool {
    let mut session = session_arc.lock().await;
    if !session.user_interrupt_flag.load(Ordering::SeqCst) {
        return false;
    }
    session.abort_flag.store(false, Ordering::SeqCst);
    session.user_interrupt_flag.store(false, Ordering::SeqCst);
    if session.runtime.state == SessionState::Generating {
        session.set_runtime_state(SessionState::Idle, None);
    }
    session.queue_notify.notify_one();
    true
}

pub(crate) fn reject_guard(session: &mut ChatSession, request_id: &str, error: String) {
    session.emit(ChatEvent::Ack {
        client_request_id: request_id.to_string(),
        accepted: false,
        result: Some(serde_json::json!({"error": error})),
    });
    let snapshot = session.snapshot();
    session.emit(snapshot);
}

pub(crate) fn prepare_guarded_command(
    session: &mut ChatSession,
    request: &CommandRequest,
) -> Result<(), String> {
    if let Some(guard) = &request.supervision_guard {
        if request.native_approval_guard.is_some() || request.priority {
            return Err("supervision cannot carry native approval or priority authority".into());
        }
        crate::buddy::supervision_authority::validate_chat_command(
            guard,
            &session.chat_id,
            &request.command,
        )?;
    }
    if request.native_approval_guard.is_some()
        && (request.priority
            || !matches!(
                request.command,
                ChatCommand::ToolDecision { .. }
                    | ChatCommand::ToolDecisions { .. }
                    | ChatCommand::BrowserContextDecision { .. }
            ))
    {
        return Err("native approval guard is not valid for this command".into());
    }
    if request.supervision_guard.is_none()
        && matches!(
            request.command,
            ChatCommand::UserMessage { .. }
                | ChatCommand::RetryFromIndex { .. }
                | ChatCommand::Abort { .. }
                | ChatCommand::SetParams { .. }
                | ChatCommand::GoalControl { .. }
                | ChatCommand::SetGoal { .. }
                | ChatCommand::AnswerQuestions { .. }
                | ChatCommand::UpdateMessage { .. }
                | ChatCommand::RemoveMessage { .. }
        )
    {
        crate::buddy::supervision_authority::invalidate_chat(&session.chat_id)?;
    }
    Ok(())
}

fn consume_native_guard(
    session: &mut ChatSession,
    guard: Option<&refact_chat_api::NativeApprovalGuard>,
    root: Option<&str>,
    request_id: &str,
) -> bool {
    let Some(guard) = guard else {
        return true;
    };
    let result = if root != Some(guard.project_root.as_str()) {
        Err("Native project changed; refresh before responding".into())
    } else {
        crate::http::routers::v1::buddy_native_approvals::validate_native_approval(
            session,
            root.unwrap_or_default(),
            &guard.engine_identity,
            &guard.source_version,
        )
    };
    if let Err(error) = result {
        reject_guard(session, request_id, error);
        return false;
    }
    true
}

fn consume_tool_guard(
    session: &mut ChatSession,
    guard: Option<&refact_chat_api::NativeApprovalGuard>,
    root: Option<&str>,
    request_id: &str,
    decisions: &[ToolDecisionItem],
) -> bool {
    if !consume_native_guard(session, guard, root, request_id) {
        return false;
    }
    if guard.is_some() {
        let mut ids = BTreeSet::new();
        if session.runtime.state != SessionState::Paused
            || decisions.is_empty()
            || decisions.iter().any(|decision| {
                !ids.insert(&decision.tool_call_id)
                    || session
                        .runtime
                        .accepted_tool_ids
                        .contains(&decision.tool_call_id)
                    || !session.runtime.pause_reasons.iter().any(|reason| {
                        reason.reason_type == "confirmation"
                            && reason.tool_call_id == decision.tool_call_id
                    })
            })
        {
            reject_guard(
                session,
                request_id,
                "Native tool approval is not pending".into(),
            );
            return false;
        }
    }
    true
}

async fn native_guard_root(
    app: &AppState,
    guard: Option<&refact_chat_api::NativeApprovalGuard>,
) -> Option<String> {
    if guard.is_none() {
        return None;
    }
    crate::http::routers::v1::buddy_decisions::decision_root(app)
        .await
        .ok()
}

fn command_triggers_generation(cmd: &ChatCommand) -> bool {
    matches!(
        cmd,
        ChatCommand::UserMessage { .. }
            | ChatCommand::RetryFromIndex { .. }
            | ChatCommand::Regenerate {}
    )
}

fn answer_questions_content(tool_call_id: &str, answers: &[QuestionAnswer]) -> String {
    serde_json::json!({"type":"ask_questions_answer","tool_call_id":tool_call_id,"answers":answers})
        .to_string()
}

fn next_state_after_released_answer(session: &ChatSession) -> SessionState {
    if !session.runtime.pause_reasons.is_empty() {
        SessionState::Paused
    } else if session
        .pending_questions
        .iter()
        .any(|set| set.status == PendingQuestionStatus::Pending)
    {
        SessionState::WaitingUserInput
    } else {
        SessionState::Idle
    }
}

fn reject_question_answers(session: &mut ChatSession, client_request_id: &str, error: String) {
    if session.runtime.state == SessionState::Generating {
        session.set_runtime_state(next_state_after_released_answer(session), None);
    }
    session.emit(ChatEvent::Ack {
        client_request_id: client_request_id.to_string(),
        accepted: false,
        result: Some(serde_json::json!({ "error": error })),
    });
    let snapshot = session.snapshot();
    session.emit(snapshot);
    session.queue_notify.notify_one();
}

fn release_answer_questions_failure(
    session: &mut ChatSession,
    client_request_id: &str,
    error: String,
    emit_snapshot: bool,
) {
    session.emit(ChatEvent::Ack {
        client_request_id: client_request_id.to_string(),
        accepted: false,
        result: Some(serde_json::json!({ "error": error.clone() })),
    });
    if emit_snapshot {
        let snapshot = session.snapshot();
        session.emit(snapshot);
    }
    if session.runtime.state == SessionState::Generating {
        session.set_runtime_state(SessionState::Error, Some(error));
    } else {
        let compression_phase = session.compression_phase;
        let compression_reason = session.compression_reason;
        let event = session.runtime_update_event(
            SessionState::Error,
            Some(error),
            false,
            compression_phase,
            compression_reason,
        );
        session.emit(event);
    }
    session.queue_notify.notify_one();
}

fn note_user_turn_resets_goal_pursuit(session: &mut ChatSession) {
    session.goal_reset_no_progress();
    session.reactivate_goal_stopped_by_manual_abort();
}

fn drain_redundant_regenerates(
    queue: &mut VecDeque<CommandRequest>,
    priority: bool,
) -> Vec<String> {
    let mut removed = Vec::new();
    while queue.front().is_some_and(|request| {
        request.priority == priority && matches!(request.command, ChatCommand::Regenerate {})
    }) {
        if let Some(request) = queue.pop_front() {
            removed.push(request.client_request_id);
        }
    }
    removed
}

pub async fn inject_priority_messages_if_any(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    let priority_requests = {
        let mut session = session_arc.lock().await;
        let requests = drain_priority_user_messages(&mut session.command_queue);
        if !requests.is_empty() {
            for request in &requests {
                session.record_command_queue_wait(&request.client_request_id);
            }
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
            context_files,
            suppress_auto_enrichment,
            client_message_id,
        } = request.command
        {
            let (session_id, project_dir) = {
                let session = session_arc.lock().await;
                let sid = session.chat_id.clone();
                drop(session);
                let pd = get_project_dir_string(app.clone()).await;
                (sid, pd)
            };
            let prompt_text = match &content {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            let hook_verdict = run_control_hooks(
                app.clone(),
                HookEvent::UserPromptSubmit,
                HookPayload::new(&HookEvent::UserPromptSubmit, session_id, project_dir)
                    .with_user_prompt(prompt_text),
            )
            .await;
            if matches!(hook_verdict, ControlVerdict::Deny(_)) {
                continue;
            }

            let (checkpoints_enabled, chat_id, latest_checkpoint, worktree) = {
                let session = session_arc.lock().await;
                (
                    session.thread.checkpoints_enabled,
                    session.chat_id.clone(),
                    find_latest_checkpoint(&session),
                    session.thread.worktree.clone(),
                )
            };

            let checkpoints = if checkpoints_enabled {
                create_checkpoint_async(
                    app.clone(),
                    latest_checkpoint.as_ref(),
                    &chat_id,
                    worktree.as_ref(),
                )
                .await
            } else {
                Vec::new()
            };

            let accepted_user_message = {
                let mut session = session_arc.lock().await;
                if !context_files.is_empty() {
                    apply_manual_context_files(&mut session, &context_files);
                }
                if suppress_auto_enrichment && context_files.is_empty() {
                    session.suppress_auto_enrichment_for_next_turn = true;
                }
                let parsed_content = parse_content_with_attachments(&content, &attachments);
                let accepted = AcceptedUserMessage {
                    chat_id: session.chat_id.clone(),
                    thread: session.thread.clone(),
                    content: parsed_content.clone(),
                };
                let user_message = user_message_with_client_message_id(
                    parsed_content,
                    checkpoints,
                    client_message_id,
                );
                session.add_message(user_message);
                note_user_turn_resets_goal_pursuit(&mut session);
                accepted
            };
            let _ = maybe_enqueue_chat_reaction(app.clone(), accepted_user_message).await;
        }
    }

    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );
    true
}

pub fn find_allowed_command_while_paused(queue: &VecDeque<CommandRequest>) -> Option<usize> {
    for (i, req) in queue.iter().enumerate() {
        match &req.command {
            ChatCommand::ToolDecision { .. }
            | ChatCommand::ToolDecisions { .. }
            | ChatCommand::AnswerQuestions { .. }
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
            // Clear provider-specific state that's invalid across models.
            // OpenAI Responses API previous_response_id is tied to a specific
            // model+endpoint; switching models makes it invalid.
            if thread.previous_response_id.is_some() {
                tracing::info!("Clearing previous_response_id on model switch");
                thread.previous_response_id = None;
            }
            changed = true;
        }
    }
    if let Some(mode) = patch.get("mode").and_then(|v| v.as_str()) {
        let normalized_mode =
            crate::yaml_configs::customization_registry::map_legacy_mode_to_id(mode);
        if thread.mode != normalized_mode {
            thread.mode = normalized_mode.to_string();
            changed = true;
        }
    }
    if let Some(boost_val) = patch.get("boost_reasoning") {
        let new_boost = if boost_val.is_null() {
            None
        } else if let Some(boost) = boost_val.as_bool() {
            Some(boost)
        } else {
            thread.boost_reasoning
        };
        if thread.boost_reasoning != new_boost {
            thread.boost_reasoning = new_boost;
            changed = true;
        }
    }
    if let Some(effort_val) = patch.get("reasoning_effort") {
        let new_val = if effort_val.is_null() {
            None
        } else if let Some(effort) = effort_val.as_str() {
            if effort.is_empty() {
                None
            } else {
                Some(effort.to_string())
            }
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
    if let Some(cap) = patch.get("auto_compression_cap") {
        // A user-supplied value (including null/zero) settles initialization.
        if (cap.is_null() || cap.as_u64().is_some()) && thread.auto_compression_cap_pending {
            thread.auto_compression_cap_pending = false;
            changed = true;
        }
        // Null is an explicit reset; defer recomputation until the model is known.
        // Invalid patches must not change initialization provenance.
        if cap.is_null() || cap.as_u64().is_some() {
            let pending = cap.is_null();
            if thread.auto_compression_cap_pending != pending {
                thread.auto_compression_cap_pending = pending;
                changed = true;
            }
        }
        if cap.is_null() {
            if thread.auto_compression_cap.is_some() {
                thread.auto_compression_cap = None;
                changed = true;
            }
        } else if let Some(n) = cap.as_u64() {
            let new_cap = Some(n as usize);
            if thread.auto_compression_cap != new_cap {
                thread.auto_compression_cap = new_cap;
                changed = true;
            }
        }
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
    if let Some(val) = patch
        .get("auto_approve_editing_tools")
        .and_then(|v| v.as_bool())
    {
        if thread.auto_approve_editing_tools != val {
            thread.auto_approve_editing_tools = val;
            changed = true;
        }
    }
    if let Some(val) = patch
        .get("auto_approve_dangerous_commands")
        .and_then(|v| v.as_bool())
    {
        if thread.auto_approve_dangerous_commands != val {
            thread.auto_approve_dangerous_commands = val;
            changed = true;
        }
    }
    if let Some(val) = patch.get("autonomous_no_confirm").and_then(|v| v.as_bool()) {
        if thread.autonomous_no_confirm != val {
            thread.autonomous_no_confirm = val;
            changed = true;
        }
    }
    if let Some(val) = patch.get("auto_enrichment_enabled") {
        if val.is_null() {
            if thread.auto_enrichment_enabled.is_some() {
                thread.auto_enrichment_enabled = None;
                changed = true;
            }
        } else if let Some(b) = val.as_bool() {
            let new_val = Some(b);
            if thread.auto_enrichment_enabled != new_val {
                thread.auto_enrichment_enabled = new_val;
                changed = true;
            }
        }
    }
    if let Some(val) = patch.get("auto_compact_enabled") {
        if val.is_null() {
            if thread.auto_compact_enabled.is_some() {
                thread.auto_compact_enabled = None;
                changed = true;
            }
        } else if let Some(b) = val.as_bool() {
            let new_val = Some(b);
            if thread.auto_compact_enabled != new_val {
                thread.auto_compact_enabled = new_val;
                changed = true;
            }
        }
    }
    if let Some(task_meta_value) = patch.get("task_meta") {
        if !task_meta_value.is_null() {
            if let Ok(task_meta) =
                serde_json::from_value::<super::types::TaskMeta>(task_meta_value.clone())
            {
                let new_task_meta = Some(task_meta);
                if thread.task_meta != new_task_meta {
                    thread.task_meta = new_task_meta;
                    changed = true;
                }
            }
        }
    }
    if let Some(v) = patch.get("buddy_meta") {
        if let Ok(meta) =
            serde_json::from_value::<Option<refact_buddy_core::types::BuddyThreadMeta>>(v.clone())
        {
            thread.buddy_meta = meta;
            changed = true;
        }
    }
    if let Some(parent_id) = patch.get("parent_id").and_then(|v| v.as_str()) {
        let new_val = if parent_id.is_empty() {
            None
        } else {
            Some(parent_id.to_string())
        };
        if thread.parent_id != new_val {
            thread.parent_id = new_val;
            changed = true;
        }
    }
    if let Some(link_type) = patch.get("link_type").and_then(|v| v.as_str()) {
        let new_val = if link_type.is_empty() {
            None
        } else {
            Some(link_type.to_string())
        };
        if thread.link_type != new_val {
            thread.link_type = new_val;
            changed = true;
        }
    }
    if let Some(root_chat_id) = patch.get("root_chat_id").and_then(|v| v.as_str()) {
        let new_val = if root_chat_id.is_empty() {
            None
        } else {
            Some(root_chat_id.to_string())
        };
        if thread.root_chat_id != new_val {
            thread.root_chat_id = new_val;
            changed = true;
        }
    }

    if let Some(worktree_val) = patch.get("worktree") {
        if worktree_val.is_null() && thread.worktree.is_some() {
            thread.worktree = None;
            changed = true;
        }
    }

    let mut sanitized_patch = patch.clone();
    if let Some(obj) = sanitized_patch.as_object_mut() {
        obj.remove("type");
        obj.remove("chat_id");
        obj.remove("seq");
        obj.remove("worktree_id");
        if patch.get("mode").and_then(|v| v.as_str()).is_some() {
            obj.insert("mode".to_string(), serde_json::json!(thread.mode));
        }
        if let Some(worktree_val) = patch.get("worktree") {
            if worktree_val.is_null() {
                obj.insert("worktree".to_string(), serde_json::Value::Null);
            } else {
                obj.remove("worktree");
            }
        }
    }

    (changed, sanitized_patch)
}

const MODE_SWITCH_TOOL_NAMES_LIMIT: usize = 20;

fn mode_switch_bool_delta(from: bool, to: bool) -> serde_json::Value {
    serde_json::json!({
        "from": from,
        "to": to,
        "changed": from != to,
    })
}

fn mode_switch_tool_delta(from: &BTreeSet<String>, to: &BTreeSet<String>) -> serde_json::Value {
    let names: Vec<_> = to
        .difference(from)
        .take(MODE_SWITCH_TOOL_NAMES_LIMIT)
        .cloned()
        .collect();
    let count = to.difference(from).count();
    serde_json::json!({
        "count": count,
        "names": names,
        "truncated": count > MODE_SWITCH_TOOL_NAMES_LIMIT,
    })
}

fn tool_confirm_changed(from: &ModeConfig, to: &ModeConfig) -> bool {
    from.tool_confirm.rules.len() != to.tool_confirm.rules.len()
        || from
            .tool_confirm
            .rules
            .iter()
            .zip(&to.tool_confirm.rules)
            .any(|(from, to)| from.match_pattern != to.match_pattern || from.action != to.action)
}

fn mode_switch_diff(from: &ModeConfig, to: &ModeConfig) -> serde_json::Value {
    let from_tools: BTreeSet<_> = from.tools.iter().cloned().collect();
    let to_tools: BTreeSet<_> = to.tools.iter().cloned().collect();
    let tool_confirm_changed = tool_confirm_changed(from, to);
    serde_json::json!({
        "resolved": true,
        "tools": {
            "added": mode_switch_tool_delta(&from_tools, &to_tools),
            "removed": mode_switch_tool_delta(&to_tools, &from_tools),
        },
        "system_prompt": {
            "changed": from.prompt != to.prompt,
        },
        "permissions": {
            "allow_integrations": mode_switch_bool_delta(
                from.allow_integrations,
                to.allow_integrations,
            ),
            "allow_mcp": mode_switch_bool_delta(from.allow_mcp, to.allow_mcp),
            "allow_subagents": mode_switch_bool_delta(
                from.allow_subagents,
                to.allow_subagents,
            ),
        },
        "tool_confirm": {
            "changed": tool_confirm_changed,
            "from_rules_count": from.tool_confirm.rules.len(),
            "to_rules_count": to.tool_confirm.rules.len(),
        },
        "thread_defaults": {
            "auto_approve_editing_tools": mode_switch_bool_delta(
                from.thread_defaults.auto_approve_editing_tools.unwrap_or(false),
                to.thread_defaults.auto_approve_editing_tools.unwrap_or(false),
            ),
            "auto_approve_dangerous_commands": mode_switch_bool_delta(
                from.thread_defaults.auto_approve_dangerous_commands.unwrap_or(false),
                to.thread_defaults.auto_approve_dangerous_commands.unwrap_or(false),
            ),
        },
    })
}

pub(crate) async fn add_mode_switch_event_and_plan_if_changed(
    app: AppState,
    session: &mut ChatSession,
    old_mode: &str,
    reason: Option<&str>,
    source: &str,
) -> bool {
    let new_mode = session.thread.mode.clone();
    if new_mode == old_mode {
        return false;
    }

    let model_id = if session.thread.model.is_empty() {
        None
    } else {
        Some(session.thread.model.as_str())
    };
    let from_config = get_mode_config(app.gcx.clone(), old_mode, model_id).await;
    let to_config = get_mode_config(app.gcx.clone(), &new_mode, model_id).await;
    let diff = match (&from_config, &to_config) {
        (Some(from), Some(to)) => mode_switch_diff(from, to),
        _ => serde_json::json!({"resolved": false}),
    };
    session.add_message(mode_switch_event_with_diff(
        source, old_mode, &new_mode, reason, diff,
    ));

    if plan_role::try_current_base_plan(session).is_ok_and(|plan| plan.is_none()) {
        if let Some(mode_config) = to_config {
            let plan_template = mode_config.plan_template.trim();
            if !plan_template.is_empty() {
                let rendered = super::prompts::render_mode_plan_template(
                    app,
                    plan_template,
                    &mode_config.title,
                    session.thread.include_project_info,
                    &session.thread.task_meta,
                )
                .await;
                if !rendered.trim().is_empty() {
                    session.install_plan(&new_mode, rendered.trim());
                }
            }
        }
    }

    true
}

fn epoch_ms_now() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

fn goal_exists_including_pending(session: &ChatSession) -> bool {
    session.goal.is_some()
        || crate::chat::goal_role::current_base_goal(session).is_some()
        || session
            .post_tool_side_effects
            .iter()
            .any(|message| message.role == internal_roles::GOAL_ROLE)
}

fn goal_base_exists_including_pending(session: &ChatSession) -> bool {
    crate::chat::goal_role::current_base_goal(session).is_some()
        || session
            .post_tool_side_effects
            .iter()
            .any(|message| message.role == internal_roles::GOAL_ROLE)
}

fn goal_delta_count_including_pending(session: &ChatSession) -> usize {
    crate::chat::goal_role::effective_goal_delta_count(&session.accepted_control_projection())
}

fn update_goal_result(
    seq: usize,
    truncation: Option<internal_roles::PlanDeltaTruncation>,
) -> serde_json::Value {
    let Some(truncation) = truncation else {
        return serde_json::json!({"seq": seq, "truncated": false});
    };
    serde_json::json!({
        "seq": seq,
        "truncated": true,
        "original_chars": truncation.original_chars,
        "kept_chars": truncation.kept_chars,
    })
}

fn handle_set_goal_command(
    session: &mut ChatSession,
    content: String,
    budget: Option<GoalBudget>,
    criteria: Option<Vec<GoalCriterion>>,
) -> Result<serde_json::Value, String> {
    if content.trim().is_empty() {
        return Err("argument `content` must be non-empty".to_string());
    }
    if goal_exists_including_pending(session) {
        return Err("goal already exists; use update_goal".to_string());
    }
    let current_mode =
        crate::yaml_configs::customization_registry::map_legacy_mode_to_id(&session.thread.mode)
            .to_string();
    let budget = match budget {
        Some(mut budget) => {
            budget.explicit = true;
            budget
        }
        None => GoalBudget::default(),
    };
    let report = session.install_goal_with_criteria(
        &current_mode,
        &content,
        true,
        budget,
        criteria.unwrap_or_default(),
    );
    session.add_message(internal_roles::event(
        EventSubkind::SystemNotice,
        "chat.command.set_goal",
        serde_json::json!({"version": report.version}),
        format!("Goal updated to v{}", report.version),
    ));
    Ok(serde_json::json!({
        "version": report.version,
        "supersedes": report.supersedes,
    }))
}

fn handle_set_goal_budget_command(
    session: &mut ChatSession,
    mut budget: GoalBudget,
) -> Result<serde_json::Value, String> {
    if session.goal.is_none() {
        return Err("no goal to set budget for; call set_goal first".to_string());
    }

    let view = refact_core::active_context::active_context(&session.messages)
        .map_err(|e| e.to_string())?;
    let mut messages = view.messages.clone();
    let Some((base_index, _, _)) = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            crate::chat::goal_role::goal_version(message).map(|version| (index, version, message))
        })
        .max_by_key(|(index, version, _)| (*version, *index))
    else {
        return Err("no goal to set budget for; call set_goal first".to_string());
    };

    budget.explicit = true;
    let serialized_budget = serde_json::to_value(&budget)
        .map_err(|error| format!("failed to serialize goal budget: {error}"))?;
    let goal_meta = messages[base_index]
        .extra
        .get_mut("goal")
        .and_then(|value| value.as_object_mut())
        .ok_or_else(|| "no goal to set budget for; call set_goal first".to_string())?;
    goal_meta.insert("budget".to_string(), serialized_budget.clone());

    session.messages =
        refact_core::active_context::writeback_active_context(&session.messages, &view, &messages)
            .map_err(|e| e.to_string())?;
    session.rebuild_goal_projection_from_messages();
    session.goal_ledger_append(GoalLedgerOp::BudgetSet {
        budget: budget.clone(),
    });
    let budget_status_change = session.goal.as_ref().and_then(|goal| {
        match goal.goal_budget_exhaustion_status_at(epoch_ms_now()) {
            Some(status) if goal.active && goal.status != status => Some(status),
            None if matches!(
                goal.status,
                GoalStatus::BudgetExhausted | GoalStatus::NoProgress
            ) =>
            {
                Some(if goal.active {
                    GoalStatus::Active
                } else {
                    GoalStatus::Paused
                })
            }
            _ => None,
        }
    });
    if let Some(status) = budget_status_change {
        session.goal_set_status_reason(status, "budget_set");
    }
    session.mark_persisted_runtime_changed();
    session.emit_goal_status();

    Ok(serde_json::json!({ "budget": serialized_budget }))
}

fn handle_update_goal_command(
    session: &mut ChatSession,
    note: String,
) -> Result<serde_json::Value, String> {
    if note.trim().is_empty() {
        return Err("argument `note` must be non-empty".to_string());
    }
    if !goal_base_exists_including_pending(session) {
        return Err("no goal to update; call set_goal first".to_string());
    }
    let seq = goal_delta_count_including_pending(session) + 1;
    let (delta, truncation) = internal_roles::goal_delta_with_truncation(
        "chat.command.update_goal",
        serde_json::json!({"seq": seq, "at_ms": epoch_ms_now()}),
        note,
    );
    session.add_message(delta);
    session.emit_goal_status();
    Ok(update_goal_result(seq, truncation))
}

fn handle_goal_control_command(
    session: &mut ChatSession,
    action: String,
) -> Result<serde_json::Value, String> {
    if session.goal.is_none() {
        return Err("no goal to control; call set_goal first".to_string());
    }
    let normalized = action.trim().to_ascii_lowercase();
    let status = match normalized.as_str() {
        "pause" => GoalStatus::Paused,
        "resume" => GoalStatus::Active,
        "stop" => GoalStatus::Stopped,
        _ => return Err("goal_control action must be pause, resume, or stop".to_string()),
    };
    if normalized == "resume"
        && !matches!(
            session.goal_status,
            Some(GoalStatus::Paused | GoalStatus::Stopped)
        )
    {
        return Err("goal_control resume requires a paused or stopped goal".to_string());
    }
    session.clear_goal_stopped_by_abort_marker();
    if matches!(status, GoalStatus::Paused | GoalStatus::Stopped) {
        session.purge_goal_generated_wakes();
    }
    session.goal_set_status_reason(status, "goal_control");
    if status == GoalStatus::Stopped {
        session.add_message(internal_roles::event(
            EventSubkind::GoalPursuit,
            "chat.command.goal_control",
            serde_json::json!({"kind": "stopped", "trigger": "goal_control", "at_ms": epoch_ms_now()}),
            "Goal pursuit stopped by user.".to_string(),
        ));
    }
    Ok(serde_json::json!({
        "action": normalized,
        "status": status,
    }))
}

fn emit_goal_command_error(session: &mut ChatSession, error: String) {
    let event = session.runtime_update_event(
        SessionState::Error,
        Some(error),
        session.is_compressing,
        session.compression_phase,
        session.compression_reason,
    );
    session.emit(event);
}

#[derive(Clone)]
pub struct WorktreeSetParamsUpdate {
    pub worktree: Option<WorktreeMeta>,
    pub previous_worktree: Option<WorktreeMeta>,
    pub changed: bool,
    pub sse_value: serde_json::Value,
}

fn reference_for_thread(
    chat_id: &str,
    thread: &ThreadParams,
    worktree_kind: &str,
) -> WorktreeReference {
    let task_meta = thread.task_meta.as_ref();
    WorktreeReference {
        kind: worktree_kind.to_string(),
        chat_id: Some(chat_id.to_string()),
        task_id: task_meta.map(|meta| meta.task_id.clone()),
        card_id: task_meta.and_then(|meta| meta.card_id.clone()),
        agent_id: task_meta.and_then(|meta| meta.agent_id.clone()),
    }
}

async fn worktree_service_from_gcx(
    app: AppState,
    requested_source_root: Option<&std::path::Path>,
) -> Result<WorktreeService, String> {
    let cache_dir = app.paths.cache_dir.clone();
    let project_dirs = get_project_dirs(app.gcx.clone()).await;
    if project_dirs.is_empty() {
        return Err("No project root available".to_string());
    }
    let source_root = match requested_source_root {
        Some(requested) => {
            let requested = tokio::fs::canonicalize(requested).await.map_err(|e| {
                format!(
                    "Failed to resolve worktree source root '{}': {}",
                    requested.display(),
                    e
                )
            })?;
            let requested = dunce::simplified(&requested).to_path_buf();
            let mut matches = false;
            for dir in &project_dirs {
                if tokio::fs::canonicalize(dir)
                    .await
                    .map(|canonical| dunce::simplified(&canonical).to_path_buf() == requested)
                    .unwrap_or(false)
                {
                    matches = true;
                    break;
                }
            }
            if !matches {
                return Err("Worktree source root is not a current workspace directory".to_string());
            }
            requested
        }
        None => project_dirs[0].clone(),
    };
    WorktreeService::new_async(cache_dir, source_root).await
}

async fn remove_thread_reference(
    app: AppState,
    chat_id: &str,
    thread: &ThreadParams,
    worktree: &WorktreeMeta,
) {
    let reference = reference_for_thread(chat_id, thread, &worktree.kind);
    let Ok(service) = worktree_service_from_gcx(app, Some(&worktree.source_workspace_root)).await
    else {
        warn!(
            "Failed to resolve worktree service while detaching '{}'",
            worktree.id
        );
        return;
    };
    if let Err(e) = service.remove_reference(&worktree.id, &reference).await {
        warn!(
            "Failed to remove worktree reference '{}': {}",
            worktree.id, e
        );
    }
}

async fn add_thread_worktree_reference(
    app: AppState,
    chat_id: &str,
    thread: &ThreadParams,
    worktree: &WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service = match worktree_service_from_gcx(app, Some(&worktree.source_workspace_root)).await
    {
        Ok(service) => service,
        Err(e) => {
            warn!(
                "Failed to resolve worktree service while preserving '{}': {}",
                worktree.id, e
            );
            return None;
        }
    };
    let reference = reference_for_thread(chat_id, thread, &worktree.kind);
    match service.add_reference(&worktree.id, reference).await {
        Ok(view) => Some(view.meta),
        Err(e) => {
            warn!(
                "Failed to add worktree reference '{}' for chat '{}': {}",
                worktree.id, chat_id, e
            );
            None
        }
    }
}

pub async fn resolve_worktree_setparams_update(
    app: AppState,
    chat_id: &str,
    thread: &ThreadParams,
    patch: &serde_json::Value,
) -> Result<Option<WorktreeSetParamsUpdate>, String> {
    let rebinds_worktree = patch.get("worktree_id").is_some_and(|id| {
        id.as_str()
            != thread
                .worktree
                .as_ref()
                .map(|worktree| worktree.id.as_str())
    }) || patch.get("worktree").is_some_and(|worktree| {
        worktree.is_null() || Some(worktree) != serde_json::to_value(&thread.worktree).ok().as_ref()
    });
    if rebinds_worktree {
        if let Some(reason) = crate::agents::seal::seal_reason(&app, chat_id).await {
            return Err(reason.message());
        }
        if app
            .agents
            .find_agent_id_by_child_chat_id(chat_id)
            .await
            .is_some()
        {
            return Err("Background agent child chats cannot change their worktree".to_string());
        }
    }
    if let Some(worktree_id) = patch.get("worktree_id") {
        let worktree_id = worktree_id
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "worktree_id must be a non-empty string".to_string())?;
        if thread
            .worktree
            .as_ref()
            .map(|worktree| worktree.id.as_str())
            == Some(worktree_id)
        {
            return Ok(None);
        }
        let service = worktree_service_from_gcx(app.clone(), None).await?;
        let view = service.get_worktree(worktree_id).await?;
        let reference = reference_for_thread(chat_id, thread, &view.meta.kind);
        let view = service.add_reference(worktree_id, reference).await?;
        if let Some(old) = thread
            .worktree
            .as_ref()
            .filter(|old| old.id != view.meta.id)
        {
            remove_thread_reference(app, chat_id, thread, old).await;
        }
        let changed = thread
            .worktree
            .as_ref()
            .map(|worktree| worktree.id.as_str())
            != Some(view.meta.id.as_str());
        return Ok(Some(WorktreeSetParamsUpdate {
            worktree: Some(view.meta.clone()),
            previous_worktree: thread.worktree.clone(),
            changed,
            sse_value: serde_json::to_value(view.meta).unwrap_or(serde_json::Value::Null),
        }));
    }

    if patch.get("worktree").map_or(false, |value| value.is_null()) {
        if let Some(old) = thread.worktree.as_ref() {
            remove_thread_reference(app, chat_id, thread, old).await;
        }
        return Ok(Some(WorktreeSetParamsUpdate {
            worktree: None,
            previous_worktree: thread.worktree.clone(),
            changed: thread.worktree.is_some(),
            sse_value: serde_json::Value::Null,
        }));
    }

    Ok(None)
}

pub fn process_command_queue(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    processor_running: Arc<AtomicBool>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(process_command_queue_inner(
        app,
        session_arc,
        processor_running,
    ))
}

struct SelectedCommand {
    command: Option<CommandRequest>,
    waiter: Option<Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
}

async fn select_next_command(session_arc: &Arc<AMutex<ChatSession>>) -> Option<SelectedCommand> {
    let mut session = session_arc.lock().await;

    if session.closed {
        return None;
    }

    let mut waiter = Box::pin(session.queue_notify.clone().notified_owned());
    let _ = Pin::as_mut(&mut waiter).enable();
    let waiter: Pin<Box<dyn std::future::Future<Output = ()> + Send>> = waiter;

    let mut index = 0;
    while index < session.command_queue.len() {
        let request = session.command_queue[index].clone();
        if request.supervision_guard.is_some() || request.native_approval_guard.is_some() {
            if let Err(error) = prepare_guarded_command(&mut session, &request) {
                session.command_queue.remove(index);
                session.clear_queue_timestamp(&request.client_request_id);
                reject_guard(&mut session, &request.client_request_id, error);
                session.emit_queue_update();
                continue;
            }
        }
        index += 1;
    }

    let state = session.runtime.state;
    let is_busy = state == SessionState::Generating
        || state == SessionState::ExecutingTools
        || session.turn_depth > 0
        || crate::chat::context_rebuild::compression_attempt_active(&session);

    let (command, waiter) = if is_busy {
        session
            .queue_processor_counters
            .empty_locks
            .fetch_add(1, Ordering::Relaxed);
        (None, Some(waiter))
    } else if session.planner_paused() {
        let command = session
            .command_queue
            .iter()
            .position(|request| matches!(request.command, ChatCommand::Abort {}))
            .and_then(|index| session.command_queue.remove(index));
        if let Some(request) = command.as_ref() {
            session.record_command_queue_wait(&request.client_request_id);
            session.emit_queue_update();
            (command, None)
        } else {
            (None, Some(waiter))
        }
    } else if state == SessionState::WaitingIde {
        if let Some(idx) = find_allowed_command_while_waiting_ide(&session.command_queue) {
            let cmd = session.command_queue.remove(idx);
            if let Some(request) = cmd.as_ref() {
                session.record_command_queue_wait(&request.client_request_id);
            }
            session.emit_queue_update();
            (cmd, None)
        } else {
            session
                .queue_processor_counters
                .empty_locks
                .fetch_add(1, Ordering::Relaxed);
            (None, Some(waiter))
        }
    } else if state == SessionState::Paused {
        if let Some(idx) = find_allowed_command_while_paused(&session.command_queue) {
            let cmd = session.command_queue.remove(idx);
            if let Some(request) = cmd.as_ref() {
                session.record_command_queue_wait(&request.client_request_id);
            }
            session.emit_queue_update();
            (cmd, None)
        } else {
            session
                .queue_processor_counters
                .empty_locks
                .fetch_add(1, Ordering::Relaxed);
            (None, Some(waiter))
        }
    } else if !session.delivery_wake_sources.is_empty() {
        session.delivery_wake_sources.clear();
        session.abort_flag.store(false, Ordering::SeqCst);
        session.user_interrupt_flag.store(false, Ordering::SeqCst);
        session.set_runtime_state(SessionState::Idle, None);
        (
            Some(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: format!("delivery-wake-{}", uuid::Uuid::new_v4()),
                priority: false,
                command: ChatCommand::Regenerate {},
            }),
            None,
        )
    } else if session.command_queue.is_empty() {
        session
            .queue_processor_counters
            .empty_locks
            .fetch_add(1, Ordering::Relaxed);
        (None, Some(waiter))
    } else {
        let cmd = session.command_queue.pop_front();
        if let Some(ref req) = cmd {
            session.record_command_queue_wait(&req.client_request_id);
            if matches!(req.command, ChatCommand::Regenerate {}) {
                for request_id in
                    drain_redundant_regenerates(&mut session.command_queue, req.priority)
                {
                    session.record_command_queue_wait(&request_id);
                }
            }
            if command_triggers_generation(&req.command) {
                session.set_runtime_state(SessionState::Idle, None);
            }
        }
        session.emit_queue_update();
        (cmd, None)
    };

    Some(SelectedCommand { command, waiter })
}

async fn process_command_queue_inner(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    processor_running: Arc<AtomicBool>,
) {
    struct ProcessorGuard {
        app: AppState,
        processor_running: Arc<AtomicBool>,
        session_arc: Arc<AMutex<ChatSession>>,
    }

    impl Drop for ProcessorGuard {
        fn drop(&mut self) {
            let app = self.app.clone();
            let processor_running = self.processor_running.clone();
            let session_arc = self.session_arc.clone();
            tokio::spawn(async move {
                let restart = {
                    let session = session_arc.lock().await;
                    session
                        .queue_processor_counters
                        .processor_exits
                        .fetch_add(1, Ordering::Relaxed);
                    processor_running.store(false, Ordering::SeqCst);
                    !session.closed && !session.command_queue.is_empty()
                };
                if restart {
                    if !processor_running.swap(true, Ordering::SeqCst) {
                        tokio::spawn(process_command_queue(app, session_arc, processor_running));
                    }
                }
            });
        }
    }
    let _guard = ProcessorGuard {
        app: app.clone(),
        processor_running,
        session_arc: session_arc.clone(),
    };

    {
        let session = session_arc.lock().await;
        session
            .queue_processor_counters
            .processor_starts
            .fetch_add(1, Ordering::Relaxed);
    }

    loop {
        let Some(SelectedCommand { command, waiter }) = select_next_command(&session_arc).await
        else {
            return;
        };

        if let Some(waiter) = waiter {
            waiter.await;
            let session = session_arc.lock().await;
            session
                .queue_processor_counters
                .notify_wakes
                .fetch_add(1, Ordering::Relaxed);
            continue;
        }

        let Some(request) = command else {
            continue;
        };

        {
            let mut session = session_arc.lock().await;
            if let Err(error) = prepare_guarded_command(&mut session, &request) {
                reject_guard(&mut session, &request.client_request_id, error);
                continue;
            }
        }
        let outcome = Box::pin(command_handlers::handle_queue_command(
            app.clone(),
            session_arc.clone(),
            request,
        ))
        .await;
        if outcome == command_handlers::QueueCommandOutcome::Continue {
            continue;
        }

        // A command finished: land any delivery whose boundary just opened.
        super::delivery::drain_deliveries_at_boundary(app.clone(), session_arc.clone()).await;
    }
}

/// Delivered user-visible messages start a fresh pursuit turn, exactly like a
/// genuine user message, without going through the `UserMessage` command path.
async fn note_delivered_messages_reset_goal_pursuit(session_arc: &Arc<AMutex<ChatSession>>) {
    let mut session = session_arc.lock().await;
    note_user_turn_resets_goal_pursuit(&mut session);
}

fn is_plan_delta_event(msg: &ChatMessage) -> bool {
    msg.role == "event"
        && msg
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(|subkind| subkind.as_str())
            == Some("plan_delta")
}

fn is_goal_delta_event(msg: &ChatMessage) -> bool {
    msg.role == "event"
        && msg
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(|subkind| subkind.as_str())
            == Some("goal_delta")
}

fn is_goal_pursuit_event(msg: &ChatMessage) -> bool {
    msg.role == "event"
        && msg
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(|subkind| subkind.as_str())
            == Some("goal_pursuit")
}

fn is_goal_hidden_event(msg: &ChatMessage) -> bool {
    is_goal_delta_event(msg) || is_goal_pursuit_event(msg)
}

fn hidden_role_extra(msg: &ChatMessage) -> serde_json::Map<String, serde_json::Value> {
    if matches!(
        msg.role.as_str(),
        "plan" | "goal" | "event" | "compression_report"
    ) {
        msg.extra.clone()
    } else {
        serde_json::Map::new()
    }
}

fn is_allowed_for_restore(msg: &ChatMessage) -> bool {
    matches!(
        msg.role.as_str(),
        "user" | "assistant" | "system" | "tool" | "plan" | "goal" | "compression_report"
    ) || is_plan_delta_event(msg)
        || is_goal_hidden_event(msg)
}

/// Sanitize message for restoring from external trajectory — strips tool_calls for security
/// and transient metadata.
fn sanitize_message_for_restore(msg: &ChatMessage) -> ChatMessage {
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: msg.role.clone(),
        content: msg.content.clone(),
        tool_calls: None, // Security: strip tool_calls to prevent prerun of restored messages
        tool_call_id: msg.tool_call_id.clone(),
        tool_failed: msg.tool_failed,
        preserve: msg.preserve,
        reasoning_content: msg.reasoning_content.clone(),
        thinking_blocks: msg.thinking_blocks.clone(),
        citations: msg.citations.clone(),
        server_content_blocks: msg.server_content_blocks.clone(),
        summarized_range: msg.summarized_range,
        summarization_tier: msg.summarization_tier.clone(),
        summarized_token_estimate: msg.summarized_token_estimate,
        extra: hidden_role_extra(msg),
        ..Default::default()
    }
}

/// Sanitize message for branching — preserves the conversation structure (including tool_calls
/// and context_file messages) but strips thinking blocks and transient metadata.
fn sanitize_message_for_branch(msg: &ChatMessage) -> ChatMessage {
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: msg.role.clone(),
        content: msg.content.clone(),
        tool_calls: msg.tool_calls.clone(),
        tool_call_id: msg.tool_call_id.clone(),
        tool_failed: msg.tool_failed,
        preserve: msg.preserve,
        checkpoints: msg.checkpoints.clone(),
        reasoning_content: msg.reasoning_content.clone(),
        citations: msg.citations.clone(),
        server_content_blocks: msg.server_content_blocks.clone(),
        summarized_range: msg.summarized_range,
        summarization_tier: msg.summarization_tier.clone(),
        summarized_token_estimate: msg.summarized_token_estimate,
        extra: hidden_role_extra(msg),
        ..Default::default()
    }
}

fn is_allowed_for_branch(msg: &ChatMessage) -> bool {
    matches!(
        msg.role.as_str(),
        "user"
            | "assistant"
            | "system"
            | "tool"
            | "context_file"
            | "plan"
            | "goal"
            | "compression_report"
    ) || is_plan_delta_event(msg)
        || is_goal_hidden_event(msg)
}

pub(crate) async fn handle_tool_decisions_guarded(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
    native_guard: Option<&refact_chat_api::NativeApprovalGuard>,
    request_id: &str,
) {
    let hooked = crate::chat::permission_decision_hooks::apply_confirmation_permission_hooks(
        &app,
        &session_arc,
        decisions,
    )
    .await;
    let decisions: &[ToolDecisionItem] = &hooked.decisions;
    let root = native_guard_root(&app, native_guard).await;
    let is_shell_privacy_pause = {
        let session = session_arc.lock().await;
        session
            .runtime
            .pause_reasons
            .iter()
            .any(crate::chat::tools::is_shell_privacy_approval)
    };

    if is_shell_privacy_pause {
        handle_shell_privacy_tool_decisions(
            app.clone(),
            session_arc.clone(),
            decisions,
            &hooked.denial_reasons,
            native_guard,
            request_id,
            root.as_deref(),
        )
        .await;
        return;
    }

    let is_cache_guard_pause = {
        let session = session_arc.lock().await;
        session
            .runtime
            .pause_reasons
            .iter()
            .any(crate::chat::cache_guard::is_cache_guard_pause_reason)
    };

    if is_cache_guard_pause {
        handle_cache_guard_tool_decisions(
            app,
            session_arc,
            decisions,
            &hooked.denial_reasons,
            native_guard,
            request_id,
            root.as_deref(),
        )
        .await;
        return;
    }

    handle_regular_tool_decisions(
        app,
        session_arc,
        decisions,
        &hooked.denial_reasons,
        native_guard,
        request_id,
        root.as_deref(),
    )
    .await;
}

async fn handle_shell_privacy_tool_decisions(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
    _denial_reasons: &std::collections::HashMap<String, String>,
    native_guard: Option<&refact_chat_api::NativeApprovalGuard>,
    request_id: &str,
    root: Option<&str>,
) {
    let should_continue = {
        let mut session = session_arc.lock().await;
        if !consume_tool_guard(&mut session, native_guard, root, request_id, decisions) {
            return;
        }
        let privacy_ids = session
            .runtime
            .pause_reasons
            .iter()
            .filter(|reason| crate::chat::tools::is_shell_privacy_approval(reason))
            .map(|reason| reason.tool_call_id.clone())
            .collect::<std::collections::HashSet<_>>();
        let accepted_ids = decisions
            .iter()
            .filter(|decision| decision.accepted && privacy_ids.contains(&decision.tool_call_id))
            .map(|decision| decision.tool_call_id.clone())
            .collect::<Vec<_>>();
        let rejected_ids = decisions
            .iter()
            .filter(|decision| !decision.accepted && privacy_ids.contains(&decision.tool_call_id))
            .map(|decision| decision.tool_call_id.clone())
            .collect::<Vec<_>>();
        let decided = accepted_ids
            .iter()
            .chain(&rejected_ids)
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let remaining = session
            .runtime
            .pause_reasons
            .iter()
            .filter(|reason| !decided.contains(&reason.tool_call_id))
            .cloned()
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            session.complete_confirmation_wait();
        }

        let updates = session
            .messages
            .iter()
            .filter(|message| decided.contains(&message.tool_call_id))
            .filter_map(|message| {
                let mut updated = message.clone();
                let accepted = accepted_ids.contains(&message.tool_call_id);
                crate::privacy::records::resolve_shell_ask(&mut updated, accepted)
                    .then_some(updated)
            })
            .collect::<Vec<_>>();
        for updated in updates {
            let message_id = updated.message_id.clone();
            session.update_message(&message_id, updated);
        }
        session.add_tool_decision_event("approve", accepted_ids, "once");
        session.add_tool_decision_event("reject", rejected_ids, "once");
        session.drain_post_tool_side_effects();
        let remaining = session
            .runtime
            .pause_reasons
            .iter()
            .filter(|reason| !decided.contains(&reason.tool_call_id))
            .cloned()
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            session.complete_confirmation_wait();
            session.release_turn_only_state();
            session.set_runtime_state(SessionState::Idle, None);
            true
        } else {
            session.set_paused_with_reasons_and_auto_approved(remaining, Vec::new(), None);
            false
        }
    };
    if should_continue && !aborted_before_start_generation(&session_arc).await {
        start_generation(app, session_arc).await;
    }
}

async fn handle_cache_guard_tool_decisions(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
    _denial_reasons: &std::collections::HashMap<String, String>,
    native_guard: Option<&refact_chat_api::NativeApprovalGuard>,
    request_id: &str,
    root: Option<&str>,
) {
    let accepted_any = decisions.iter().any(|d| d.accepted);

    {
        let mut session = session_arc.lock().await;
        if !consume_tool_guard(&mut session, native_guard, root, request_id, decisions) {
            return;
        }
        if accepted_any {
            session.reset_cache_guard_snapshot();
        }
        let approved_ids = decisions
            .iter()
            .filter(|d| d.accepted)
            .map(|d| d.tool_call_id.clone())
            .collect::<Vec<_>>();
        let rejected_ids = decisions
            .iter()
            .filter(|d| !d.accepted)
            .map(|d| d.tool_call_id.clone())
            .collect::<Vec<_>>();
        session.add_tool_decision_event("approve", approved_ids, "once");
        session.add_tool_decision_event("reject", rejected_ids, "once");
        session.drain_post_tool_side_effects();
        session.runtime.pause_reasons.clear();
        session.runtime.accepted_tool_ids.clear();
        session.runtime.auto_approved_tool_ids.clear();
        session.runtime.paused_message_index = None;
        session.complete_confirmation_wait();
        if accepted_any {
            session.release_turn_only_state();
            session.set_runtime_state(SessionState::Idle, None);
        } else {
            session.set_runtime_state(SessionState::Idle, None);
            session.release_turn_only_state();
        }
    }

    if accepted_any {
        if !aborted_before_start_generation(&session_arc).await {
            start_generation(app.clone(), session_arc.clone()).await;
        }
    } else {
        maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await;
    }
}

async fn handle_regular_tool_decisions(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
    denial_reasons: &std::collections::HashMap<String, String>,
    native_guard: Option<&refact_chat_api::NativeApprovalGuard>,
    request_id: &str,
    root: Option<&str>,
) {
    let (
        auto_approved_ids,
        has_remaining_pauses,
        tool_calls_to_execute,
        messages,
        thread,
        any_rejected,
    ) = {
        let mut session = session_arc.lock().await;
        if !consume_tool_guard(
            &mut session,
            native_guard,
            root.as_deref(),
            request_id,
            decisions,
        ) {
            return;
        }
        let auto_approved = session.runtime.auto_approved_tool_ids.clone();
        let paused_msg_idx = session.runtime.paused_message_index;
        let outcome = session.process_tool_decisions_with_reasons(decisions, denial_reasons);
        let any_rejected = !outcome.denied_ids.is_empty();

        for id in &outcome.denied_ids {
            if let Some(call) = session.approval_wrapper_calls.remove(id) {
                if let Ok(args) =
                    serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                {
                    if args.get("action").and_then(serde_json::Value::as_str) == Some("approve") {
                        if let Some(request_id) =
                            args.get("request_id").and_then(serde_json::Value::as_str)
                        {
                            let now = crate::agents::approval::now_ms();
                            if let Ok(request) =
                                app.agent_approvals.get(&session.thread.id, request_id, now)
                            {
                                let _ = app.agent_approvals.decide(
                                    &session.thread.id,
                                    request.run_generation,
                                    request_id,
                                    false,
                                    now,
                                );
                            }
                        }
                    }
                }
            }
        }

        for id in &outcome.accepted_ids {
            if !session.runtime.accepted_tool_ids.contains(id) {
                session.runtime.accepted_tool_ids.push(id.clone());
            }
        }

        let remaining = !session.runtime.pause_reasons.is_empty();

        let mut ids_to_execute: std::collections::HashSet<String> =
            session.runtime.accepted_tool_ids.iter().cloned().collect();
        if !any_rejected && !remaining {
            for id in &auto_approved {
                ids_to_execute.insert(id.clone());
            }
        }

        let mut tool_calls: Vec<crate::call_validation::ChatToolCall> =
            if let Some(msg_idx) = paused_msg_idx {
                session
                    .messages
                    .get(msg_idx)
                    .and_then(|m| m.tool_calls.as_ref())
                    .map(|tcs| {
                        tcs.iter()
                            .filter(|tc| ids_to_execute.contains(&tc.id))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                session
                    .messages
                    .iter()
                    .filter_map(|m| m.tool_calls.as_ref())
                    .flatten()
                    .filter(|tc| ids_to_execute.contains(&tc.id))
                    .cloned()
                    .collect()
            };

        let invalid_wrappers: Vec<_> = session
            .approval_wrapper_calls
            .iter()
            .filter(|(id, _)| ids_to_execute.contains(*id))
            .filter(|(_, original)| {
                !tool_calls.iter().any(|call| {
                    call.id == original.id
                        && call.function.name == original.function.name
                        && call.function.arguments == original.function.arguments
                })
            })
            .map(|(id, call)| (id.clone(), call.clone()))
            .collect();
        for (id, original) in invalid_wrappers {
            if let Ok(args) =
                serde_json::from_str::<serde_json::Value>(&original.function.arguments)
            {
                if let Some(request_id) = args.get("request_id").and_then(serde_json::Value::as_str)
                {
                    let now = crate::agents::approval::now_ms();
                    if let Ok(request) =
                        app.agent_approvals.get(&session.thread.id, request_id, now)
                    {
                        let _ = app.agent_approvals.cancel(
                            &session.thread.id,
                            request.run_generation,
                            request_id,
                            now,
                        );
                    }
                }
            }
            tool_calls.retain(|call| call.id != id);
            session
                .runtime
                .accepted_tool_ids
                .retain(|accepted| accepted != &id);
            session.approval_wrapper_calls.remove(&id);
            if session
                .messages
                .iter()
                .filter_map(|message| message.tool_calls.as_ref())
                .flatten()
                .any(|call| call.id == id)
                && !session
                    .messages
                    .iter()
                    .any(|message| message.role == "tool" && message.tool_call_id == id)
            {
                session.add_message(ChatMessage {
                    role: "tool".into(),
                    tool_call_id: id,
                    tool_failed: Some(true),
                    content: ChatContent::SimpleText(
                        "Child approval wrapper changed while awaiting the user decision; no authority granted."
                            .into(),
                    ),
                    ..Default::default()
                });
            }
        }
        tool_calls.retain(|call| {
            if call.function.name != "agent_approval"
                && !session.approval_wrapper_calls.contains_key(&call.id)
            {
                return true;
            }
            let matches = session
                .approval_wrapper_calls
                .get(&call.id)
                .is_some_and(|original| {
                    original.function.name == call.function.name
                        && original.function.arguments == call.function.arguments
                });
            if !matches {
                session.add_message(ChatMessage {
                    role: "tool".into(),
                    tool_call_id: call.id.clone(),
                    tool_failed: Some(true),
                    content: ChatContent::SimpleText(
                        "Child approval wrapper changed while awaiting the user decision; no authority granted."
                            .into(),
                    ),
                    ..Default::default()
                });
            }
            matches
        });
        for call in &tool_calls {
            if remaining
                || !session.runtime.accepted_tool_ids.contains(&call.id)
                || call.function.name != "agent_approval"
            {
                continue;
            }
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&call.function.arguments)
            else {
                continue;
            };
            if args.get("action").and_then(serde_json::Value::as_str) != Some("approve") {
                continue;
            }
            let Some(request_id) = args.get("request_id").and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let now = crate::agents::approval::now_ms();
            if let Ok(request) = app.agent_approvals.get(&session.thread.id, request_id, now) {
                let _ = app.agent_approvals.grant_user_once(
                    &session.thread.id,
                    request_id,
                    request.run_generation,
                    &call.id,
                    now,
                );
            }
        }

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
            let already_handled = session
                .messages
                .iter()
                .any(|m| m.role == "tool" && m.tool_call_id == *id);
            if already_handled {
                continue;
            }
            let tool_message = ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText(
                    "Tool execution skipped due to user rejection of related tools".to_string(),
                ),
                tool_call_id: id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            };
            session.add_message(tool_message);
        }
        session.drain_post_tool_side_effects();
    }

    execute_regular_tool_calls(
        app,
        session_arc,
        tool_calls_to_execute,
        messages,
        thread,
        any_rejected,
    )
    .await;
}

async fn execute_regular_tool_calls(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    tool_calls_to_execute: Vec<crate::call_validation::ChatToolCall>,
    messages: Vec<ChatMessage>,
    thread: ThreadParams,
    any_rejected: bool,
) {
    let had_tool_calls = !tool_calls_to_execute.is_empty();
    if had_tool_calls {
        let session_catalog = {
            let session = session_arc.lock().await;
            session.tool_catalog.clone()
        };
        let catalog = match session_catalog {
            Some(catalog) => catalog,
            None => {
                app.tool_registry
                    .acquire_tool_catalog(
                        &thread.mode,
                        Some(&thread.model),
                        thread
                            .worktree
                            .as_ref()
                            .map(|worktree| worktree.root.to_string_lossy().into_owned())
                            .as_deref(),
                    )
                    .await
            }
        };
        let tool_calls_to_execute =
            resolve_tool_call_aliases_with_catalog(tool_calls_to_execute, &catalog);
        let turn_tool_pool = acquire_session_turn_tool_pool(
            &app,
            &session_arc,
            &thread,
            &thread.mode,
            Some(&thread.model),
            &catalog,
        )
        .await;

        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::ExecutingTools, None);
        }

        let (tool_results, _) = execute_tools_with_session(
            app.clone(),
            session_arc.clone(),
            &tool_calls_to_execute,
            &messages,
            &thread,
            &thread.mode,
            Some(thread.model.as_str()),
            super::tools::ExecuteToolsOptions {
                catalog: Some(catalog),
                turn_tool_pool,
                ..Default::default()
            },
        )
        .await;

        let privacy_approvals =
            super::tools::shell_privacy_approval_reasons(&tool_calls_to_execute, &tool_results);
        if !privacy_approvals.is_empty() {
            let mut session = session_arc.lock().await;
            for result_msg in tool_results {
                session.add_message(result_msg);
            }
            session.drain_post_tool_side_effects();
            session.set_paused_with_reasons_and_auto_approved(privacy_approvals, Vec::new(), None);
            return;
        }

        // Determine tool-requested final state before checking abort.
        // Some tools (ask_questions/task_done/agent_finish) set abort_flag=true as part of
        // normal operation to stop further LLM generation.
        let mut final_state = SessionState::Idle;
        let mut completion_trigger: Option<String> = None;
        for tool_call in &tool_calls_to_execute {
            let tool_name =
                crate::llm::adapters::claude_code_compat::cc_normalize_internal_tool_name(
                    &tool_call.function.name,
                );
            match tool_name.as_str() {
                "ask_questions" | "wait_agents" => final_state = SessionState::WaitingUserInput,
                "task_done" | "finish" => {
                    final_state = SessionState::Completed;
                    completion_trigger = Some(tool_name.to_string());
                }
                "agent_finish" => {
                    final_state = SessionState::Completed;
                    completion_trigger = Some(tool_name.to_string());
                }
                _ => {}
            }
        }

        let tool_initiated_stop = matches!(
            final_state,
            SessionState::Completed | SessionState::WaitingUserInput
        );

        // Check if we were aborted during tool execution
        let was_aborted = {
            let session = session_arc.lock().await;
            session
                .abort_flag
                .load(std::sync::atomic::Ordering::Relaxed)
        };

        let mut verify_completion = false;
        {
            let mut session = session_arc.lock().await;
            for result_msg in tool_results {
                session.add_message(result_msg);
            }
            session.drain_post_tool_side_effects();
            if tool_initiated_stop {
                if final_state == SessionState::Completed
                    && completion_trigger.is_some()
                    && should_verify_goal_on_done(&session)
                {
                    session.set_runtime_state(SessionState::ExecutingTools, None);
                    verify_completion = true;
                } else {
                    session.set_runtime_state(final_state, None);
                }
            } else if was_aborted {
                session.set_runtime_state(SessionState::Idle, None);
                session.release_turn_only_state();
            } else {
                session.release_turn_only_state();
                session.set_runtime_state(SessionState::Idle, None);
            }
        }

        if verify_completion {
            let trigger = completion_trigger.as_deref().unwrap_or("task_done");
            match verify_goal_before_completion(app.clone(), session_arc.clone(), trigger).await {
                GoalCompletionGateOutcome::Passthrough => {
                    let mut session = session_arc.lock().await;
                    session.set_runtime_state(SessionState::Completed, None);
                }
                GoalCompletionGateOutcome::Finalized => {}
                GoalCompletionGateOutcome::Rearmed | GoalCompletionGateOutcome::Waiting => {
                    maybe_save_trajectory_with_intent(
                        app.clone(),
                        session_arc.clone(),
                        TrajectoryCommitIntent::Required,
                    )
                    .await;
                    return;
                }
                GoalCompletionGateOutcome::BudgetExhausted(_) => {}
                GoalCompletionGateOutcome::VerificationUnavailable
                | GoalCompletionGateOutcome::VerificationStalled => {
                    let mut session = session_arc.lock().await;
                    session.set_runtime_state(SessionState::Completed, None);
                }
                GoalCompletionGateOutcome::Aborted => {
                    maybe_save_trajectory_with_intent(
                        app.clone(),
                        session_arc.clone(),
                        TrajectoryCommitIntent::Required,
                    )
                    .await;
                    return;
                }
            }
        }

        if was_aborted || tool_initiated_stop {
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        } else {
            maybe_save_trajectory_background_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Checkpoint,
            );
        }

        if was_aborted || tool_initiated_stop {
            return;
        }
    }

    if any_rejected {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
            session.release_turn_only_state();
        }
        maybe_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required).await;
    } else if had_tool_calls {
        if !aborted_before_start_generation(&session_arc).await {
            start_generation(app, session_arc).await;
        }
    } else {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
            session.release_turn_only_state();
        }
        maybe_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required).await;
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

async fn create_checkpoint_async(
    app: AppState,
    latest_checkpoint: Option<&crate::git::checkpoints::Checkpoint>,
    chat_id: &str,
    worktree: Option<&crate::worktrees::types::WorktreeMeta>,
) -> Vec<crate::git::checkpoints::Checkpoint> {
    use crate::git::checkpoints::{create_workspace_checkpoint, create_workspace_checkpoint_for_root};

    let result = if let Some(worktree) = worktree {
        create_workspace_checkpoint_for_root(
            app.gcx.clone(),
            &worktree.root,
            latest_checkpoint,
            chat_id,
        )
        .await
    } else {
        create_workspace_checkpoint(app.gcx.clone(), latest_checkpoint, chat_id).await
    };

    match result {
        Ok((checkpoint, _)) => {
            tracing::info!("Checkpoint created for chat {}: {:?}", chat_id, checkpoint);
            vec![checkpoint]
        }
        Err(e) => {
            warn!("Failed to create checkpoint for chat {}: {}", chat_id, e);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::types::DEFAULT_EXEC_OUTPUT_LIMIT_BYTES;
    use crate::exec::{ExecMode, ExecOwnerMeta, ExecProcessId, ExecProcessMeta};
    use crate::yaml_configs::customization_types::{ModeOverride, ModeThreadDefaults, ProjectRegistry};
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::process::Command;

    #[tokio::test]
    async fn guarded_tool_decision_rechecks_source_after_waiting_for_session_lock() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            workspace.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let root = crate::http::routers::v1::buddy_decisions::decision_root(&app)
            .await
            .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new("guard-toctou".into())));
        let mut session = session_arc.lock().await;
        session.runtime.state = SessionState::Paused;
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "shell".into(),
            rule: "ask".into(),
            tool_call_id: "pending".into(),
            integr_config_path: None,
        });
        let guard = refact_chat_api::NativeApprovalGuard {
            project_root: root.clone(),
            engine_identity:
                crate::http::routers::v1::buddy_native_approvals::native_engine_identity().into(),
            source_version: crate::http::routers::v1::buddy_native_approvals::source_version(
                &session, &root,
            ),
        };
        assert!(consume_native_guard(
            &mut session,
            Some(&guard),
            Some(&root),
            "preflight"
        ));
        let mut events = session.subscribe();
        let worker_session = session_arc.clone();
        let task = tokio::spawn(async move {
            handle_tool_decisions_guarded(
                app,
                worker_session,
                &[ToolDecisionItem {
                    tool_call_id: "pending".into(),
                    accepted: true,
                }],
                Some(&guard),
                "stale",
            )
            .await;
        });
        session.runtime.pause_reasons[0].command = "replacement command".into();
        drop(session);
        task.await.unwrap();
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.state, SessionState::Paused);
        assert_eq!(
            session.runtime.pause_reasons[0].command,
            "replacement command"
        );
        assert!(session.runtime.accepted_tool_ids.is_empty());
        assert!(session.messages.is_empty());
        let ack: serde_json::Value = serde_json::from_str(&events.try_recv().unwrap()).unwrap();
        assert_eq!(ack["accepted"], false);
        assert!(events.try_recv().is_ok());
    }

    #[tokio::test]
    async fn queued_browser_approval_preserves_replaced_pending_request() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            workspace.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let root = crate::http::routers::v1::buddy_decisions::decision_root(&app)
            .await
            .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new("guard-browser".into())));
        let mut session = session_arc.lock().await;
        session.runtime.state = SessionState::WaitingUserInput;
        session.pending_browser_message = Some(PendingBrowserMessage {
            pending_message_id: "old-browser".into(),
            content: json!("pending message"),
            attachments: vec![],
            client_message_id: None,
            checkpoints: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            skill_activation_name: None,
            skill_context_msg: None,
        });
        let guard = refact_chat_api::NativeApprovalGuard {
            project_root: root.clone(),
            engine_identity:
                crate::http::routers::v1::buddy_native_approvals::native_engine_identity().into(),
            source_version: crate::http::routers::v1::buddy_native_approvals::source_version(
                &session, &root,
            ),
        };
        let mut request = make_request(ChatCommand::BrowserContextDecision {
            pending_message_id: "old-browser".into(),
            include_actions: false,
            include_console: false,
            include_network: false,
            include_mutations: false,
            include_screenshot: false,
            last_n_actions: None,
            last_n_console: None,
            last_n_network: None,
        });
        request.native_approval_guard = Some(guard);
        assert_eq!(
            session.enqueue_command(request),
            EnqueueCommandOutcome::Accepted
        );
        session
            .pending_browser_message
            .as_mut()
            .unwrap()
            .pending_message_id = "new-browser".into();
        let mut events = session.subscribe();
        let running = session.queue_processor_running.clone();
        running.store(true, Ordering::SeqCst);
        drop(session);
        let worker = tokio::spawn(process_command_queue(app, session_arc.clone(), running));
        loop {
            let event: serde_json::Value =
                serde_json::from_str(&events.recv().await.unwrap()).unwrap();
            if event["client_request_id"] == "req-1" {
                assert_eq!(event["accepted"], false);
                break;
            }
        }
        let mut session = session_arc.lock().await;
        assert_eq!(
            session
                .pending_browser_message
                .as_ref()
                .unwrap()
                .pending_message_id,
            "new-browser"
        );
        assert!(session.messages.is_empty());
        assert_eq!(session.runtime.state, SessionState::WaitingUserInput);
        session.close_event_channel();
        drop(session);
        worker.await.unwrap();
    }

    #[test]
    fn guarded_command_wire_roundtrip_and_wrong_kind_fail_closed() {
        let mut request = make_request(ChatCommand::Abort {});
        request.native_approval_guard = Some(refact_chat_api::NativeApprovalGuard {
            project_root: "p".into(),
            engine_identity: "e".into(),
            source_version: "v".into(),
        });
        let decoded: CommandRequest =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert_eq!(decoded.native_approval_guard, request.native_approval_guard);
        let mut session = ChatSession::new("guard-kind".into());
        assert!(prepare_guarded_command(&mut session, &decoded).is_err());
        assert!(session.command_queue.is_empty());
        let legacy: CommandRequest =
            serde_json::from_value(json!({"client_request_id":"legacy", "type":"abort"})).unwrap();
        assert!(legacy.native_approval_guard.is_none());
        assert!(legacy.supervision_guard.is_none());
    }

    fn test_mode_config(id: &str) -> ModeConfig {
        ModeConfig {
            schema_version: 1,
            id: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            specific: false,
            prompt: String::new(),
            plan_template: String::new(),
            tools: Vec::new(),
            allow_integrations: false,
            allow_mcp: false,
            allow_subagents: false,
            model_defaults: Default::default(),
            tool_confirm: Default::default(),
            thread_defaults: Default::default(),
            ui: Default::default(),
            base: None,
            match_models: None,
            override_config: None,
        }
    }

    fn make_request(cmd: ChatCommand) -> CommandRequest {
        CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-1".into(),
            priority: false,
            command: cmd,
        }
    }

    fn abort_request(request_id: &str) -> CommandRequest {
        CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: request_id.to_string(),
            priority: false,
            command: ChatCommand::Abort {},
        }
    }

    const QUEUE_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    async fn wait_for_empty_lock(session_arc: &Arc<AMutex<ChatSession>>) {
        tokio::time::timeout(QUEUE_TEST_TIMEOUT, async {
            loop {
                let waiting = session_arc
                    .lock()
                    .await
                    .queue_processor_counters
                    .empty_locks
                    .load(Ordering::Relaxed);
                if waiting > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queue processor did not wait for notification");
    }

    async fn wait_for_queue_to_drain(session_arc: &Arc<AMutex<ChatSession>>) {
        tokio::time::timeout(QUEUE_TEST_TIMEOUT, async {
            loop {
                if session_arc.lock().await.command_queue.is_empty() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("queue processor did not drain queued command");
    }

    async fn close_processor(
        session_arc: &Arc<AMutex<ChatSession>>,
        handle: tokio::task::JoinHandle<()>,
    ) {
        session_arc.lock().await.close_event_channel();
        tokio::time::timeout(QUEUE_TEST_TIMEOUT, handle)
            .await
            .expect("queue processor did not exit after session close")
            .expect("queue processor panicked");
    }

    fn user_message_command(content: &str, client_message_id: Option<&str>) -> ChatCommand {
        ChatCommand::UserMessage {
            content: json!(content),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: client_message_id.map(str::to_string),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn queue_notify_idle_processors_do_not_wake_without_notifications() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-idle".to_string(),
        )));
        let processor_running = session_arc.lock().await.queue_processor_running.clone();
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));

        wait_for_empty_lock(&session_arc).await;
        let counters = session_arc.lock().await.queue_processor_counters.clone();
        let empty_locks = counters.empty_locks.load(Ordering::Relaxed);
        let notify_wakes = counters.notify_wakes.load(Ordering::Relaxed);

        tokio::time::advance(std::time::Duration::from_secs(3_600)).await;
        tokio::task::yield_now().await;

        assert_eq!(counters.empty_locks.load(Ordering::Relaxed), empty_locks);
        assert_eq!(counters.notify_wakes.load(Ordering::Relaxed), notify_wakes);
        assert_eq!(counters.timeout_wakes.load(Ordering::Relaxed), 0);

        close_processor(&session_arc, handle).await;
    }

    #[tokio::test]
    async fn queue_notify_processes_commands_enqueued_before_and_after_wait_registration() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let before_wait = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-before".to_string(),
        )));
        let before_running = before_wait.lock().await.queue_processor_running.clone();
        {
            let mut session = before_wait.lock().await;
            assert_eq!(
                session.enqueue_accepted_command(abort_request("before-wait")),
                EnqueueCommandOutcome::Accepted
            );
        }
        before_running.store(true, Ordering::SeqCst);
        let before_handle = tokio::spawn(process_command_queue(
            app.clone(),
            before_wait.clone(),
            before_running,
        ));
        wait_for_queue_to_drain(&before_wait).await;
        close_processor(&before_wait, before_handle).await;

        let after_wait = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-after".to_string(),
        )));
        let after_running = after_wait.lock().await.queue_processor_running.clone();
        after_running.store(true, Ordering::SeqCst);
        let after_handle = tokio::spawn(process_command_queue(
            app,
            after_wait.clone(),
            after_running,
        ));
        wait_for_empty_lock(&after_wait).await;
        {
            let mut session = after_wait.lock().await;
            assert_eq!(
                session.enqueue_accepted_command(abort_request("after-wait")),
                EnqueueCommandOutcome::Accepted
            );
        }
        wait_for_queue_to_drain(&after_wait).await;
        let counters = after_wait.lock().await.queue_processor_counters.clone();
        assert!(counters.notify_wakes.load(Ordering::Relaxed) >= 1);
        assert_eq!(counters.timeout_wakes.load(Ordering::Relaxed), 0);
        close_processor(&after_wait, after_handle).await;
    }

    #[tokio::test]
    async fn process_command_queue_restarts_after_enqueue_races_processor_teardown() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-teardown".to_string(),
        )));
        let processor_running = session_arc.lock().await.queue_processor_running.clone();
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));
        wait_for_empty_lock(&session_arc).await;

        {
            let mut session = session_arc.lock().await;
            session
                .command_queue
                .push_back(abort_request("teardown-race"));
            session.queue_notify.notify_one();
            handle.abort();
        }
        let _ = handle.await;

        wait_for_queue_to_drain(&session_arc).await;
        let counters = session_arc.lock().await.queue_processor_counters.clone();
        assert!(counters.processor_starts.load(Ordering::Relaxed) >= 2);
        assert!(counters.processor_exits.load(Ordering::Relaxed) >= 1);
        let next_processor = session_arc.lock().await.queue_processor_running.clone();
        assert!(next_processor.load(Ordering::SeqCst));
        session_arc.lock().await.close_event_channel();
        tokio::time::timeout(QUEUE_TEST_TIMEOUT, async {
            while next_processor.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("restarted queue processor did not exit after session close");
    }

    #[tokio::test]
    async fn process_command_queue_wakes_for_paused_decision_enqueued_after_wait() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-paused".to_string(),
        )));
        let processor_running = session_arc.lock().await.queue_processor_running.clone();
        {
            let mut session = session_arc.lock().await;
            session.runtime.pause_reasons.push(PauseReason {
                reason_type: "confirmation".to_string(),
                tool_name: "shell".to_string(),
                command: "shell".to_string(),
                rule: "ask".to_string(),
                tool_call_id: "paused-tool".to_string(),
                integr_config_path: None,
            });
            session.set_runtime_state(SessionState::Paused, None);
        }
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));

        wait_for_empty_lock(&session_arc).await;
        {
            let mut session = session_arc.lock().await;
            assert_eq!(
                session.enqueue_accepted_command(CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: "paused-decision".to_string(),
                    priority: false,
                    command: ChatCommand::ToolDecision {
                        tool_call_id: "paused-tool".to_string(),
                        accepted: false,
                    },
                }),
                EnqueueCommandOutcome::Accepted
            );
        }
        wait_for_queue_to_drain(&session_arc).await;
        assert_eq!(session_arc.lock().await.runtime.state, SessionState::Idle);
        close_processor(&session_arc, handle).await;
    }

    #[tokio::test]
    async fn process_command_queue_wakes_for_waiting_ide_callback_enqueued_after_wait() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-waiting-ide".to_string(),
        )));
        let processor_running = session_arc.lock().await.queue_processor_running.clone();
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "ide-tool".to_string(),
                    index: None,
                    tool_type: "function".to_string(),
                    function: crate::call_validation::ChatToolFunction {
                        name: "ide_action".to_string(),
                        arguments: "{}".to_string(),
                    },
                    extra_content: None,
                    started_at_ms: None,
                    completed_at_ms: None,
                }]),
                ..Default::default()
            });
            session.set_runtime_state(SessionState::WaitingIde, None);
        }
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));

        wait_for_empty_lock(&session_arc).await;
        {
            let mut session = session_arc.lock().await;
            assert_eq!(
                session.enqueue_accepted_command(CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: "ide-result".to_string(),
                    priority: false,
                    command: ChatCommand::IdeToolResult {
                        tool_call_id: "ide-tool".to_string(),
                        content: "accepted".to_string(),
                        tool_failed: false,
                    },
                }),
                EnqueueCommandOutcome::Accepted
            );
        }
        wait_for_queue_to_drain(&session_arc).await;
        assert!(session_arc
            .lock()
            .await
            .messages
            .iter()
            .any(|message| { message.role == "tool" && message.tool_call_id == "ide-tool" }));
        close_processor(&session_arc, handle).await;
    }

    #[tokio::test]
    async fn queue_notify_rapid_enqueues_leave_no_stranded_commands() {
        const COMMAND_COUNT: usize = 10_000;

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "queue-notify-rapid".to_string(),
        )));
        let processor_running = session_arc.lock().await.queue_processor_running.clone();
        {
            let mut session = session_arc.lock().await;
            for index in 0..COMMAND_COUNT {
                session.command_queue.push_back(CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: format!("rapid-{index}"),
                    priority: false,
                    command: ChatCommand::Regenerate {},
                });
            }
            session.user_interrupt_flag.store(true, Ordering::SeqCst);
        }
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));

        wait_for_queue_to_drain(&session_arc).await;
        assert_eq!(session_arc.lock().await.command_queue.len(), 0);
        handle.abort();
        let _ = handle.await;
    }

    #[test]
    fn redundant_regenerates_coalesce_until_next_command_boundary() {
        let mut queue = VecDeque::from([
            CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "regen-1".to_string(),
                priority: true,
                command: ChatCommand::Regenerate {},
            },
            CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "regen-2".to_string(),
                priority: true,
                command: ChatCommand::Regenerate {},
            },
            CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "user-1".to_string(),
                priority: false,
                command: ChatCommand::UserMessage {
                    content: json!("keep ordering"),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                    client_message_id: None,
                },
            },
            CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "regen-3".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            },
        ]);

        let removed = drain_redundant_regenerates(&mut queue, true);

        assert_eq!(removed, vec!["regen-1", "regen-2"]);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].client_request_id, "user-1");
        assert_eq!(queue[1].client_request_id, "regen-3");
    }

    #[test]
    fn mode_switch_diff_bounds_large_tool_delta() {
        let from = test_mode_config("from");
        let mut to = test_mode_config("to");
        to.tools = (0..100).map(|index| format!("tool-{index}")).collect();

        let diff = mode_switch_diff(&from, &to);
        let added = &diff["tools"]["added"];

        assert_eq!(added["count"], json!(100));
        assert_eq!(
            added["names"].as_array().unwrap().len(),
            MODE_SWITCH_TOOL_NAMES_LIMIT
        );
        assert_eq!(added["truncated"], json!(true));
        assert!(added["names"]
            .as_array()
            .unwrap()
            .contains(&json!("tool-0")));
    }

    #[tokio::test]
    async fn mode_switch_diff_uses_resolved_overlay_defaults() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];

        let mut explore = test_mode_config("explore");
        explore.prompt = "explore prompt".to_string();
        explore.thread_defaults = ModeThreadDefaults {
            auto_approve_editing_tools: Some(false),
            auto_approve_dangerous_commands: Some(false),
            ..Default::default()
        };
        let mut agent = test_mode_config("agent");
        agent.prompt = "agent prompt".to_string();
        agent.thread_defaults = ModeThreadDefaults {
            auto_approve_editing_tools: Some(true),
            auto_approve_dangerous_commands: Some(true),
            ..Default::default()
        };
        let mut overlay = test_mode_config("agent-for-test-model");
        overlay.base = Some("agent".to_string());
        overlay.match_models = Some(vec!["test/*".to_string()]);
        overlay.override_config = Some(ModeOverride {
            prompt: Some("overlay prompt".to_string()),
            ..Default::default()
        });

        gcx.project_registry_cache.write().unwrap().insert(
            workspace.path().to_path_buf(),
            ProjectRegistry {
                modes: HashMap::from([
                    ("explore".to_string(), explore),
                    ("agent".to_string(), agent),
                ]),
                mode_overrides: vec![overlay],
                ..Default::default()
            },
        );

        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("mode-switch-overlay".to_string());
        session.thread.mode = "explore".to_string();
        session.thread.model = "test/model".to_string();
        let old_mode = session.thread.mode.clone();
        session.thread.mode = "agent".to_string();

        assert!(
            add_mode_switch_event_and_plan_if_changed(
                app,
                &mut session,
                &old_mode,
                None,
                "chat.session",
            )
            .await
        );

        let payload = &session.messages.last().unwrap().extra["event"]["payload"];
        assert_eq!(payload["diff"]["resolved"], json!(true));
        assert_eq!(
            payload["diff"]["thread_defaults"]["auto_approve_editing_tools"]["to"],
            json!(true)
        );
        assert_eq!(
            payload["diff"]["thread_defaults"]["auto_approve_dangerous_commands"]["to"],
            json!(true)
        );
        assert_eq!(payload["diff"]["system_prompt"]["changed"], json!(true));
    }

    fn sample_worktree(id: &str) -> WorktreeMeta {
        WorktreeMeta {
            id: id.to_string(),
            kind: "chat".to_string(),
            root: std::path::PathBuf::from(format!("/tmp/{id}-worktree")),
            source_workspace_root: std::path::PathBuf::from("/tmp/source"),
            repo_root: std::path::PathBuf::from("/tmp/source"),
            branch: Some(format!("refact/chat/{id}")),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        }
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
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn source() {}\n").unwrap();
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    async fn register_running_process(
        gcx: &crate::global_context::GlobalContext,
        process_id: &str,
        mode: ExecMode,
        chat_id: &str,
    ) -> ExecProcessId {
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(mode, "test command".to_string())
                    .with_process_id(ExecProcessId(process_id.to_string()))
                    .with_owner(ExecOwnerMeta {
                        chat_id: Some(chat_id.to_string()),
                        ..ExecOwnerMeta::default()
                    }),
                DEFAULT_EXEC_OUTPUT_LIMIT_BYTES,
            )
            .await;
        gcx.exec_registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();
        snapshot.meta.process_id
    }

    async fn wait_for_cleanup_notice(
        rx: &mut tokio::sync::broadcast::Receiver<Arc<String>>,
    ) -> ChatMessage {
        tokio::time::timeout(QUEUE_TEST_TIMEOUT, async {
            loop {
                let json = rx.recv().await.unwrap();
                let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
                if let ChatEvent::MessageAdded { message, .. } = envelope.event {
                    if message.role == "event"
                        && message.extra.get("event").is_some_and(|event| {
                            event["subkind"] == json!("system_notice")
                                && event["source"] == json!("chat.session")
                        })
                    {
                        return message;
                    }
                }
            }
        })
        .await
        .unwrap()
    }

    #[test]
    fn test_find_allowed_command_empty_queue() {
        let queue = VecDeque::new();
        assert!(find_allowed_command_while_paused(&queue).is_none());
    }

    #[tokio::test]
    async fn clean_background_processes_command_kills_chat_scoped() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let killed_background = register_running_process(
            &gcx,
            "exec_clean_chat_background",
            ExecMode::Background,
            "chat-clean",
        )
        .await;
        let killed_service = register_running_process(
            &gcx,
            "exec_clean_chat_service",
            ExecMode::Service,
            "chat-clean",
        )
        .await;
        let kept_other_chat = register_running_process(
            &gcx,
            "exec_clean_other_chat_background",
            ExecMode::Background,
            "chat-other",
        )
        .await;
        let kept_foreground = register_running_process(
            &gcx,
            "exec_clean_chat_foreground",
            ExecMode::Foreground,
            "chat-clean",
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new("chat-clean".to_string())));
        let (processor_running, mut rx) = {
            let mut session = session_arc.lock().await;
            let rx = session.subscribe();
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "clean-background".to_string(),
                priority: false,
                command: ChatCommand::CleanBackgroundProcesses {
                    include_services: true,
                },
            });
            session.emit_queue_update();
            (session.queue_processor_running.clone(), rx)
        };
        processor_running.store(true, Ordering::SeqCst);

        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));
        let notice = wait_for_cleanup_notice(&mut rx).await;

        assert_eq!(
            notice.content.content_text_only(),
            "Cleared 2 background processes from this chat"
        );
        let event = notice.extra.get("event").unwrap();
        assert_eq!(event["subkind"], json!("system_notice"));
        assert_eq!(event["source"], json!("chat.session"));
        assert_eq!(event["payload"], json!({ "killed_count": 2 }));
        assert!(gcx.exec_registry.get(&killed_background).await.is_none());
        assert!(gcx.exec_registry.get(&killed_service).await.is_none());
        assert!(gcx.exec_registry.get(&kept_other_chat).await.is_some());
        assert!(gcx.exec_registry.get(&kept_foreground).await.is_some());

        {
            let mut session = session_arc.lock().await;
            session.close_event_channel();
            session.queue_notify.notify_waiters();
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), handle)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn chat_started_pushed_on_first_user_message() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        push_user_activity(
            app.clone(),
            UserAction::ChatStarted {
                chat_id: "chat-1".to_string(),
                first_user_text_preview: "hello from first user message".chars().take(80).collect(),
                ts: Utc::now(),
            },
        )
        .await;

        let user_activity = app.buddy.user_activity.clone();
        let ring = user_activity.lock().await;
        assert!(ring.snapshot().iter().any(|action| matches!(
            action,
            UserAction::ChatStarted { chat_id, first_user_text_preview, .. }
                if chat_id == "chat-1" && first_user_text_preview == "hello from first user message"
        )));
    }

    #[test]
    fn test_find_allowed_command_no_allowed() {
        let mut queue = VecDeque::new();
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("hi"),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
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
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
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
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
        }));
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("another"),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
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
        assert_eq!(thread.mode, "explore");
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
    fn test_apply_setparams_auto_compression_cap_set_null_and_invalid() {
        let mut thread = ThreadParams::default();
        let (changed, _) =
            apply_setparams_patch(&mut thread, &json!({"auto_compression_cap": 4096}));
        assert!(changed);
        assert_eq!(thread.auto_compression_cap, Some(4096));
        assert!(!thread.auto_compression_cap_pending);

        let (changed, _) =
            apply_setparams_patch(&mut thread, &json!({"auto_compression_cap": "invalid"}));
        assert!(!changed);
        assert_eq!(thread.auto_compression_cap, Some(4096));

        let (changed, _) =
            apply_setparams_patch(&mut thread, &json!({"auto_compression_cap": null}));
        assert!(changed);
        assert_eq!(thread.auto_compression_cap, None);
        assert!(thread.auto_compression_cap_pending);
        assert!(thread.resolve_pending_compression_cap(Some(1000)));
        assert_eq!(thread.auto_compression_cap, Some(900));
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
    fn test_apply_setparams_autonomous_no_confirm_persists_in_trajectory_snapshot() {
        let mut thread = ThreadParams::default();
        let (changed, sanitized) =
            apply_setparams_patch(&mut thread, &json!({"autonomous_no_confirm": true}));

        assert!(changed);
        assert!(thread.autonomous_no_confirm);
        assert_eq!(sanitized["autonomous_no_confirm"], json!(true));

        let snapshot =
            refact_chat_history::trajectory_snapshot::TrajectorySnapshot::from_thread_parts(
                "autonomous-no-confirm".to_string(),
                &thread,
                Vec::new(),
                "2026-08-24T00:00:00Z".to_string(),
                1,
            );
        let encoded = serde_json::to_value(snapshot).unwrap();
        let decoded: refact_chat_history::trajectory_snapshot::TrajectorySnapshot =
            serde_json::from_value(encoded).unwrap();
        assert!(decoded.autonomous_no_confirm);

        let (changed, _) =
            apply_setparams_patch(&mut thread, &json!({"autonomous_no_confirm": "invalid"}));
        assert!(!changed);
        assert!(thread.autonomous_no_confirm);
    }

    #[test]
    fn test_apply_setparams_auto_compact_enabled_persists_in_trajectory_snapshot() {
        let mut thread = ThreadParams::default();
        let (changed, sanitized) =
            apply_setparams_patch(&mut thread, &json!({"auto_compact_enabled": true}));

        assert!(changed);
        assert_eq!(thread.auto_compact_enabled, Some(true));
        assert_eq!(sanitized["auto_compact_enabled"], json!(true));

        let snapshot =
            refact_chat_history::trajectory_snapshot::TrajectorySnapshot::from_thread_parts(
                "auto-compact-enabled".to_string(),
                &thread,
                Vec::new(),
                "2026-08-24T00:00:00Z".to_string(),
                1,
            );
        let encoded = serde_json::to_value(snapshot).unwrap();
        let decoded: refact_chat_history::trajectory_snapshot::TrajectorySnapshot =
            serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.auto_compact_enabled, Some(true));

        let (changed, _) =
            apply_setparams_patch(&mut thread, &json!({"auto_compact_enabled": "invalid"}));
        assert!(!changed);
        assert_eq!(thread.auto_compact_enabled, Some(true));

        let (changed, _) =
            apply_setparams_patch(&mut thread, &json!({"auto_compact_enabled": null}));
        assert!(changed);
        assert_eq!(thread.auto_compact_enabled, None);
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
        assert_eq!(thread.mode, "explore");
        assert_eq!(thread.boost_reasoning, Some(true));
    }

    #[test]
    fn test_apply_setparams_mode_canonicalizes_task_agent() {
        let mut thread = ThreadParams::default();
        let patch = json!({"mode": "TASK_AGENT"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert_eq!(thread.mode, "task_agent");
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
    fn trajectory_worktree_apply_setparams_detaches_on_null() {
        let mut thread = ThreadParams::default();
        thread.worktree = Some(crate::worktrees::types::WorktreeMeta {
            id: "wt-1".to_string(),
            kind: "task_agent".to_string(),
            root: std::path::PathBuf::from("/tmp/wt"),
            source_workspace_root: std::path::PathBuf::from("/tmp/src"),
            repo_root: std::path::PathBuf::from("/tmp/src"),
            branch: Some("branch".to_string()),
            base_branch: None,
            base_commit: None,
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        });
        let patch = json!({"worktree": null});
        let (changed, sanitized) = apply_setparams_patch(&mut thread, &patch);
        assert!(changed);
        assert!(thread.worktree.is_none());
        assert!(sanitized.get("worktree").unwrap().is_null());
    }

    #[test]
    fn trajectory_worktree_apply_setparams_ignores_attach_object() {
        let mut thread = ThreadParams::default();
        let patch = json!({"worktree": {"root": "/tmp/untrusted"}});
        let (changed, sanitized) = apply_setparams_patch(&mut thread, &patch);
        assert!(!changed);
        assert!(thread.worktree.is_none());
        assert!(sanitized.get("worktree").is_none());
    }

    #[test]
    fn worktree_scope_messages_announce_attach_and_detach() {
        let worktree = sample_worktree("wt-msg");

        let enabled = worktree_activation_message(&worktree);
        let disabled = worktree_disabled_message(Some(&worktree));

        assert_eq!(enabled.role, "cd_instruction");
        assert_eq!(enabled.tool_call_id, "worktree_enabled");
        assert!(enabled
            .content
            .content_text_only()
            .contains("WORKTREE_ENABLED"));
        assert!(enabled
            .content
            .content_text_only()
            .contains("Worktree root"));
        assert_eq!(disabled.role, "cd_instruction");
        assert_eq!(disabled.tool_call_id, "worktree_disabled");
        assert!(disabled
            .content
            .content_text_only()
            .contains("WORKTREE_DISABLED"));
        assert!(disabled
            .content
            .content_text_only()
            .contains("main workspace"));
        assert!(disabled.content.content_text_only().contains("wt-msg"));
    }

    #[tokio::test]
    async fn worktree_setparams_attach_by_id_resolves_registry_and_scopes_create() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
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
        let app = AppState::from_gcx(gcx.clone()).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/attach-id".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut thread = ThreadParams::default();
        thread.id = "chat-attach".to_string();
        let update = resolve_worktree_setparams_update(
            app,
            "chat-attach",
            &thread,
            &json!({"worktree_id": created.worktree.meta.id.clone()}),
        )
        .await
        .unwrap()
        .unwrap();
        thread.worktree = update.worktree;
        let scope = crate::worktrees::scope::ExecutionScope::from_thread(&thread).unwrap();
        let resolved = scope
            .resolve_creatable_path(Path::new("nested/file.rs"))
            .unwrap();
        assert!(resolved.path.starts_with(&created.worktree.meta.root));
        assert!(!resolved.path.starts_with(&source));
        let registry = service.load_registry().await.unwrap();
        assert_eq!(registry.records[0].references.len(), 1);
        assert_eq!(
            registry.records[0].references[0].chat_id.as_deref(),
            Some("chat-attach")
        );
    }

    #[tokio::test]
    async fn worktree_setparams_same_id_skips_registry_lookup() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut thread = ThreadParams::default();
        thread.worktree = Some(sample_worktree("wt-existing"));

        let update = resolve_worktree_setparams_update(
            app,
            "chat-existing",
            &thread,
            &json!({"worktree_id": "wt-existing"}),
        )
        .await
        .unwrap();

        assert!(update.is_none());
    }

    #[tokio::test]
    async fn worktree_setparams_detach_clears_registry_reference() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
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
        let app = AppState::from_gcx(gcx.clone()).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/detach-id".to_string()),
                chat_id: Some("chat-detach".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut thread = ThreadParams::default();
        thread.worktree = Some(created.worktree.meta.clone());
        let update = resolve_worktree_setparams_update(
            app,
            "chat-detach",
            &thread,
            &json!({"worktree": null}),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(update.worktree.is_none());
        let registry = service.load_registry().await.unwrap();
        assert!(registry.records[0].references.is_empty());
    }

    #[tokio::test]
    async fn worktree_branch_reference_preserves_scope_for_new_chat() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
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
        let app = AppState::from_gcx(gcx.clone()).await;
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/branch-preserve".to_string()),
                chat_id: Some("source-chat".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        let mut target = ThreadParams::default();
        target.id = "target-chat".to_string();
        let validated =
            add_thread_worktree_reference(app, "target-chat", &target, &created.worktree.meta)
                .await
                .unwrap();

        assert_eq!(validated.id, created.worktree.meta.id);
        let registry = service.load_registry().await.unwrap();
        let record = &registry.records[0];
        assert!(record.references.iter().any(|reference| {
            reference.kind == "chat" && reference.chat_id.as_deref() == Some("source-chat")
        }));
        assert!(record.references.iter().any(|reference| {
            reference.kind == "chat" && reference.chat_id.as_deref() == Some("target-chat")
        }));
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
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
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
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
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
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
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
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("first"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-2".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("second"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        let priority_req = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-priority".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
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
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-p1".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("p1"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("normal"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        let priority_req = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("p2"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
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
            native_approval_guard: None,
            supervision_guard: None,
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
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-p1".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        let priority_req = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
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
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-p1".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority 1"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("normal"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority 2"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
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

    #[tokio::test]
    async fn priority_user_message_resets_goal_no_progress() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new("priority-reset".to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.checkpoints_enabled = false;
            session.install_goal("agent", "ship it", true, GoalBudget::default());
            session.goal.as_mut().unwrap().progress.no_progress_turns = 3;
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "priority-user".to_string(),
                priority: true,
                command: ChatCommand::UserMessage {
                    content: json!("resume pursuit"),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                    client_message_id: None,
                },
            });
        }

        assert!(inject_priority_messages_if_any(app, session_arc.clone()).await);

        let session = session_arc.lock().await;
        assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);
        assert!(session.messages.iter().any(|message| {
            message.role == "user"
                && message
                    .content
                    .content_text_only()
                    .contains("resume pursuit")
        }));
    }

    #[tokio::test]
    async fn priority_user_message_preserves_client_message_id_and_server_message_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "priority-client-message-id".to_string(),
        )));
        {
            let mut session = session_arc.lock().await;
            session.thread.checkpoints_enabled = false;
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "priority-user".to_string(),
                priority: true,
                command: user_message_command("same content", Some("optimistic-priority")),
            });
        }

        assert!(inject_priority_messages_if_any(app, session_arc.clone()).await);

        let session = session_arc.lock().await;
        let message = session
            .messages
            .iter()
            .find(|message| message.role == "user")
            .expect("priority user message");
        assert_eq!(
            message.extra.get(CLIENT_MESSAGE_ID_EXTRA_KEY),
            Some(&json!("optimistic-priority"))
        );
        assert_ne!(message.message_id, "optimistic-priority");
        assert!(uuid::Uuid::parse_str(&message.message_id).is_ok());
    }

    #[tokio::test]
    async fn ordinary_batched_user_messages_preserve_distinct_client_message_ids() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "ordinary-client-message-ids".to_string(),
        )));
        {
            let mut session = session_arc.lock().await;
            session.thread.checkpoints_enabled = false;
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "ordinary-first".to_string(),
                priority: false,
                command: user_message_command("same content", Some("optimistic-1")),
            });
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "ordinary-second".to_string(),
                priority: false,
                command: user_message_command("same content", Some("optimistic-2")),
            });
        }
        let processor_running = session_arc.lock().await.queue_processor_running.clone();
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));

        tokio::time::timeout(QUEUE_TEST_TIMEOUT, async {
            loop {
                if session_arc
                    .lock()
                    .await
                    .messages
                    .iter()
                    .filter(|message| message.role == "user")
                    .count()
                    == 2
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("ordinary user messages were not added");

        {
            let session = session_arc.lock().await;
            let messages = session
                .messages
                .iter()
                .filter(|message| message.role == "user")
                .collect::<Vec<_>>();
            assert_eq!(messages.len(), 2);
            assert_eq!(
                messages[0].extra.get(CLIENT_MESSAGE_ID_EXTRA_KEY),
                Some(&json!("optimistic-1"))
            );
            assert_eq!(
                messages[1].extra.get(CLIENT_MESSAGE_ID_EXTRA_KEY),
                Some(&json!("optimistic-2"))
            );
            assert_ne!(messages[0].message_id, messages[1].message_id);
            assert!(uuid::Uuid::parse_str(&messages[0].message_id).is_ok());
            assert!(uuid::Uuid::parse_str(&messages[1].message_id).is_ok());
        }
        close_processor(&session_arc, handle).await;
    }

    #[tokio::test]
    async fn priority_user_message_suppresses_auto_enrichment_without_context_files() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "priority-suppress-enrichment".to_string(),
        )));
        {
            let mut session = session_arc.lock().await;
            session.thread.checkpoints_enabled = false;
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "priority-user".to_string(),
                priority: true,
                command: ChatCommand::UserMessage {
                    content: json!("skip enrichment"),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: true,
                    client_message_id: None,
                },
            });
        }

        assert!(inject_priority_messages_if_any(app, session_arc.clone()).await);

        let session = session_arc.lock().await;
        assert!(session.suppress_auto_enrichment_for_next_turn);
        assert!(session.messages.iter().any(|message| {
            message.role == "user"
                && message
                    .content
                    .content_text_only()
                    .contains("skip enrichment")
        }));
    }

    #[tokio::test]
    async fn promoted_multimodal_message_reaches_priority_injection_unchanged() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(
            "priority-multimodal".to_string(),
        )));
        let image_data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIW2Nk+M/wHwAF/gL+3MxZ5wAAAABJRU5ErkJggg==";
        {
            let mut session = session_arc.lock().await;
            session.thread.checkpoints_enabled = false;
            session.command_queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
                client_request_id: "multimodal-user".to_string(),
                priority: false,
                command: ChatCommand::UserMessage {
                    content: json!([
                        {"type": "text", "text": "describe this"},
                        {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{image_data}")}}
                    ]),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                    client_message_id: None,
                },
            });
            assert!(session.reprioritize_queued_command("multimodal-user", true));
        }

        assert!(inject_priority_messages_if_any(app, session_arc.clone()).await);

        let session = session_arc.lock().await;
        let user_message = session
            .messages
            .iter()
            .find(|message| message.role == "user")
            .expect("promoted user message");
        let ChatContent::Multimodal(elements) = &user_message.content else {
            panic!("expected multimodal user message");
        };
        assert_eq!(elements[0].m_type, "text");
        assert_eq!(elements[0].m_content, "describe this");
        assert_eq!(elements[1].m_type, "image/png");
        assert_eq!(elements[1].m_content, image_data);
    }

    #[test]
    fn test_drain_non_priority_user_messages_extracts_all_non_priority() {
        let mut queue = VecDeque::new();
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("first"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-p".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-2".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("second"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
            },
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-3".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("third"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
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
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-abort".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
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

    #[test]
    fn test_model_switch_clears_previous_response_id() {
        let mut thread = ThreadParams::default();
        thread.model = "openai/gpt-4".into();
        thread.previous_response_id = Some("resp_abc123".to_string());

        let patch = json!({"model": "anthropic/claude-3"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);

        assert!(changed);
        assert_eq!(thread.model, "anthropic/claude-3");
        assert_eq!(
            thread.previous_response_id, None,
            "previous_response_id must be cleared on model switch"
        );
    }

    #[test]
    fn test_same_model_preserves_previous_response_id() {
        let mut thread = ThreadParams::default();
        thread.model = "openai/gpt-4".into();
        thread.previous_response_id = Some("resp_abc123".to_string());

        let patch = json!({"model": "openai/gpt-4"});
        let (changed, _) = apply_setparams_patch(&mut thread, &patch);

        assert!(!changed);
        assert_eq!(
            thread.previous_response_id,
            Some("resp_abc123".to_string()),
            "previous_response_id should be preserved when model doesn't change"
        );
    }

    #[test]
    fn test_skill_activation_sets_context_marker() {
        assert_eq!(
            SKILLS_CONTEXT_MARKER, "skills_context",
            "SKILLS_CONTEXT_MARKER must equal 'skills_context' so prompts.rs detects existing skills context"
        );
    }

    fn make_plan_message(content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "plan".to_string(),
            json!({
                "mode": "agent",
                "version": 1,
                "created_at_ms": 123,
                "supersedes": null,
            }),
        );
        ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "plan".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            preserve: Some(true),
            extra,
            ..Default::default()
        }
    }

    fn make_plan_delta_event(content: &str, seq: u32) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "event".to_string(),
            json!({
                "subkind": "plan_delta",
                "source": "tool.set_plan",
                "payload": {"seq": seq},
            }),
        );
        ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "event".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn make_goal_message(content: &str) -> ChatMessage {
        crate::chat::internal_roles::goal("agent", 1, content, None, true, GoalBudget::default())
    }

    fn make_goal_event(content: &str, subkind: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "event".to_string(),
            json!({
                "subkind": subkind,
                "source": "goal.test",
                "payload": {"seq": 1},
            }),
        );
        ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "event".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn drain_events(rx: &mut tokio::sync::broadcast::Receiver<Arc<String>>) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        while let Ok(json) = rx.try_recv() {
            let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
            events.push(envelope.event);
        }
        events
    }

    fn answer_question_set(tool_call_id: &str) -> PendingQuestionSet {
        PendingQuestionSet {
            tool_call_id: tool_call_id.to_string(),
            questions: vec![PendingQuestion {
                id: "proceed".into(),
                question_type: "yes_no".into(),
                text: "Proceed?".into(),
                options: None,
            }],
            asked_at_ms: 1,
            status: PendingQuestionStatus::Pending,
            answers: Vec::new(),
            resolution_id: None,
        }
    }

    #[test]
    fn answer_questions_generation_trigger_releases_queue_on_invalid_answer() {
        let mut session = ChatSession::new("answer-release".to_string());
        session
            .register_pending_questions(answer_question_set("ask-release"))
            .unwrap();
        session.set_runtime_state(SessionState::Generating, None);
        let mut rx = session.subscribe();

        reject_question_answers(
            &mut session,
            "req-invalid",
            "invalid answer for question id: proceed".to_string(),
        );

        assert_eq!(session.runtime.state, SessionState::WaitingUserInput);
        assert!(session.runtime.error.is_none());
        assert_eq!(
            session.pending_questions[0].status,
            PendingQuestionStatus::Pending
        );
        let events = drain_events(&mut rx);
        assert!(events.iter().any(|event| matches!(
            event,
            ChatEvent::Ack {
                client_request_id,
                accepted: false,
                result: Some(result),
            } if client_request_id == "req-invalid"
                && result["error"].as_str().is_some_and(|error| error.contains("invalid answer"))
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ChatEvent::RuntimeUpdated {
                state: SessionState::WaitingUserInput,
                ..
            }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            ChatEvent::RuntimeUpdated {
                state: SessionState::Generating,
                ..
            }
        )));
        assert!(!matches!(session.runtime.state, SessionState::Generating));
    }

    #[test]
    fn answer_questions_save_failure_rolls_back_and_emits_snapshot_with_pending_set() {
        let mut session = ChatSession::new("answer-save-failure".to_string());
        session
            .register_pending_questions(answer_question_set("ask-save"))
            .unwrap();
        session
            .answer_pending_questions(
                "ask-save",
                vec![QuestionAnswer {
                    id: "proceed".into(),
                    answer: serde_json::json!(true),
                }],
                "req-save".into(),
            )
            .unwrap();
        let staged = user_message_with_client_message_id(
            ChatContent::SimpleText("answer".into()),
            Vec::new(),
            Some("req-save".to_string()),
        );
        let staged_id = staged.message_id.clone();
        session.add_message(staged);
        session.set_runtime_state(SessionState::Generating, None);
        let mut rx = session.subscribe();

        let rolled_back =
            session.rollback_answered_questions("ask-save", "req-save", Some(&staged_id));
        assert!(rolled_back);
        release_answer_questions_failure(
            &mut session,
            "req-save",
            "failed to persist question answers: disk full".to_string(),
            rolled_back,
        );

        assert_eq!(
            session.pending_questions[0].status,
            PendingQuestionStatus::Pending
        );
        assert!(session.messages.is_empty());
        assert_eq!(session.runtime.state, SessionState::Error);
        let events = drain_events(&mut rx);
        assert!(events.iter().any(|event| matches!(
            event,
            ChatEvent::Ack {
                client_request_id,
                accepted: false,
                ..
            } if client_request_id == "req-save"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ChatEvent::Snapshot {
                pending_questions,
                messages,
                ..
            } if messages.is_empty()
                && pending_questions
                    .iter()
                    .all(|set| set.status == PendingQuestionStatus::Pending)
        )));
    }

    #[test]
    fn answer_questions_conflict_preserves_new_pending_question() {
        let mut session = ChatSession::new("answer-conflict".to_string());
        session
            .register_pending_questions(answer_question_set("old"))
            .unwrap();
        session
            .answer_pending_questions(
                "old",
                vec![QuestionAnswer {
                    id: "proceed".into(),
                    answer: json!(true),
                }],
                "first-answer".into(),
            )
            .unwrap();
        session
            .register_pending_questions(answer_question_set("current"))
            .unwrap();
        session.set_runtime_state(SessionState::Generating, None);
        let mut rx = session.subscribe();
        let before = session.pending_questions.clone();
        let error = session
            .answer_pending_questions(
                "old",
                vec![QuestionAnswer {
                    id: "proceed".into(),
                    answer: json!(false),
                }],
                "second-answer".into(),
            )
            .unwrap_err();

        reject_question_answers(&mut session, "second-answer", error);

        assert_eq!(session.pending_questions, before);
        assert_eq!(session.runtime.state, SessionState::WaitingUserInput);
        assert!(session.runtime.error.is_none());
        let events = drain_events(&mut rx);
        assert!(events.iter().any(|event| matches!(event,
            ChatEvent::Ack { client_request_id, accepted: false, .. }
                if client_request_id == "second-answer"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            ChatEvent::RuntimeUpdated {
                state: SessionState::Error,
                ..
            }
        )));
    }

    #[test]
    fn answer_questions_rejection_preserves_paused_runtime() {
        let mut session = ChatSession::new("answer-paused-rejection".to_string());
        session.set_runtime_state(SessionState::Paused, None);
        let mut rx = session.subscribe();

        reject_question_answers(&mut session, "stale", "unknown question".into());

        assert_eq!(session.runtime.state, SessionState::Paused);
        assert!(session.runtime.error.is_none());
        assert!(!drain_events(&mut rx).iter().any(|event| matches!(
            event,
            ChatEvent::RuntimeUpdated {
                state: SessionState::Error,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn abort_command_cancels_pending_browser_message() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new("browser-abort".into())));
        let processor_running = {
            let mut session = session_arc.lock().await;
            session.pending_browser_message = Some(PendingBrowserMessage {
                pending_message_id: "pending-browser".into(),
                content: json!("message waiting for browser context"),
                attachments: vec![],
                client_message_id: None,
                checkpoints: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                skill_activation_name: None,
                skill_context_msg: None,
            });
            session.set_runtime_state(SessionState::WaitingUserInput, None);
            session
                .command_queue
                .push_back(abort_request("browser-stop"));
            session.queue_processor_running.clone()
        };
        processor_running.store(true, Ordering::SeqCst);
        let handle = tokio::spawn(process_command_queue(
            app,
            session_arc.clone(),
            processor_running,
        ));
        tokio::time::timeout(QUEUE_TEST_TIMEOUT, async {
            loop {
                if session_arc.lock().await.pending_browser_message.is_none() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("abort did not cancel pending browser message");
        assert_eq!(session_arc.lock().await.runtime.state, SessionState::Idle);
        close_processor(&session_arc, handle).await;
    }

    #[test]
    fn answer_questions_retry_after_rollback_adds_single_message_and_answer() {
        let mut session = ChatSession::new("answer-retry".to_string());
        session
            .register_pending_questions(answer_question_set("ask-retry"))
            .unwrap();
        let answers = vec![QuestionAnswer {
            id: "proceed".into(),
            answer: serde_json::json!(true),
        }];
        session
            .answer_pending_questions("ask-retry", answers.clone(), "req-1".into())
            .unwrap();
        let staged = user_message_with_client_message_id(
            ChatContent::SimpleText(answer_questions_content("ask-retry", &answers)),
            Vec::new(),
            Some("req-1".to_string()),
        );
        let staged_id = staged.message_id.clone();
        session.add_message(staged);
        assert!(session.rollback_answered_questions("ask-retry", "req-1", Some(&staged_id)));

        assert!(session
            .answer_pending_questions("ask-retry", answers.clone(), "req-2".into())
            .unwrap());
        session.add_message(user_message_with_client_message_id(
            ChatContent::SimpleText(answer_questions_content("ask-retry", &answers)),
            Vec::new(),
            Some("req-2".to_string()),
        ));

        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.pending_questions[0].resolution_id.as_deref(),
            Some("req-2")
        );
        assert!(!session
            .answer_pending_questions("ask-retry", answers, "req-3".into())
            .unwrap());
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn answer_questions_failure_release_when_not_generating_reports_error_only() {
        let mut session = ChatSession::new("answer-paused".to_string());
        session
            .register_pending_questions(answer_question_set("ask-paused"))
            .unwrap();

        release_answer_questions_failure(
            &mut session,
            "req-paused",
            "unknown question tool_call_id".to_string(),
            false,
        );

        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(
            session.pending_questions[0].status,
            PendingQuestionStatus::Pending
        );
    }

    #[test]
    fn stale_question_instance_rejects_before_answering_or_changing_runtime() {
        let mut session = ChatSession::new("instance-guard".into());
        let mut set = answer_question_set("ask-instance");
        set.asked_at_ms = 20;
        session.register_pending_questions(set).unwrap();
        let before = session.pending_questions.clone();
        let answer = vec![QuestionAnswer {
            id: "proceed".into(),
            answer: json!(true),
        }];
        assert!(session
            .answer_pending_questions_at("ask-instance", answer.clone(), "stale".into(), Some(19))
            .is_err());
        assert_eq!(session.pending_questions, before);
        assert!(session.messages.is_empty());
        assert!(session
            .answer_pending_questions_at("ask-instance", answer, "current".into(), Some(20))
            .unwrap());
    }

    #[test]
    fn answer_questions_waits_for_required_save_before_generating() {
        assert!(!command_triggers_generation(
            &ChatCommand::AnswerQuestions {
                tool_call_id: "ask".into(),
                answers: Vec::new(),
                asked_at_ms: None,
            }
        ));
    }

    #[test]
    fn goal_budget_set_goal_command_installs_snapshot_and_runtime() {
        let mut session = ChatSession::new("goal-command".to_string());
        let mut rx = session.subscribe();

        let result =
            handle_set_goal_command(&mut session, "Ship the pond".to_string(), None, None).unwrap();

        assert_eq!(result, json!({"version": 1, "supersedes": null}));
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.content, "Ship the pond");
        assert!(goal.active);
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.budget, GoalBudget::default());
        assert!(session.messages[0].extra["goal"]["budget"]
            .get("max_turns")
            .is_none());
        assert_eq!(goal.progress.turns_used, 0);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert!(drain_events(&mut rx).into_iter().any(|event| matches!(
            event,
            ChatEvent::RuntimeUpdated {
                goal_active: true,
                goal_status: Some(GoalStatus::Active),
                ..
            }
        )));
    }

    #[test]
    fn goal_budget_set_goal_command_installs_explicit_budget() {
        let mut session = ChatSession::new("goal-command".to_string());
        let budget = GoalBudget {
            max_turns: Some(3),
            max_minutes: Some(4),
            max_tokens: Some(5),
            max_cost_cents: None,
            cooldown_ms: 1_500,
            no_progress_token_threshold: 50,
            no_progress_turns: Some(6),
            explicit: false,
        };

        handle_set_goal_command(
            &mut session,
            "Ship the pond".to_string(),
            Some(budget.clone()),
            None,
        )
        .unwrap();

        let expected = GoalBudget {
            explicit: true,
            ..budget
        };
        assert_eq!(session.goal.as_ref().unwrap().budget, expected);
        assert_eq!(
            session.messages[0].extra["goal"]["budget"]["max_turns"],
            json!(3)
        );
        assert_eq!(
            session.messages[0].extra["goal"]["budget"]["explicit"],
            json!(true)
        );
    }

    #[test]
    fn goal_budget_set_goal_command_rejects_when_exists() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "First".to_string(), None, None).unwrap();

        let error =
            handle_set_goal_command(&mut session, "Second".to_string(), None, None).unwrap_err();

        assert_eq!(error, "goal already exists; use update_goal");
        assert_eq!(session.goal.as_ref().unwrap().content, "First");
    }

    #[test]
    fn goal_budget_set_goal_budget_command_replaces_budget_and_projection() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();
        let budget = GoalBudget {
            max_turns: Some(2),
            max_minutes: None,
            max_tokens: Some(1_000),
            max_cost_cents: None,
            cooldown_ms: 2_000,
            no_progress_token_threshold: 25,
            no_progress_turns: None,
            explicit: false,
        };

        let result = handle_set_goal_budget_command(&mut session, budget.clone()).unwrap();

        let expected = GoalBudget {
            explicit: true,
            ..budget
        };
        assert_eq!(result, json!({"budget": expected.clone()}));
        assert_eq!(session.goal.as_ref().unwrap().budget, expected);
        assert_eq!(session.messages[0].extra["goal"]["budget"], json!(expected));
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
    }

    #[test]
    fn goal_budget_set_goal_budget_legacy_quad_is_stamped_explicit() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();

        handle_set_goal_budget_command(&mut session, GoalBudget::legacy_default_hard_limits())
            .unwrap();

        let goal_meta = session.messages[0].extra.get_mut("goal").unwrap();
        assert_eq!(goal_meta["budget"]["explicit"], json!(true));
        goal_meta["status"] = json!(GoalStatus::BudgetExhausted);
        goal_meta["progress"] = json!(GoalProgress {
            turns_used: 10,
            ..Default::default()
        });

        let reloaded = ChatSession::new_with_trajectory(
            "goal-command".to_string(),
            session.messages.clone(),
            ThreadParams::default(),
            "2024-01-01T00:00:00Z".to_string(),
            None,
            Vec::new(),
            None,
        );

        let goal = reloaded.goal.expect("goal projection");
        assert_eq!(goal.budget.max_turns, Some(10));
        assert_eq!(goal.budget.max_minutes, Some(15));
        assert_eq!(goal.budget.max_tokens, Some(200_000));
        assert_eq!(goal.budget.no_progress_turns, Some(2));
        assert!(goal.budget.explicit);
        assert_eq!(goal.status, GoalStatus::BudgetExhausted);
    }

    #[test]
    fn goal_budget_set_goal_budget_command_exhausts_when_already_over_limit() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();
        session.goal.as_mut().unwrap().progress.turns_used = 3;
        session.refresh_goal_runtime_mirror();
        let epoch = session.goal_ledger_last_seq();

        handle_set_goal_budget_command(
            &mut session,
            GoalBudget {
                max_turns: Some(2),
                max_minutes: None,
                max_tokens: None,
                max_cost_cents: None,
                cooldown_ms: 1_500,
                no_progress_token_threshold: 50,
                no_progress_turns: None,
                explicit: false,
            },
        )
        .unwrap();

        assert_eq!(
            session.goal.as_ref().unwrap().status,
            GoalStatus::BudgetExhausted
        );
        assert_eq!(session.goal_status, Some(GoalStatus::BudgetExhausted));
        assert!(session.goal_status_changed_since(epoch));
        assert!(matches!(
            session.goal_ledger.last().map(|entry| &entry.op),
            Some(GoalLedgerOp::StatusChanged {
                to: GoalStatus::BudgetExhausted,
                ..
            })
        ));
    }

    #[test]
    fn goal_budget_set_goal_budget_command_clears_limits_and_heals_terminal_status() {
        let mut session = ChatSession::new("goal-command".to_string());
        let budget = GoalBudget {
            max_turns: Some(1),
            max_minutes: None,
            max_tokens: None,
            max_cost_cents: None,
            cooldown_ms: 1_500,
            no_progress_token_threshold: 50,
            no_progress_turns: None,
            explicit: false,
        };
        handle_set_goal_command(&mut session, "Base".to_string(), Some(budget), None).unwrap();
        session.goal.as_mut().unwrap().progress.turns_used = 1;
        session.goal_set_status(GoalStatus::BudgetExhausted);
        session.messages[0].extra["goal"]["progress"] =
            json!(session.goal.as_ref().unwrap().progress);
        session.messages[0].extra["goal"]["status"] = json!(GoalStatus::BudgetExhausted);

        handle_set_goal_budget_command(&mut session, GoalBudget::default()).unwrap();

        let goal = session.goal.as_ref().unwrap();
        let expected_budget = GoalBudget {
            explicit: true,
            ..GoalBudget::default()
        };
        assert_eq!(goal.budget, expected_budget);
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert!(session.messages[0].extra["goal"]["budget"]
            .get("max_turns")
            .is_none());
        assert_eq!(
            session.messages[0].extra["goal"]["budget"]["explicit"],
            json!(true)
        );
    }

    #[test]
    fn goal_budget_set_goal_budget_command_rejects_without_goal() {
        let mut session = ChatSession::new("goal-command".to_string());

        let error =
            handle_set_goal_budget_command(&mut session, GoalBudget::default()).unwrap_err();

        assert_eq!(error, "no goal to set budget for; call set_goal first");
        assert!(session.goal.is_none());
    }

    #[test]
    fn goal_budget_update_goal_command_appends_delta_and_emits_runtime() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();
        let mut rx = session.subscribe();

        let result = handle_update_goal_command(&mut session, "Add tests".to_string()).unwrap();

        assert_eq!(result, json!({"seq": 1, "truncated": false}));
        assert_eq!(crate::chat::goal_role::goal_delta_events(&session).len(), 1);
        assert_eq!(
            session.goal.as_ref().unwrap().content,
            "Base\n\n---\n\n## Goal updates\n\nAdd tests"
        );
        assert!(drain_events(&mut rx).into_iter().any(|event| matches!(
            event,
            ChatEvent::RuntimeUpdated {
                goal_active: true,
                goal_status: Some(GoalStatus::Active),
                ..
            }
        )));
    }

    #[test]
    fn goal_budget_goal_control_command_transitions_status() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();

        let paused = handle_goal_control_command(&mut session, "pause".to_string()).unwrap();
        assert_eq!(paused, json!({"action": "pause", "status": "paused"}));
        assert_eq!(session.goal_status, Some(GoalStatus::Paused));
        assert!(!session.goal_can_pursue());

        let resumed = handle_goal_control_command(&mut session, "resume".to_string()).unwrap();
        assert_eq!(resumed, json!({"action": "resume", "status": "active"}));
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert!(session.goal_can_pursue());

        let stopped = handle_goal_control_command(&mut session, "stop".to_string()).unwrap();
        assert_eq!(stopped, json!({"action": "stop", "status": "stopped"}));
        assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
        assert!(!session.goal_can_pursue());
    }

    #[test]
    fn explicit_goal_control_stop_is_not_reactivated_by_next_user_turn() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();

        handle_goal_control_command(&mut session, "stop".to_string()).unwrap();
        assert_eq!(session.goal_status, Some(GoalStatus::Stopped));

        note_user_turn_resets_goal_pursuit(&mut session);

        assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
    }

    #[test]
    fn goal_control_resume_requires_paused_or_stopped() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();

        let error = handle_goal_control_command(&mut session, "resume".to_string()).unwrap_err();
        assert!(error.contains("paused or stopped"));
        assert_eq!(session.goal_status, Some(GoalStatus::Active));

        session.goal_set_status(GoalStatus::Completed);
        assert!(handle_goal_control_command(&mut session, "resume".to_string()).is_err());
        assert_eq!(session.goal_status, Some(GoalStatus::Completed));

        session.goal_set_status(GoalStatus::Stopped);
        handle_goal_control_command(&mut session, "resume".to_string()).unwrap();
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
    }

    #[test]
    fn goal_control_stop_purges_goal_generated_regenerates_and_records_event() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();
        session.command_queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "goal-nudge-1".to_string(),
            priority: true,
            command: ChatCommand::Regenerate {},
        });
        session.command_queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "goal-verifier-regenerate-1".to_string(),
            priority: true,
            command: ChatCommand::Regenerate {},
        });
        session.command_queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "user-regenerate".to_string(),
            priority: false,
            command: ChatCommand::Regenerate {},
        });
        let now = std::time::Instant::now();
        session
            .command_enqueued_at
            .insert("goal-nudge-1".to_string(), now);
        session
            .command_enqueued_at
            .insert("goal-verifier-regenerate-1".to_string(), now);
        session
            .command_enqueued_at
            .insert("user-regenerate".to_string(), now);

        handle_goal_control_command(&mut session, "stop".to_string()).unwrap();

        assert_eq!(session.command_queue.len(), 1);
        assert_eq!(
            session.command_queue.front().unwrap().client_request_id,
            "user-regenerate"
        );
        assert!(!session.command_enqueued_at.contains_key("goal-nudge-1"));
        assert!(!session
            .command_enqueued_at
            .contains_key("goal-verifier-regenerate-1"));
        assert!(session.command_enqueued_at.contains_key("user-regenerate"));
        assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
        let stop_events = session
            .messages
            .iter()
            .filter(|message| {
                message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("payload"))
                    .and_then(|payload| payload.get("trigger"))
                    .and_then(|trigger| trigger.as_str())
                    == Some("goal_control")
            })
            .count();
        assert_eq!(stop_events, 1);
    }

    #[test]
    fn goal_control_stop_survives_projection_rebuild() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();

        handle_goal_control_command(&mut session, "stop".to_string()).unwrap();
        session.rebuild_goal_projection_from_messages();

        assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
        assert!(!session.goal_can_pursue());
    }

    #[test]
    fn manual_abort_stop_is_reactivated_by_next_user_turn() {
        let mut session = ChatSession::new("goal-command".to_string());
        handle_set_goal_command(&mut session, "Base".to_string(), None, None).unwrap();

        assert!(session.stop_goal_on_manual_abort());
        assert_eq!(session.goal_status, Some(GoalStatus::Stopped));

        note_user_turn_resets_goal_pursuit(&mut session);

        assert_eq!(session.goal_status, Some(GoalStatus::Active));
    }

    #[test]
    fn restore_keeps_plan_role() {
        let plan = make_plan_message("base plan");
        let sanitized = sanitize_message_for_restore(&plan);

        assert!(is_allowed_for_restore(&plan));
        assert_eq!(sanitized.role, "plan");
        assert_eq!(sanitized.content.content_text_only(), "base plan");
        assert_eq!(sanitized.preserve, Some(true));
        assert_eq!(sanitized.extra["plan"]["version"], json!(1));
    }

    #[test]
    fn restore_keeps_goal_and_goal_events() {
        let goal = make_goal_message("ship the frog");
        let delta = make_goal_event("tighten acceptance", "goal_delta");
        let pursuit = make_goal_event("verifying", "goal_pursuit");

        for message in [&goal, &delta, &pursuit] {
            assert!(is_allowed_for_restore(message));
        }
        let restored_goal = sanitize_message_for_restore(&goal);
        let restored_delta = sanitize_message_for_restore(&delta);
        let restored_pursuit = sanitize_message_for_restore(&pursuit);

        assert_eq!(restored_goal.role, "goal");
        assert_eq!(restored_goal.extra["goal"]["version"], json!(1));
        assert_eq!(
            restored_delta.extra["event"]["subkind"],
            json!("goal_delta")
        );
        assert_eq!(
            restored_pursuit.extra["event"]["subkind"],
            json!("goal_pursuit")
        );
    }

    #[test]
    fn branch_keeps_plan_and_plan_delta() {
        let plan = make_plan_message("base plan");
        let delta = make_plan_delta_event("update", 2);
        let other_event = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "test",
            json!({}),
            "skip",
        );

        assert!(is_allowed_for_branch(&plan));
        assert!(is_allowed_for_branch(&delta));
        assert!(!is_allowed_for_branch(&other_event));
        let branched_plan = sanitize_message_for_branch(&plan);
        let branched_delta = sanitize_message_for_branch(&delta);

        assert_eq!(branched_plan.role, "plan");
        assert_eq!(branched_plan.extra["plan"]["version"], json!(1));
        assert_eq!(branched_delta.role, "event");
        assert_eq!(
            branched_delta.extra["event"]["subkind"],
            json!("plan_delta")
        );
        assert_eq!(branched_delta.extra["event"]["payload"]["seq"], json!(2));
    }

    #[test]
    fn branch_keeps_goal_and_goal_events() {
        let goal = make_goal_message("base goal");
        let delta = make_goal_event("update", "goal_delta");
        let pursuit = make_goal_event("verifying", "goal_pursuit");
        let other_event = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "test",
            json!({}),
            "skip",
        );

        assert!(is_allowed_for_branch(&goal));
        assert!(is_allowed_for_branch(&delta));
        assert!(is_allowed_for_branch(&pursuit));
        assert!(!is_allowed_for_branch(&other_event));
        let branched_goal = sanitize_message_for_branch(&goal);
        let branched_delta = sanitize_message_for_branch(&delta);
        let branched_pursuit = sanitize_message_for_branch(&pursuit);

        assert_eq!(branched_goal.role, "goal");
        assert_eq!(branched_goal.extra["goal"]["version"], json!(1));
        assert_eq!(
            branched_delta.extra["event"]["subkind"],
            json!("goal_delta")
        );
        assert_eq!(
            branched_pursuit.extra["event"]["subkind"],
            json!("goal_pursuit")
        );
    }
}
