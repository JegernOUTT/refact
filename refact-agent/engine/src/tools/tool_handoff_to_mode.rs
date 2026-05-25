use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::agentic::mode_transition::{AgenticPathContext, ParsedDecisions, assemble_new_chat};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tasks::storage;
use crate::tools::tool_task_documents::{
    create_document_at, documents_dir_for_task, next_available_slug_at,
};
use refact_chat_history::trajectory_ops::sanitize_messages_for_new_thread;
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use refact_runtime_api::SessionState;
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
) -> Result<Option<refact_chat_api::TaskMeta>, String> {
    if canonical_mode != "task_planner" {
        return Ok(existing_task_meta);
    }
    if let Some(task_meta) = existing_task_meta {
        if task_meta.role == "planner" && task_meta.planner_chat_id.is_some() {
            return Ok(Some(task_meta));
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

        if matches!(session_state, SessionState::Generating) {
            return Err("Cannot handoff while generating".to_string());
        }
        if messages.is_empty() {
            return Err("Cannot handoff an empty chat".to_string());
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
        let initial_plan = decisions.initial_plan.clone();

        let path_context = { AgenticPathContext::from_context(&*gcx) };
        let new_messages = assemble_new_chat(&path_context, &messages, &decisions)
            .await
            .map_err(|e| format!("handoff assembly failed: {}", e))?;

        let new_messages = sanitize_messages_for_new_thread(&new_messages);
        let task_meta =
            ensure_task_for_planner_handoff(gcx.clone(), &canonical_mode, existing_task_meta)
                .await?;
        let new_chat_id = task_meta
            .as_ref()
            .and_then(|meta| meta.planner_chat_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = chrono::Utc::now().to_rfc3339();

        let snapshot_task_meta = task_meta.clone();
        let snapshot = TrajectorySnapshot {
            chat_id: new_chat_id.clone(),
            title: String::new(),
            model: thread.model.clone(),
            mode: canonical_mode.clone(),
            tool_use: thread.tool_use.clone(),
            messages: new_messages.clone(),
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
            link_type: Some("mode_transition".to_string()),
            root_chat_id: thread
                .root_chat_id
                .clone()
                .or_else(|| Some(chat_id.clone())),
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
            wake_up_at: None,
        };

        chat_facade
            .save_trajectory_snapshot(snapshot)
            .await
            .map_err(|e| format!("Failed to save handoff trajectory: {}", e))?;

        let initial_plan_doc = if canonical_mode == "task_planner" {
            match (
                task_meta.as_ref().map(|meta| meta.task_id.as_str()),
                initial_plan
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty()),
            ) {
                (Some(task_id), Some(plan_text)) => {
                    let result =
                        create_initial_plan_document(gcx.clone(), task_id, plan_text).await;
                    if let Err(error) = &result {
                        tracing::warn!(
                            "failed to create initial-plan document for task {}: {}",
                            task_id,
                            error
                        );
                    }
                    result.map(Some)
                }
                _ => Ok(None),
            }
        } else {
            Ok(None)
        };
        let initial_plan_doc_slug = initial_plan_doc.clone().ok().flatten();

        let result = json!({
            "type": "handoff_to_mode",
            "new_chat_id": new_chat_id,
            "target_mode": canonical_mode,
            "reason": reason,
            "messages_count": new_messages.len(),
            "task_meta": task_meta,
            "initial_plan_document": initial_plan_doc_slug,
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
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                "source-chat".to_string(),
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
        }
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
    }
}
