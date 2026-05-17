use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::Utc;
use tokio::sync::{Mutex as AMutex};
use tracing::warn;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage, ContextFile};
use crate::buddy::user_activity::UserAction;
use crate::files_correction::get_project_dirs;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, first_block_reason, get_project_dir_string, run_hooks};

use super::types::*;
use super::browser_context;
use super::content::parse_content_with_attachments;
use super::generation::{start_generation, prepare_session_preamble_and_knowledge};
use super::tools::{execute_tools_with_session, resolve_tool_call_aliases};
use super::trajectories::maybe_save_trajectory;
use crate::ext::slash_expand::expand_slash_command;
use crate::ext::skills_context::{expand_skill_includes, SKILLS_CONTEXT_MARKER};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{WorktreeMeta, WorktreeReference};

async fn push_user_activity(app: AppState, action: UserAction) {
    let user_activity = app.buddy.user_activity.clone();
    if let Ok(mut ring) = user_activity.try_lock() {
        ring.push(action);
    };
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

fn command_triggers_generation(cmd: &ChatCommand) -> bool {
    matches!(
        cmd,
        ChatCommand::UserMessage { .. }
            | ChatCommand::RetryFromIndex { .. }
            | ChatCommand::Regenerate {}
    )
}

pub async fn inject_priority_messages_if_any(
    app: AppState,
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
            context_files,
            suppress_auto_enrichment: _,
        } = request.command
        {
            let (session_id, project_dir) = {
                let session = session_arc.lock().await;
                let sid = session.chat_id.clone();
                drop(session);
                let pd = get_project_dir_string(app.gcx.clone()).await;
                (sid, pd)
            };
            let prompt_text = match &content {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            let hook_results = run_hooks(
                app.gcx.clone(),
                HookEvent::UserPromptSubmit,
                HookPayload {
                    hook_event_name: "UserPromptSubmit".to_string(),
                    session_id,
                    project_dir,
                    tool_name: None,
                    tool_input: None,
                    tool_output: None,
                    user_prompt: Some(prompt_text),
                    extra: std::collections::HashMap::new(),
                },
            )
            .await;
            if first_block_reason(&hook_results).is_some() {
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

            let mut session = session_arc.lock().await;
            if !context_files.is_empty() {
                apply_manual_context_files(&mut session, &context_files);
            }
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

    maybe_save_trajectory(app.clone(), session_arc.clone()).await;
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
            serde_json::from_value::<Option<crate::buddy::types::BuddyThreadMeta>>(v.clone())
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

#[derive(Clone)]
pub struct WorktreeSetParamsUpdate {
    pub worktree: Option<WorktreeMeta>,
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
    let cache_dir = app.paths.cache_dir.read().unwrap().clone();
    let project_dirs = get_project_dirs(app.gcx.clone()).await;
    if project_dirs.is_empty() {
        return Err("No project root available".to_string());
    }
    let source_root = match requested_source_root {
        Some(requested) => {
            let requested = std::fs::canonicalize(requested).map_err(|e| {
                format!(
                    "Failed to resolve worktree source root '{}': {}",
                    requested.display(),
                    e
                )
            })?;
            let requested = dunce::simplified(&requested).to_path_buf();
            let matches = project_dirs.iter().any(|dir| {
                std::fs::canonicalize(dir)
                    .map(|canonical| dunce::simplified(&canonical).to_path_buf() == requested)
                    .unwrap_or(false)
            });
            if !matches {
                return Err("Worktree source root is not a current workspace directory".to_string());
            }
            requested
        }
        None => project_dirs[0].clone(),
    };
    WorktreeService::new(cache_dir, source_root)
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
    if let Some(worktree_id) = patch.get("worktree_id") {
        let worktree_id = worktree_id
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "worktree_id must be a non-empty string".to_string())?;
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
            changed: thread.worktree.is_some(),
            sse_value: serde_json::Value::Null,
        }));
    }

    Ok(None)
}

