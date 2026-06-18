use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::agentic::mode_transition::{AgenticPathContext, ParsedDecisions, assemble_new_chat};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::trajectories::{
    chat_id_is_planner_for_task, resolve_task_planner_controller_chat_id,
    verified_planner_linked_root_chat_id,
};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tasks::storage;
use crate::tools::tool_task_documents::{
    create_document_at, documents_dir_for_task, next_available_slug_at,
};
use refact_chat_history::trajectory_ops::sanitize_messages_for_new_thread;
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use refact_runtime_api::{ChatSessionUpdate, SessionState};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

fn parse_string_list(args: &HashMap<String, Value>, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.starts_with('[') {
                serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_default()
            } else {
                trimmed
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
        }
        _ => vec![],
    }
}

fn parse_optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    match args.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

fn apply_overrides(decisions: &mut ParsedDecisions, args: &HashMap<String, Value>) {
    if let Some(summary) = parse_optional_string(args, "summary") {
        decisions.summary = summary;
    }
    if let Some(summary) = parse_optional_string(args, "context_summary") {
        decisions.summary = summary;
    }
    let files_to_open = parse_string_list(args, "files_to_open");
    if !files_to_open.is_empty() {
        decisions.files_to_open = files_to_open;
    }
    let key_files = parse_string_list(args, "key_files");
    if !key_files.is_empty() {
        decisions.files_to_open = key_files;
    }
    let messages_to_preserve = parse_string_list(args, "messages_to_preserve");
    if !messages_to_preserve.is_empty() {
        decisions.messages_to_preserve = messages_to_preserve;
    }
    let memories_to_include = parse_string_list(args, "memories_to_include");
    if !memories_to_include.is_empty() {
        decisions.memories_to_include = memories_to_include;
    }
    let tool_outputs_to_include = parse_string_list(args, "tool_outputs_to_include");
    if !tool_outputs_to_include.is_empty() {
        decisions.tool_outputs_to_include = tool_outputs_to_include;
    }
    let pending_tasks = parse_string_list(args, "pending_tasks");
    if !pending_tasks.is_empty() {
        decisions.pending_tasks = pending_tasks;
    }
    if let Some(handoff_message) = parse_optional_string(args, "handoff_message") {
        decisions.handoff_message = handoff_message;
    }
    if let Some(initial_plan) = parse_optional_string(args, "initial_plan") {
        decisions.initial_plan = Some(initial_plan);
    }
}

async fn ensure_task_for_planner_handoff(
    gcx: Arc<crate::global_context::GlobalContext>,
    canonical_mode: &str,
    existing_task_meta: Option<refact_chat_api::TaskMeta>,
    current_chat_id: &str,
    current_root_chat_id: Option<&str>,
) -> Result<Option<refact_chat_api::TaskMeta>, String> {
    if canonical_mode != "task_planner" {
        return Ok(existing_task_meta);
    }
    if let Some(task_meta) = existing_task_meta {
        if task_meta.role == "planner" {
            let planner_chat_id = resolve_task_planner_controller_chat_id(
                gcx.clone(),
                current_chat_id,
                current_root_chat_id,
                Some(&task_meta),
            )
            .await;
            return Ok(Some(refact_chat_api::TaskMeta {
                task_id: task_meta.task_id,
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: Some(planner_chat_id),
            }));
        }
        if let Some(planner_chat_id) = verified_planner_linked_root_chat_id(
            gcx.clone(),
            current_chat_id,
            current_root_chat_id,
            &task_meta,
        )
        .await
        {
            return Ok(Some(refact_chat_api::TaskMeta {
                task_id: task_meta.task_id,
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: Some(planner_chat_id),
            }));
        }
        if let Some(planner_chat_id) = task_meta
            .planner_chat_id
            .clone()
            .filter(|id| !id.is_empty())
        {
            if chat_id_is_planner_for_task(gcx.clone(), &planner_chat_id, &task_meta.task_id).await
            {
                return Ok(Some(refact_chat_api::TaskMeta {
                    task_id: task_meta.task_id,
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some(planner_chat_id),
                }));
            }
        }
        let chat_id = storage::next_planner_chat_id(gcx, &task_meta.task_id).await?;
        return Ok(Some(refact_chat_api::TaskMeta {
            task_id: task_meta.task_id,
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some(chat_id),
        }));
    }
    let task = storage::create_task(gcx.clone(), "New Task").await?;
    let chat_id = storage::next_planner_chat_id(gcx, &task.id).await?;
    Ok(Some(refact_chat_api::TaskMeta {
        task_id: task.id,
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
        planner_chat_id: Some(chat_id),
    }))
}