pub async fn process_command_queue(
    app: AppState,
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

                maybe_save_trajectory(app.clone(), session_arc.clone()).await;

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
                mut content,
                attachments,
                context_files,
                suppress_auto_enrichment,
            } => {
                let mut skill_activation_info = None;
                if let Some(text) = content.as_str() {
                    match expand_slash_command(app.gcx.clone(), text).await {
                        Ok(Some(expanded)) => {
                            skill_activation_info = expanded.skill_to_activate;
                            content = serde_json::Value::String(expanded.expanded_text);
                            let mut session = session_arc.lock().await;
                            session.active_command = ActiveCommandContext {
                                name: expanded.source_command,
                                allowed_tools: expanded.allowed_tools,
                                model_override: expanded.model_override,
                                context_fork: expanded.context_fork,
                                started_at_index: None,
                                activation_tool_call_id: None,
                            };
                        }
                        Ok(None) => {
                            // No slash command — only reset active_command when no skill is
                            // active. While a skill is active, active_command carries the
                            // compaction anchor (started_at_index) and must not be wiped on
                            // every normal user message.
                            let mut session = session_arc.lock().await;
                            if session.thread.active_skill.is_none() {
                                session.active_command = ActiveCommandContext::default();
                            }
                        }
                        Err(e) => {
                            warn!("slash command expansion error: {}", e);
                            let mut session = session_arc.lock().await;
                            if session.thread.active_skill.is_none() {
                                session.active_command = ActiveCommandContext::default();
                            }
                        }
                    }
                }

                let skill_activation_name: Option<String> =
                    skill_activation_info.as_ref().map(|i| i.name.clone());
                let skill_context_msg = if let Some(info) = skill_activation_info {
                    let body = expand_skill_includes(&info.body, &info.skill_dir).await;
                    let line_count = body.lines().count().max(1);
                    Some(ChatMessage {
                        message_id: Uuid::new_v4().to_string(),
                        role: "context_file".to_string(),
                        content: ChatContent::ContextFiles(vec![ContextFile {
                            file_name: format!("skill://{}", info.name),
                            file_content: body,
                            line1: 1,
                            line2: line_count,
                            file_rev: None,
                            symbols: vec![],
                            gradient_type: 0,
                            usefulness: 95.0,
                            skip_pp: true,
                        }]),
                        tool_call_id: SKILLS_CONTEXT_MARKER.to_string(),
                        ..Default::default()
                    })
                } else {
                    None
                };

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

                let (has_browser_meta, attach_screenshot_on_send, browser_chat_id) = {
                    let session = session_arc.lock().await;
                    let bm = session.thread.browser_meta.as_ref();
                    (
                        bm.is_some(),
                        bm.map_or(false, |m| m.attach_screenshot_on_send),
                        session.chat_id.clone(),
                    )
                };

                let browser_ctx_result = browser_context::maybe_insert_browser_context(
                    app.gcx.clone(),
                    &browser_chat_id,
                    has_browser_meta,
                    attach_screenshot_on_send,
                )
                .await;

                let (session_id_for_hook, project_dir_for_hook) = {
                    let session = session_arc.lock().await;
                    let sid = session.chat_id.clone();
                    drop(session);
                    let pd = get_project_dir_string(app.gcx.clone()).await;
                    (sid, pd)
                };
                let prompt_text = match &content {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                let prompt_payload = HookPayload {
                    hook_event_name: "UserPromptSubmit".to_string(),
                    session_id: session_id_for_hook.clone(),
                    project_dir: project_dir_for_hook.clone(),
                    tool_name: None,
                    tool_input: None,
                    tool_output: None,
                    user_prompt: Some(prompt_text),
                    extra: std::collections::HashMap::new(),
                };
                let prompt_results =
                    run_hooks(app.gcx.clone(), HookEvent::UserPromptSubmit, prompt_payload).await;
                if let Some(reason) = first_block_reason(&prompt_results) {
                    let mut session = session_arc.lock().await;
                    session.emit(super::types::ChatEvent::RuntimeUpdated {
                        state: super::types::SessionState::Error,
                        error: Some(format!("Message blocked by hook: {}", reason)),
                    });
                    session.set_runtime_state(super::types::SessionState::Idle, None);
                    continue;
                }

                let additional_messages = {
                    let mut approved = Vec::new();
                    for additional in additional_messages {
                        let text = if let ChatCommand::UserMessage { ref content, .. } =
                            additional.command
                        {
                            match content {
                                serde_json::Value::String(s) => s.clone(),
                                other => serde_json::to_string(other).unwrap_or_default(),
                            }
                        } else {
                            approved.push(additional);
                            continue;
                        };
                        let add_results = run_hooks(
                            app.gcx.clone(),
                            HookEvent::UserPromptSubmit,
                            HookPayload {
                                hook_event_name: "UserPromptSubmit".to_string(),
                                session_id: session_id_for_hook.clone(),
                                project_dir: project_dir_for_hook.clone(),
                                tool_name: None,
                                tool_input: None,
                                tool_output: None,
                                user_prompt: Some(text),
                                extra: std::collections::HashMap::new(),
                            },
                        )
                        .await;
                        if first_block_reason(&add_results).is_none() {
                            approved.push(additional);
                        }
                    }
                    approved
                };

                let is_oversize = browser_ctx_result
                    .as_ref()
                    .map_or(false, |(_, oversize)| *oversize);

                if is_oversize {
                    if let Some((_, true)) = browser_ctx_result {
                        let snapshot = browser_context::get_browser_context_for_chat(
                            app.gcx.clone(),
                            &browser_chat_id,
                        )
                        .await;
                        if let Some(ref snap) = snapshot {
                            let action_bytes = serde_json::to_string(&snap.actions)
                                .unwrap_or_default()
                                .len();
                            let console_bytes = serde_json::to_string(&snap.console)
                                .unwrap_or_default()
                                .len();
                            let network_bytes = serde_json::to_string(&snap.network)
                                .unwrap_or_default()
                                .len();
                            let mutation_bytes = serde_json::to_string(&snap.mutations)
                                .unwrap_or_default()
                                .len();
                            let pending_message_id = Uuid::new_v4().to_string();
                            let mut session = session_arc.lock().await;
                            session.pending_browser_message = Some(PendingBrowserMessage {
                                pending_message_id: pending_message_id.clone(),
                                content: content.clone(),
                                attachments: attachments.clone(),
                                checkpoints: checkpoints.clone(),
                                context_files: context_files.clone(),
                                suppress_auto_enrichment,
                                skill_activation_name: skill_activation_name.clone(),
                                skill_context_msg: skill_context_msg.clone(),
                            });
                            session.emit(ChatEvent::BrowserContextOversize {
                                total_bytes: action_bytes
                                    + console_bytes
                                    + network_bytes
                                    + mutation_bytes,
                                action_count: snap.actions.len(),
                                action_bytes,
                                console_count: snap.console.len(),
                                console_bytes,
                                network_count: snap.network.len(),
                                network_bytes,
                                mutation_bytes,
                                pending_message_id: pending_message_id.clone(),
                            });
                            session.set_runtime_state(SessionState::WaitingUserInput, None);
                        }
                    }
                    continue;
                }

                {
                    let mut session = session_arc.lock().await;

                    if let Some((ctx_msg, _)) = browser_ctx_result {
                        session.add_message(ctx_msg);
                    }

                    // Set compaction anchor for slash-command skill activation before any skill
                    // messages are added, so deactivate_skill can truncate back to this point.
                    if skill_activation_name.is_some()
                        && session.active_command.started_at_index.is_none()
                    {
                        session.active_command.started_at_index = Some(session.messages.len());
                    }

                    if let Some(skill_msg) = skill_context_msg {
                        session.add_message(skill_msg);
                    }

                    if !context_files.is_empty() {
                        apply_manual_context_files(&mut session, &context_files);
                    }

                    let parsed_content = parse_content_with_attachments(&content, &attachments);
                    let user_message = ChatMessage {
                        message_id: Uuid::new_v4().to_string(),
                        role: "user".to_string(),
                        content: parsed_content.clone(),
                        checkpoints,
                        ..Default::default()
                    };
                    session.add_message(user_message);
                    if session.messages.iter().filter(|m| m.role == "user").count() == 1 {
                        let chat_id = session.chat_id.clone();
                        let first_user_text_preview = parsed_content
                            .content_text_only()
                            .chars()
                            .take(80)
                            .collect();
                        drop(session);
                        push_user_activity(
                            app.clone(),
                            UserAction::ChatStarted {
                                chat_id,
                                first_user_text_preview,
                                ts: Utc::now(),
                            },
                        )
                        .await;
                        session = session_arc.lock().await;
                    }

                    if suppress_auto_enrichment && context_files.is_empty() {
                        session.suppress_auto_enrichment_for_next_turn = true;
                    }

                    if let Some(ref skill_name) = skill_activation_name {
                        session.set_active_skill(skill_name.clone());
                    }

                    for additional in additional_messages {
                        if let ChatCommand::UserMessage {
                            content: add_content,
                            attachments: add_attachments,
                            context_files: add_ctx_files,
                            suppress_auto_enrichment: _,
                        } = additional.command
                        {
                            if !add_ctx_files.is_empty() {
                                apply_manual_context_files(&mut session, &add_ctx_files);
                            }
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

                maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
                start_generation(app.clone(), session_arc.clone()).await;
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

                maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
                start_generation(app.clone(), session_arc.clone()).await;
            }
            ChatCommand::SetParams { patch } => {
                if !patch.is_object() {
                    warn!("SetParams patch must be an object, ignoring");
                    continue;
                }
                let (chat_id, thread_before) = {
                    let session = session_arc.lock().await;
                    (session.chat_id.clone(), session.thread.clone())
                };
                let worktree_update = match resolve_worktree_setparams_update(
                    app.clone(),
                    &chat_id,
                    &thread_before,
                    &patch,
                )
                .await
                {
                    Ok(update) => update,
                    Err(e) => {
                        warn!("SetParams worktree update rejected: {}", e);
                        let mut session = session_arc.lock().await;
                        session.emit(ChatEvent::RuntimeUpdated {
                            state: SessionState::Error,
                            error: Some(e),
                        });
                        session.set_runtime_state(SessionState::Idle, None);
                        continue;
                    }
                };
                let mut session = session_arc.lock().await;
                let (mut changed, sanitized_patch) =
                    apply_setparams_patch(&mut session.thread, &patch);
                if let Some(update) = worktree_update.clone() {
                    session.thread.worktree = update.worktree;
                    changed |= update.changed;
                }

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
                    if let Some(update) = worktree_update {
                        obj.insert("worktree".to_string(), update.sse_value);
                    }
                }
                session.emit(ChatEvent::ThreadUpdated {
                    params: patch_for_chat_sse,
                });
                if changed {
                    session.increment_version();
                    session.touch();
                }
                drop(session);
                if changed {
                    maybe_save_trajectory(app.clone(), session_arc.clone()).await;
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
                handle_tool_decisions(app.clone(), session_arc.clone(), &decisions).await;
            }
            ChatCommand::ToolDecisions { decisions } => {
                handle_tool_decisions(app.clone(), session_arc.clone(), &decisions).await;
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
                start_generation(app.clone(), session_arc.clone()).await;
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
                        maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                        prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone())
                            .await;
                        start_generation(app.clone(), session_arc.clone()).await;
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
                        maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                        prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone())
                            .await;
                        start_generation(app.clone(), session_arc.clone()).await;
                    }
                }
            }
            ChatCommand::Regenerate {} => {
                prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
                start_generation(app.clone(), session_arc.clone()).await;
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
                maybe_save_trajectory(app.clone(), session_arc.clone()).await;
            }
            ChatCommand::BranchFromChat {
                source_chat_id,
                up_to_message_id,
            } => {
                if let Err(e) = super::trajectories::validate_trajectory_id(&source_chat_id) {
                    warn!("BranchFromChat: invalid source_chat_id: {}", e);
                    continue;
                }

                let sessions = app.chat.sessions.clone();

                let source_session_arc = super::session::get_or_create_session_with_trajectory(
                    app.clone(),
                    &sessions,
                    &source_chat_id,
                )
                .await;

                let (messages_to_copy, root_id, source_worktree) = {
                    let source_session = source_session_arc.lock().await;
                    let mut msgs = Vec::new();
                    let mut found = false;
                    for m in &source_session.messages {
                        if is_allowed_role_for_branch(&m.role) {
                            msgs.push(sanitize_message_for_branch(m));
                        }
                        if m.message_id == up_to_message_id {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        warn!(
                            "BranchFromChat: up_to_message_id '{}' not found in source chat",
                            up_to_message_id
                        );
                        continue;
                    }
                    let root = source_session
                        .thread
                        .root_chat_id
                        .clone()
                        .unwrap_or_else(|| source_chat_id.clone());
                    (msgs, root, source_session.thread.worktree.clone())
                };

                let mut session = session_arc.lock().await;
                session.thread.parent_id = Some(source_chat_id.clone());
                session.thread.link_type = Some("branch".to_string());
                session.thread.root_chat_id = Some(root_id);

                for msg in messages_to_copy {
                    session.add_message(msg);
                }
                let target_thread = session.thread.clone();
                drop(session);
                if let Some(worktree) = source_worktree.as_ref() {
                    if let Some(validated) = add_thread_worktree_reference(
                        app.clone(),
                        &target_thread.id,
                        &target_thread,
                        worktree,
                    )
                    .await
                    {
                        let mut session = session_arc.lock().await;
                        session.thread.worktree = Some(validated);
                    }
                }
                maybe_save_trajectory(app.clone(), session_arc.clone()).await;
            }
            ChatCommand::BrowserContextDecision {
                pending_message_id,
                include_actions,
                include_console,
                include_network,
                include_mutations,
                include_screenshot,
                last_n_actions,
                last_n_console,
                last_n_network,
            } => {
                let pending = {
                    let mut session = session_arc.lock().await;
                    session.pending_browser_message.take()
                };
                let Some(pending) = pending else {
                    warn!("BrowserContextDecision: no pending message found");
                    let mut session = session_arc.lock().await;
                    session.set_runtime_state(SessionState::Idle, None);
                    continue;
                };
                if pending.pending_message_id != pending_message_id {
                    warn!("BrowserContextDecision: pending_message_id mismatch");
                    let mut session = session_arc.lock().await;
                    session.set_runtime_state(SessionState::Idle, None);
                    continue;
                }

                let browser_chat_id = {
                    let session = session_arc.lock().await;
                    session.chat_id.clone()
                };

                let snapshot =
                    browser_context::get_browser_context_for_chat(app.gcx.clone(), &browser_chat_id)
                        .await;

                {
                    let mut session = session_arc.lock().await;

                    if let Some(mut snap) = snapshot {
                        browser_context::apply_decision_to_snapshot(
                            &mut snap,
                            include_actions,
                            include_console,
                            include_network,
                            include_mutations,
                            last_n_actions,
                            last_n_console,
                            last_n_network,
                        );
                        let ctx_msg =
                            browser_context::make_context_message(&snap, include_screenshot);
                        session.add_message(ctx_msg);
                    }

                    if pending.skill_activation_name.is_some()
                        && session.active_command.started_at_index.is_none()
                    {
                        session.active_command.started_at_index = Some(session.messages.len());
                    }
                    if let Some(skill_msg) = pending.skill_context_msg {
                        session.add_message(skill_msg);
                    }
                    if !pending.context_files.is_empty() {
                        apply_manual_context_files(&mut session, &pending.context_files);
                    }

                    if pending.suppress_auto_enrichment && pending.context_files.is_empty() {
                        session.suppress_auto_enrichment_for_next_turn = true;
                    }

                    let parsed_content =
                        parse_content_with_attachments(&pending.content, &pending.attachments);
                    let user_message = ChatMessage {
                        message_id: Uuid::new_v4().to_string(),
                        role: "user".to_string(),
                        content: parsed_content,
                        checkpoints: pending.checkpoints,
                        ..Default::default()
                    };
                    session.add_message(user_message);

                    if let Some(ref skill_name) = pending.skill_activation_name {
                        session.set_active_skill(skill_name.clone());
                    }
                }

                browser_context::commit_browser_cursors(app.gcx.clone(), &browser_chat_id).await;
                maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
                start_generation(app.clone(), session_arc.clone()).await;
            }
        }
    }
}