async fn create_initial_plan_document(
    gcx: Arc<crate::global_context::GlobalContext>,
    task_id: &str,
    plan_text: &str,
) -> Result<String, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    let slug = next_available_slug_at(&documents_dir, "initial-plan").await?;
    create_document_at(
        &documents_dir,
        &slug,
        "Initial Plan",
        "plan",
        plan_text,
        true,
        Vec::new(),
        "planner",
    )
    .await?;
    Ok(slug)
}

struct ExistingHandoffMessages {
    messages: Vec<ChatMessage>,
    live_session: bool,
}

async fn existing_messages_for_handoff_target(
    gcx: Arc<crate::global_context::GlobalContext>,
    chat_id: &str,
) -> Option<ExistingHandoffMessages> {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        return Some(ExistingHandoffMessages {
            messages: session_arc.lock().await.messages.clone(),
            live_session: true,
        });
    }

    crate::chat::trajectories::load_trajectory_for_chat(gcx, chat_id)
        .await
        .map(|loaded| ExistingHandoffMessages {
            messages: loaded.messages,
            live_session: false,
        })
}

pub struct ToolHandoffToMode {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolHandoffToMode {
    fn tool_description(&self) -> ToolDesc {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "target_mode": {
                    "type": "string",
                    "description": "Target mode ID to hand off to."
                },
                "reason": {
                    "type": "string",
                    "description": "Why the new mode is appropriate"
                },
                "summary": {
                    "type": "string",
                    "description": "Optional summary to include in the handoff context"
                },
                "context_summary": {
                    "type": "string",
                    "description": "Summary of what has been done and what to continue"
                },
                "files_to_open": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "File paths to include in the new chat"
                },
                "key_files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Key files to carry over (alias of files_to_open)"
                },
                "messages_to_preserve": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "MSG_ID entries to preserve verbatim"
                },
                "memories_to_include": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Memory/knowledge file paths to include"
                },
                "tool_outputs_to_include": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "MSG_ID entries of tool outputs to include"
                },
                "pending_tasks": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Pending tasks to carry forward"
                },
                "handoff_message": {
                    "type": "string",
                    "description": "Short handoff message for the new chat"
                },
                "initial_plan": {
                    "type": "string",
                    "description": "Optional plan text to save as the initial task document when target_mode is task_planner"
                }
            },
            "required": ["target_mode"]
        });

        ToolDesc {
            name: "handoff_to_mode".to_string(),
            display_name: "Handoff To Mode".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description:
                "Create a new chat in another mode using the current conversation context."
                    .to_string(),
            input_schema,
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let target_mode = match args.get("target_mode") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Err("Missing required argument `target_mode`".to_string()),
        };
        let reason = parse_optional_string(args, "reason").unwrap_or_default();

        let (gcx, chat_facade, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.chat.facade.clone(),
                ccx_lock.chat_id.clone(),
            )
        };

        let session_snapshot = chat_facade.session_snapshot(&chat_id).await?;
        let messages = session_snapshot.messages;
        let thread = session_snapshot.thread;
        let existing_task_meta = thread.task_meta.clone();
        let session_state = session_snapshot.session_state;
        let pause_reasons = session_snapshot.pause_reasons;

        match session_state {
            SessionState::Generating => {
                return Err("Cannot handoff while model is generating. Wait for the current response to complete.".to_string());
            }
            SessionState::Paused => {
                return Err(
                    "Cannot handoff while session is paused. Resume or abort first.".to_string(),
                );
            }
            SessionState::WaitingIde => {
                return Err("Cannot handoff while waiting for IDE response. Cancel the IDE wait or wait for it to complete.".to_string());
            }
            SessionState::Error => {
                return Err(
                    "Cannot handoff from an error state. Acknowledge the error first.".to_string(),
                );
            }
            SessionState::WaitingUserInput => {
                if !pause_reasons.is_empty() {
                    return Err(
                        "Cannot handoff while pending tool approvals exist. Resolve them first."
                            .to_string(),
                    );
                }
            }
            SessionState::ExecutingTools | SessionState::Idle | SessionState::Completed => {}
        }
        if messages.is_empty() {
            return Err("Cannot handoff an empty chat".to_string());
        }
        let last_asst_idx = messages
            .iter()
            .rposition(|m| m.role == "assistant" && m.tool_calls.is_some());
        if let Some(asst_idx) = last_asst_idx {
            let asst = &messages[asst_idx];
            let call_ids: std::collections::HashSet<&str> = asst
                .tool_calls
                .as_ref()
                .unwrap()
                .iter()
                .map(|c| c.id.as_str())
                .collect();
            let result_ids: std::collections::HashSet<&str> = messages[asst_idx + 1..]
                .iter()
                .filter(|m| {
                    (m.role == "tool" || m.role == "diff" || m.role == "context_file")
                        && !m.tool_call_id.is_empty()
                })
                .map(|m| m.tool_call_id.as_str())
                .collect();
            let mut missing_ids: Vec<&str> = call_ids
                .difference(&result_ids)
                .copied()
                .filter(|id| *id != tool_call_id.as_str())
                .collect();
            if !missing_ids.is_empty() {
                missing_ids.sort();
                return Err(format!(
                    "Cannot handoff: the latest assistant message has {} tool calls without results: {:?}",
                    missing_ids.len(),
                    missing_ids
                ));
            }
        }

        let canonical_mode = map_legacy_mode_to_id(&target_mode).to_string();
        let mode_config = get_mode_config(gcx.clone(), &canonical_mode, None)
            .await
            .ok_or_else(|| format!("Mode '{}' not found", canonical_mode))?;
        if thread.mode == canonical_mode {
            return Err("Target mode matches current mode".to_string());
        }

        let mode_title = if mode_config.title.is_empty() {
            mode_config.id.clone()
        } else {
            mode_config.title.clone()
        };
        let mode_description = if mode_config.description.is_empty() {
            mode_title.clone()
        } else {
            format!("{} — {}", mode_title, mode_config.description)
        };

        let mut decisions = ParsedDecisions {
            summary: if reason.is_empty() {
                format!("Continue the conversation in {}.", mode_description)
            } else {
                reason.clone()
            },
            handoff_message: if reason.is_empty() {
                format!("Continue in {}.", mode_description)
            } else {
                reason.clone()
            },
            ..Default::default()
        };

        apply_overrides(&mut decisions, args);
        let plan_body = parse_optional_string(args, "initial_plan")
            .or_else(|| parse_optional_string(args, "summary"))
            .or_else(|| parse_optional_string(args, "context_summary"))
            .or_else(|| parse_optional_string(args, "handoff_message"))
            .unwrap_or_else(|| {
                "# Initial Plan\n\nNo plan content provided at handoff. Edit this document or use doc_create.".to_string()
            });

        let path_context = { AgenticPathContext::from_context(&*gcx) };
        let new_messages = assemble_new_chat(&path_context, &messages, &decisions)
            .await
            .map_err(|e| format!("handoff assembly failed: {}", e))?;

        let new_messages = sanitize_messages_for_new_thread(&new_messages);
        let task_meta = ensure_task_for_planner_handoff(
            gcx.clone(),
            &canonical_mode,
            existing_task_meta,
            &chat_id,
            thread.root_chat_id.as_deref(),
        )
        .await?;
        let new_chat_id = if canonical_mode == "task_planner" {
            task_meta
                .as_ref()
                .and_then(|meta| meta.planner_chat_id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string())
        } else {
            Uuid::new_v4().to_string()
        };
        let root_chat_id = if canonical_mode == "task_planner" {
            Some(new_chat_id.clone())
        } else {
            thread
                .root_chat_id
                .clone()
                .or_else(|| Some(chat_id.clone()))
        };
        let now = chrono::Utc::now().to_rfc3339();

        let existing_target = if canonical_mode == "task_planner" {
            existing_messages_for_handoff_target(gcx.clone(), &new_chat_id).await
        } else {
            None
        };
        let live_target_session = existing_target
            .as_ref()
            .is_some_and(|existing| existing.live_session);
        let snapshot_messages = existing_target
            .map(|mut existing| {
                existing.messages.extend(new_messages.clone());
                existing.messages
            })
            .unwrap_or_else(|| new_messages.clone());

        let snapshot_task_meta = task_meta.clone();
        let snapshot = TrajectorySnapshot {
            goal: None,
            chat_id: new_chat_id.clone(),
            title: String::new(),
            model: thread.model.clone(),
            mode: canonical_mode.clone(),
            tool_use: thread.tool_use.clone(),
            messages: snapshot_messages.clone(),
            created_at: now,
            boost_reasoning: thread.boost_reasoning.unwrap_or(false),
            checkpoints_enabled: thread.checkpoints_enabled,
            context_tokens_cap: thread.context_tokens_cap,
            include_project_info: thread.include_project_info,
            is_title_generated: false,
            auto_approve_editing_tools: thread.auto_approve_editing_tools,
            auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
            autonomous_no_confirm: thread.autonomous_no_confirm,
            version: 1,
            task_meta: snapshot_task_meta,
            worktree: thread.worktree.clone(),
            parent_id: Some(chat_id.clone()),
            link_type: Some("handoff".to_string()),
            root_chat_id: root_chat_id.clone(),
            reasoning_effort: thread.reasoning_effort.clone(),
            thinking_budget: thread.thinking_budget,
            temperature: thread.temperature,
            frequency_penalty: thread.frequency_penalty,
            max_tokens: thread.max_tokens,
            parallel_tool_calls: thread.parallel_tool_calls,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: thread.auto_enrichment_enabled,
            buddy_meta: None,
            auto_compact_enabled: thread.auto_compact_enabled,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
        };

        if live_target_session {
            chat_facade
                .update_session(
                    &new_chat_id,
                    ChatSessionUpdate {
                        messages: snapshot_messages.clone(),
                        previous_response_id: None,
                    },
                )
                .await
                .map_err(|e| format!("Failed to update planner handoff session: {}", e))?;
        }

        chat_facade
            .save_trajectory_snapshot(snapshot)
            .await
            .map_err(|e| format!("Failed to save handoff trajectory: {}", e))?;

        let (initial_plan_doc_slug, initial_plan_doc_error) = if canonical_mode == "task_planner" {
            if let Some(task_id_str) = task_meta.as_ref().map(|meta| meta.task_id.as_str()) {
                match create_initial_plan_document(gcx.clone(), task_id_str, &plan_body).await {
                    Ok(slug) => (Some(slug), None::<String>),
                    Err(err) => {
                        tracing::warn!(
                            "failed to create initial-plan document for task {}: {}",
                            task_id_str,
                            err
                        );
                        (None, Some(err))
                    }
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let result = json!({
            "type": "handoff_to_mode",
            "new_chat_id": new_chat_id,
            "target_mode": canonical_mode,
            "reason": reason,
            "messages_count": new_messages.len(),
            "task_meta": task_meta,
            "root_chat_id": root_chat_id,
            "parent_id": chat_id,
            "link_type": "handoff",
            "initial_plan_document": initial_plan_doc_slug,
            "initial_plan_error": initial_plan_doc_error,
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
                tool_call_id: tool_call_id.clone(),
                preserve: Some(true),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let target = args
            .get("target_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        Ok(format!("handoff_to_mode {}", target))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self
            .command_to_match_against_confirm_deny(ccx.clone(), args)
            .await
            .map_err(|e| format!("Error getting tool command to match: {}", e))?;
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: command_to_match,
            rule: "default".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_runtime_api::{ChatSessionFacade, ChatSessionSnapshot, ChatSessionUpdate};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MockChatFacade {
        snapshot: StdMutex<Option<ChatSessionSnapshot>>,
        saved: StdMutex<Vec<TrajectorySnapshot>>,
        fail_update_session: StdMutex<bool>,
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            self.snapshot
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "missing snapshot".to_string())
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            _update: ChatSessionUpdate,
        ) -> Result<(), String> {
            if *self.fail_update_session.lock().unwrap() {
                return Err("update_session should not be called".to_string());
            }
            Ok(())
        }

        async fn create_session(
            &self,
            _request: refact_runtime_api::CreateSessionRequest,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(
            &self,
            _chat_id: &str,
            _command: refact_chat_api::ChatCommand,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn session_state(&self, _chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(Some(SessionState::Idle))
        }

        async fn maybe_save_session(&self, _chat_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn save_trajectory_snapshot(
            &self,
            snapshot: TrajectorySnapshot,
        ) -> Result<(), String> {
            self.saved.lock().unwrap().push(snapshot);
            Ok(())
        }
    }

    async fn test_app_with_workspace(
        root: &std::path::Path,
        facade: Arc<MockChatFacade>,
    ) -> crate::app_state::AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        let mut app = crate::app_state::AppState::from_gcx(gcx).await;
        app.chat.facade = facade;
        app
    }

    async fn handoff_ccx(app: crate::app_state::AppState) -> Arc<AMutex<AtCommandsContext>> {
        handoff_ccx_with_chat_id(app, "source-chat").await
    }

    async fn handoff_ccx_with_chat_id(
        app: crate::app_state::AppState,
        chat_id: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn source_snapshot() -> ChatSessionSnapshot {
        let mut thread = refact_chat_api::ThreadParams::default();
        thread.id = "source-chat".to_string();
        thread.mode = "agent".to_string();
        thread.tool_use = "agent".to_string();
        thread.model = "model".to_string();
        ChatSessionSnapshot {
            messages: vec![ChatMessage::new(
                "user".to_string(),
                "Please create a plan.".to_string(),
            )],
            thread,
            session_state: SessionState::Idle,
            pause_reasons: vec![],
        }
    }

    async fn insert_planner_session(
        app: &crate::app_state::AppState,
        chat_id: &str,
        task_id: &str,
    ) {
        insert_planner_session_with_messages(app, chat_id, task_id, Vec::new()).await;
    }

    async fn insert_planner_session_with_messages(
        app: &crate::app_state::AppState,
        chat_id: &str,
        task_id: &str,
        messages: Vec<ChatMessage>,
    ) {
        let mut session = crate::chat::types::ChatSession::new(chat_id.to_string());
        session.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: task_id.to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some(chat_id.to_string()),
        });
        for message in messages {
            session.add_message(message);
        }
        app.gcx.chat_sessions.write().await.insert(
            chat_id.to_string(),
            Arc::new(tokio::sync::Mutex::new(session)),
        );
    }

    fn handoff_args(initial_plan: &str) -> HashMap<String, Value> {
        HashMap::from([
            ("target_mode".to_string(), json!("task_planner")),
            ("reason".to_string(), json!("Plan this task")),
            ("initial_plan".to_string(), json!(initial_plan)),
        ])
    }

    fn tool_result_json(messages: &[ContextEnum]) -> serde_json::Value {
        match &messages[0] {
            ContextEnum::ChatMessage(message) => {
                serde_json::from_str(&message.content.content_text_only()).unwrap()
            }
            ContextEnum::ContextFile(_) => panic!("expected tool chat message"),
        }
    }

    #[tokio::test]
    async fn handoff_to_task_planner_creates_initial_plan_document() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        facade.snapshot.lock().unwrap().replace(source_snapshot());
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        let (_, messages) = tool
            .tool_execute(
                ccx,
                &"handoff-call".to_string(),
                &handoff_args("Wave 0\n- Card T-1\n- Acceptance Criteria: tests pass"),
            )
            .await
            .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        let document = temp
            .path()
            .join(".refact/tasks")
            .join(&task_meta.task_id)
            .join("documents/initial-plan.md");
        let result = tool_result_json(&messages);
        assert_eq!(result["initial_plan_document"], "initial-plan");
        let raw = tokio::fs::read_to_string(document).await.unwrap();
        assert!(raw.contains("slug: \"initial-plan\""));
        assert!(raw.contains("kind: \"plan\""));
        assert!(raw.contains("pinned: true"));
        assert!(raw.contains("Card T-1"));
    }

    #[tokio::test]
    async fn handoff_to_task_planner_uses_active_root_as_planner_chat_id() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.id = "child-chat".to_string();
        snapshot.thread.root_chat_id = Some("controller-planner".to_string());
        snapshot.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "task-lineage".to_string(),
            role: "subchats".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("stale-planner".to_string()),
        });
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        insert_planner_session(&app, "controller-planner", "task-lineage").await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        tool.tool_execute(
            ccx,
            &"handoff-call".to_string(),
            &handoff_args("Lineage plan"),
        )
        .await
        .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        assert_eq!(saved[0].chat_id, "controller-planner");
        assert_eq!(task_meta.role, "planner");
        assert_eq!(
            task_meta.planner_chat_id.as_deref(),
            Some("controller-planner")
        );
        assert_eq!(saved[0].root_chat_id.as_deref(), Some("controller-planner"));
    }

    #[tokio::test]
    async fn handoff_to_task_planner_appends_to_existing_controller_history() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.id = "child-chat".to_string();
        snapshot.thread.root_chat_id = Some("controller-planner".to_string());
        snapshot.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "task-lineage".to_string(),
            role: "subchats".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("controller-planner".to_string()),
        });
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        insert_planner_session_with_messages(
            &app,
            "controller-planner",
            "task-lineage",
            vec![ChatMessage::new(
                "user".to_string(),
                "Existing planner history".to_string(),
            )],
        )
        .await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        tool.tool_execute(
            ccx,
            &"handoff-call".to_string(),
            &handoff_args("Lineage plan"),
        )
        .await
        .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        assert_eq!(saved[0].chat_id, "controller-planner");
        assert!(saved[0].messages.len() > 1);
        assert_eq!(
            saved[0].messages[0].content.content_text_only(),
            "Existing planner history"
        );
        assert!(saved[0]
            .messages
            .iter()
            .skip(1)
            .any(|message| message.content.content_text_only().contains("Lineage plan")));
    }

    #[tokio::test]
    async fn handoff_to_task_planner_appends_disk_only_controller_history_without_session_update() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        *facade.fail_update_session.lock().unwrap() = true;
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let task = crate::tasks::storage::create_task(app.gcx.clone(), "Lineage Task")
            .await
            .unwrap();
        let planner_chat_id = format!("planner-{}-1", task.id);
        crate::chat::trajectories::save_trajectory_snapshot(
            app.gcx.clone(),
            TrajectorySnapshot {
                goal: None,
                chat_id: planner_chat_id.clone(),
                title: String::new(),
                model: "model".to_string(),
                mode: "task_planner".to_string(),
                tool_use: "agent".to_string(),
                messages: vec![ChatMessage::new(
                    "user".to_string(),
                    "Persisted planner history".to_string(),
                )],
                created_at: chrono::Utc::now().to_rfc3339(),
                boost_reasoning: false,
                checkpoints_enabled: true,
                context_tokens_cap: None,
                include_project_info: true,
                is_title_generated: false,
                auto_approve_editing_tools: false,
                auto_approve_dangerous_commands: false,
                autonomous_no_confirm: false,
                version: 1,
                task_meta: Some(refact_chat_api::TaskMeta {
                    task_id: task.id.clone(),
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some(planner_chat_id.clone()),
                }),
                worktree: None,
                parent_id: None,
                link_type: None,
                root_chat_id: Some(planner_chat_id.clone()),
                reasoning_effort: None,
                thinking_budget: None,
                temperature: None,
                frequency_penalty: None,
                max_tokens: None,
                parallel_tool_calls: None,
                previous_response_id: None,
                active_skill: None,
                auto_enrichment_enabled: None,
                buddy_meta: None,
                auto_compact_enabled: None,
                frozen_request_prefix: None,
                claude_code_identity: None,
                reactive_compact_attempts: None,
                wake_up_at: None,
                waiting_for_card_ids: Vec::new(),
            },
        )
        .await
        .unwrap();

        let mut snapshot = source_snapshot();
        snapshot.thread.id = "child-chat".to_string();
        snapshot.thread.root_chat_id = Some(planner_chat_id.clone());
        snapshot.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: task.id.clone(),
            role: "subchats".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some(planner_chat_id.clone()),
        });
        facade.snapshot.lock().unwrap().replace(snapshot);

        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        tool.tool_execute(
            ccx,
            &"handoff-call".to_string(),
            &handoff_args("Disk lineage plan"),
        )
        .await
        .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        assert_eq!(saved[0].chat_id, planner_chat_id);
        assert!(saved[0].messages.len() > 1);
        assert_eq!(
            saved[0].messages[0].content.content_text_only(),
            "Persisted planner history"
        );
        assert!(saved[0].messages.iter().skip(1).any(|message| message
            .content
            .content_text_only()
            .contains("Disk lineage plan")));
    }

    #[tokio::test]
    async fn handoff_to_task_planner_chat_id_ignores_non_planner_root() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.id = "planner-chat".to_string();
        snapshot.thread.root_chat_id = Some("source-chat".to_string());
        snapshot.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "task-standalone".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("planner-chat".to_string()),
        });
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx_with_chat_id(app, "planner-chat").await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        tool.tool_execute(
            ccx,
            &"handoff-call".to_string(),
            &handoff_args("Standalone plan"),
        )
        .await
        .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        assert_eq!(saved[0].chat_id, "planner-chat");
        assert_eq!(task_meta.planner_chat_id.as_deref(), Some("planner-chat"));
        assert_eq!(saved[0].root_chat_id.as_deref(), Some("planner-chat"));
    }

    #[tokio::test]
    async fn standalone_planner_chat_id_handoff_anchors_root_to_new_planner() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.root_chat_id = Some("normal-source-root".to_string());
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        tool.tool_execute(
            ccx,
            &"handoff-call".to_string(),
            &handoff_args("New standalone planner"),
        )
        .await
        .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        assert_eq!(
            task_meta.planner_chat_id.as_deref(),
            Some(saved[0].chat_id.as_str())
        );
        assert_eq!(
            saved[0].root_chat_id.as_deref(),
            Some(saved[0].chat_id.as_str())
        );
        assert_ne!(saved[0].root_chat_id.as_deref(), Some("normal-source-root"));
    }

    async fn tool_with_snapshot(
        snapshot: ChatSessionSnapshot,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([("target_mode".to_string(), json!("task_planner"))]);
        tool.tool_execute(ccx, &"call-id".to_string(), &args).await
    }

    #[tokio::test]
    async fn handoff_rejects_generating_state() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::Generating;
        let err = tool_with_snapshot(snap).await.unwrap_err();
        assert!(err.contains("model is generating"), "{err}");
    }

    #[tokio::test]
    async fn handoff_allows_executing_tools_state() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::ExecutingTools;
        tool_with_snapshot(snap).await.unwrap();
    }

    #[tokio::test]
    async fn handoff_succeeds_through_real_facade_with_pending_self_call() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;

        let chat_id = "real-exec-chat";
        {
            let mut session = crate::chat::types::ChatSession::new(chat_id.to_string());
            session.thread.mode = "agent".to_string();
            session.thread.tool_use = "agent".to_string();
            session.thread.model = "model".to_string();
            session.add_message(ChatMessage::new(
                "user".to_string(),
                "Please create a plan.".to_string(),
            ));
            session.add_message(ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "handoff-call".to_string(),
                    index: Some(0),
                    function: ChatToolFunction {
                        name: "switch_mode".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            });
            session.set_runtime_state(SessionState::ExecutingTools, None);
            app.gcx.chat_sessions.write().await.insert(
                chat_id.to_string(),
                Arc::new(tokio::sync::Mutex::new(session)),
            );
        }

        let observed = app
            .chat
            .facade
            .session_snapshot(chat_id)
            .await
            .unwrap()
            .session_state;
        assert_eq!(observed, SessionState::ExecutingTools);

        let ccx = handoff_ccx_with_chat_id(app, chat_id).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([("target_mode".to_string(), json!("task_planner"))]);

        let (_, messages) = tool
            .tool_execute(ccx, &"handoff-call".to_string(), &args)
            .await
            .unwrap();
        let result = tool_result_json(&messages);
        assert_eq!(result["target_mode"], "task_planner");
    }

    #[tokio::test]
    async fn handoff_rejects_paused_state() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::Paused;
        let err = tool_with_snapshot(snap).await.unwrap_err();
        assert!(err.contains("session is paused"), "{err}");
    }

    #[tokio::test]
    async fn handoff_rejects_waiting_ide_state() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::WaitingIde;
        let err = tool_with_snapshot(snap).await.unwrap_err();
        assert!(err.contains("waiting for IDE response"), "{err}");
    }

    #[tokio::test]
    async fn handoff_rejects_error_state() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::Error;
        let err = tool_with_snapshot(snap).await.unwrap_err();
        assert!(err.contains("error state"), "{err}");
    }

    #[tokio::test]
    async fn handoff_rejects_waiting_user_input_with_pause_reasons() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::WaitingUserInput;
        snap.pause_reasons = vec![refact_chat_api::PauseReason {
            reason_type: "tool_approval".to_string(),
            tool_name: "shell".to_string(),
            command: "ls".to_string(),
            rule: "default".to_string(),
            tool_call_id: "tc1".to_string(),
            integr_config_path: None,
        }];
        let err = tool_with_snapshot(snap).await.unwrap_err();
        assert!(err.contains("pending tool approvals"), "{err}");
    }

    #[tokio::test]
    async fn handoff_allows_idle_state() {
        let snap = source_snapshot();
        assert_eq!(snap.session_state, SessionState::Idle);
        tool_with_snapshot(snap).await.unwrap();
    }

    #[tokio::test]
    async fn handoff_allows_completed_state() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::Completed;
        tool_with_snapshot(snap).await.unwrap();
    }

    #[tokio::test]
    async fn handoff_allows_waiting_user_input_with_no_pause_reasons() {
        let mut snap = source_snapshot();
        snap.session_state = SessionState::WaitingUserInput;
        snap.pause_reasons = vec![];
        tool_with_snapshot(snap).await.unwrap();
    }

    #[tokio::test]
    async fn handoff_to_mode_does_not_copy_source_frozen_prefix_or_claude_identity() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            system_prompt: Some("source system".to_string()),
            tools_canonical: Some(json!([{"type":"function","function":{"name":"source_tool"}}])),
        });
        snapshot.thread.claude_code_identity = Some(refact_chat_api::ClaudeCodeIdentity {
            device_id: "source-device".to_string(),
            session_id: "source-session".to_string(),
        });
        snapshot.thread.previous_response_id = Some("resp_source".to_string());
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([("target_mode".to_string(), json!("task_planner"))]);

        tool.tool_execute(ccx, &"call-id".to_string(), &args)
            .await
            .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].frozen_request_prefix.is_none());
        assert!(saved[0].claude_code_identity.is_none());
        assert!(saved[0].previous_response_id.is_none());
    }

    #[tokio::test]
    async fn handoff_rejects_unmatched_tool_calls_in_history() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};
        let mut snap = source_snapshot();
        snap.messages = vec![
            ChatMessage::new("user".to_string(), "Please help.".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call-unmatched".to_string(),
                    index: Some(0),
                    function: ChatToolFunction {
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
        ];
        let err = tool_with_snapshot(snap).await.unwrap_err();
        assert!(err.contains("tool calls without results"), "{err}");
    }

    #[tokio::test]
    async fn handoff_accepts_diff_messages_as_tool_call_results() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};
        let mut snap = source_snapshot();
        snap.messages = vec![
            ChatMessage::new("user".to_string(), "Please help.".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call-diff".to_string(),
                    index: Some(0),
                    function: ChatToolFunction {
                        name: "apply_patch".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText("diff accepted".to_string()),
                tool_call_id: "call-diff".to_string(),
                ..Default::default()
            },
        ];
        tool_with_snapshot(snap).await.unwrap();
    }

    #[tokio::test]
    async fn initial_plan_document_failure_does_not_break_handoff() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "missing-task".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("planner-missing-task-1".to_string()),
        });
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        let (_, messages) = tool
            .tool_execute(
                ccx,
                &"handoff-call".to_string(),
                &handoff_args("Wave 0\n- Card T-1\n- Acceptance Criteria: tests pass"),
            )
            .await
            .unwrap();

        assert_eq!(facade.saved.lock().unwrap().len(), 1);
        let result = tool_result_json(&messages);
        assert!(result["initial_plan_document"].is_null());
        assert_eq!(facade.saved.lock().unwrap().len(), 1);
        let result = tool_result_json(&messages);
        assert!(result["initial_plan_document"].is_null());
        assert!(
            result["initial_plan_error"].is_string(),
            "error should surface in output"
        );
    }

    #[tokio::test]
    async fn handoff_with_initial_plan_creates_pinned_document() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        facade.snapshot.lock().unwrap().replace(source_snapshot());
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };

        let (_, messages) = tool
            .tool_execute(
                ccx,
                &"call-id".to_string(),
                &handoff_args("# Wave 0\n- Card T-1"),
            )
            .await
            .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        let document = temp
            .path()
            .join(".refact/tasks")
            .join(&task_meta.task_id)
            .join("documents/initial-plan.md");
        let result = tool_result_json(&messages);
        assert_eq!(result["initial_plan_document"], "initial-plan");
        assert!(result["initial_plan_error"].is_null());
        let raw = tokio::fs::read_to_string(document).await.unwrap();
        assert!(raw.contains("pinned: true"), "{raw}");
        assert!(raw.contains("Card T-1"), "{raw}");
    }

    #[tokio::test]
    async fn handoff_with_only_summary_uses_summary_as_plan_body() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        facade.snapshot.lock().unwrap().replace(source_snapshot());
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([
            ("target_mode".to_string(), json!("task_planner")),
            ("summary".to_string(), json!("Summary plan content here")),
        ]);

        let (_, messages) = tool
            .tool_execute(ccx, &"call-id".to_string(), &args)
            .await
            .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        let document = temp
            .path()
            .join(".refact/tasks")
            .join(&task_meta.task_id)
            .join("documents/initial-plan.md");
        let result = tool_result_json(&messages);
        assert_eq!(result["initial_plan_document"], "initial-plan");
        let raw = tokio::fs::read_to_string(document).await.unwrap();
        assert!(raw.contains("Summary plan content here"), "{raw}");
    }

    #[tokio::test]
    async fn handoff_with_only_handoff_message_uses_it_as_plan_body() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        facade.snapshot.lock().unwrap().replace(source_snapshot());
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([
            ("target_mode".to_string(), json!("task_planner")),
            ("handoff_message".to_string(), json!("Handoff message body")),
        ]);

        let (_, messages) = tool
            .tool_execute(ccx, &"call-id".to_string(), &args)
            .await
            .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        let document = temp
            .path()
            .join(".refact/tasks")
            .join(&task_meta.task_id)
            .join("documents/initial-plan.md");
        let result = tool_result_json(&messages);
        assert_eq!(result["initial_plan_document"], "initial-plan");
        let raw = tokio::fs::read_to_string(document).await.unwrap();
        assert!(raw.contains("Handoff message body"), "{raw}");
    }

    #[tokio::test]
    async fn handoff_with_no_plan_content_creates_stub_document() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        facade.snapshot.lock().unwrap().replace(source_snapshot());
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([("target_mode".to_string(), json!("task_planner"))]);

        let (_, messages) = tool
            .tool_execute(ccx, &"call-id".to_string(), &args)
            .await
            .unwrap();

        let saved = facade.saved.lock().unwrap().clone();
        let task_meta = saved[0].task_meta.as_ref().unwrap();
        let document = temp
            .path()
            .join(".refact/tasks")
            .join(&task_meta.task_id)
            .join("documents/initial-plan.md");
        let result = tool_result_json(&messages);
        assert_eq!(result["initial_plan_document"], "initial-plan");
        let raw = tokio::fs::read_to_string(document).await.unwrap();
        assert!(raw.contains("No plan content provided at handoff"), "{raw}");
    }

    #[tokio::test]
    async fn handoff_surfaces_document_creation_error_prominently() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatFacade::default());
        let mut snapshot = source_snapshot();
        snapshot.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "missing-task".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("planner-missing-task-1".to_string()),
        });
        facade.snapshot.lock().unwrap().replace(snapshot);
        let app = test_app_with_workspace(temp.path(), facade.clone()).await;
        let ccx = handoff_ccx(app).await;
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([
            ("target_mode".to_string(), json!("task_planner")),
            ("initial_plan".to_string(), json!("Some plan content")),
        ]);

        let (_, messages) = tool
            .tool_execute(ccx, &"call-id".to_string(), &args)
            .await
            .unwrap();

        let result = tool_result_json(&messages);
        assert!(result["initial_plan_document"].is_null());
        assert!(
            result["initial_plan_error"].is_string(),
            "error should be surfaced in tool output, got: {:?}",
            result["initial_plan_error"]
        );
    }

    #[tokio::test]
    async fn handoff_accepts_context_file_as_read_tool_completion() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};
        let mut snap = source_snapshot();
        snap.messages = vec![
            ChatMessage::new("user".to_string(), "Read the file.".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call-cat".to_string(),
                    index: Some(0),
                    function: ChatToolFunction {
                        name: "cat".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "context_file".to_string(),
                tool_call_id: "call-cat".to_string(),
                content: ChatContent::SimpleText("file contents".to_string()),
                ..Default::default()
            },
        ];
        tool_with_snapshot(snap).await.unwrap();
    }
}