fn is_allowed_role_for_restore(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "system" | "tool")
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
        usage: None,
        checkpoints: vec![],
        reasoning_content: msg.reasoning_content.clone(),
        thinking_blocks: msg.thinking_blocks.clone(),
        citations: msg.citations.clone(),
        server_content_blocks: msg.server_content_blocks.clone(),
        finish_reason: None,
        extra: serde_json::Map::new(),
        output_filter: None,
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
        usage: None,
        checkpoints: msg.checkpoints.clone(),
        reasoning_content: msg.reasoning_content.clone(),
        thinking_blocks: None,
        citations: msg.citations.clone(),
        server_content_blocks: msg.server_content_blocks.clone(),
        finish_reason: None,
        extra: serde_json::Map::new(),
        output_filter: None,
    }
}

fn is_allowed_role_for_branch(role: &str) -> bool {
    matches!(
        role,
        "user" | "assistant" | "system" | "tool" | "context_file"
    )
}

async fn handle_tool_decisions(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
) {
    let is_cache_guard_pause = {
        let session = session_arc.lock().await;
        session
            .runtime
            .pause_reasons
            .iter()
            .any(crate::chat::cache_guard::is_cache_guard_pause_reason)
    };

    if is_cache_guard_pause {
        let accepted_any = decisions.iter().any(|d| d.accepted);

        {
            let mut session = session_arc.lock().await;
            if accepted_any {
                session.cache_guard_force_next = true;
            }
            session.runtime.pause_reasons.clear();
            session.runtime.accepted_tool_ids.clear();
            session.runtime.auto_approved_tool_ids.clear();
            session.runtime.paused_message_index = None;
            session.set_runtime_state(SessionState::Idle, None);
        }

        if accepted_any {
            start_generation(app.clone(), session_arc.clone()).await;
        } else {
            maybe_save_trajectory(app.clone(), session_arc.clone()).await;
        }
        return;
    }

    let (
        auto_approved_ids,
        has_remaining_pauses,
        tool_calls_to_execute,
        messages,
        thread,
        any_rejected,
    ) = {
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

        let mut ids_to_execute: std::collections::HashSet<String> =
            session.runtime.accepted_tool_ids.iter().cloned().collect();
        if !any_rejected && !remaining {
            for id in &auto_approved {
                ids_to_execute.insert(id.clone());
            }
        }

        let tool_calls: Vec<crate::call_validation::ChatToolCall> =
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
    }

    let had_tool_calls = !tool_calls_to_execute.is_empty();
    if had_tool_calls {
        let tool_calls_to_execute = resolve_tool_call_aliases(
            app.clone(),
            tool_calls_to_execute,
            &thread.mode,
            Some(&thread.model),
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
            Some(&thread.model),
            super::tools::ExecuteToolsOptions::default(),
        )
        .await;

        // Determine tool-requested final state before checking abort.
        // Some tools (ask_questions/task_done/task_agent_finish) set abort_flag=true as part of
        // normal operation to stop further LLM generation.
        let mut final_state = SessionState::Idle;
        for tool_call in &tool_calls_to_execute {
            let tool_name =
                crate::llm::adapters::claude_code_compat::cc_normalize_internal_tool_name(
                    &tool_call.function.name,
                );
            match tool_name.as_str() {
                "ask_questions" | "task_wait_for_agents" => {
                    final_state = SessionState::WaitingUserInput
                }
                "task_done" => final_state = SessionState::Completed,
                "task_agent_finish" => final_state = SessionState::Completed,
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

        {
            let mut session = session_arc.lock().await;
            for result_msg in tool_results {
                session.add_message(result_msg);
            }
            if tool_initiated_stop {
                session.set_runtime_state(final_state, None);
            } else {
                // Always transition to Idle — either normally or after user abort.
                // abort_stream() may have already set Idle, but set_runtime_state
                // is idempotent and ensures the UI gets the RuntimeUpdated event.
                session.set_runtime_state(SessionState::Idle, None);
            }
        }

        {
            let mut session = session_arc.lock().await;
            if session.pending_skill_deactivation.is_some() {
                session.perform_skill_deactivation_cleanup();
            }
        }

        maybe_save_trajectory(app.clone(), session_arc.clone()).await;

        if was_aborted || tool_initiated_stop {
            return;
        }
    }

    if any_rejected {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
        }
        maybe_save_trajectory(app, session_arc).await;
    } else if had_tool_calls {
        start_generation(app, session_arc).await;
    } else {
        {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
        }
        maybe_save_trajectory(app, session_arc).await;
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
        create_workspace_checkpoint_for_root(app.gcx.clone(), &worktree.root, latest_checkpoint, chat_id).await
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
    use serde_json::json;
    use std::path::Path;
    use std::process::Command;

    fn make_request(cmd: ChatCommand) -> CommandRequest {
        CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: cmd,
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

    #[test]
    fn test_find_allowed_command_empty_queue() {
        let queue = VecDeque::new();
        assert!(find_allowed_command_while_paused(&queue).is_none());
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
        }));
        queue.push_back(make_request(ChatCommand::UserMessage {
            content: json!("another"),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
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

    #[tokio::test]
    async fn worktree_setparams_attach_by_id_resolves_registry_and_scopes_create() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            *app.paths.cache_dir.write().unwrap() = cache.clone();
        }
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
    async fn worktree_setparams_detach_clears_registry_reference() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            *app.paths.cache_dir.write().unwrap() = cache.clone();
        }
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            *app.paths.cache_dir.write().unwrap() = cache.clone();
        }
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
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-2".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("second"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        let priority_req = CommandRequest {
            client_request_id: "req-priority".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
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
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("normal"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        let priority_req = CommandRequest {
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("p2"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
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
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("normal"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-p2".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority 2"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
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
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-p".into(),
            priority: true,
            command: ChatCommand::UserMessage {
                content: json!("priority"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-2".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("second"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        queue.push_back(CommandRequest {
            client_request_id: "req-3".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("third"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
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
        assert_eq!(SKILLS_CONTEXT_MARKER, "skills_context",
            "SKILLS_CONTEXT_MARKER must equal 'skills_context' so prompts.rs detects existing skills context");
    }
}
