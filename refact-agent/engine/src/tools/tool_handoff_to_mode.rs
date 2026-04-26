use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::agentic::mode_transition::{analyze_mode_transition, assemble_new_chat, ParsedDecisions};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::trajectory_ops::sanitize_messages_for_new_thread;
use crate::chat::trajectories::save_trajectory_snapshot;
use crate::chat::types::SessionState;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::postprocessing::pp_command_output::OutputFilter;
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
            description: "Create a new chat in another mode using the current conversation context. Approval required.".to_string(),
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

        let (gcx, chat_id, abort_flag) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.global_context.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.abort_flag.clone(),
            )
        };

        let sessions = gcx.read().await.chat_sessions.clone();
        let session_arc =
            get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;

        let (messages, thread, task_meta, session_state) = {
            let session = session_arc.lock().await;
            (
                session.messages.clone(),
                session.thread.clone(),
                session.thread.task_meta.clone(),
                session.runtime.state,
            )
        };

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

        let mut decisions =
            analyze_mode_transition(gcx.clone(), &messages, &canonical_mode, &mode_description)
                .await
                .map_err(|e| format!("mode transition analysis failed: {}", e))?;

        apply_overrides(&mut decisions, args);

        let new_messages = assemble_new_chat(gcx.clone(), &messages, &decisions)
            .await
            .map_err(|e| format!("handoff assembly failed: {}", e))?;

        let new_messages = sanitize_messages_for_new_thread(&new_messages);
        let new_chat_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let snapshot = crate::chat::trajectories::TrajectorySnapshot {
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
            version: 1,
            task_meta,
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
        };

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .map_err(|e| format!("Failed to save handoff trajectory: {}", e))?;

        abort_flag.store(true, Ordering::SeqCst);

        let result = json!({
            "type": "handoff_to_mode",
            "new_chat_id": new_chat_id,
            "target_mode": canonical_mode,
            "reason": reason,
            "messages_count": new_messages.len(),
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
                tool_call_id: tool_call_id.clone(),
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

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["*".to_string()],
            deny: vec![],
        })
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
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: command_to_match,
            rule: "default".to_string(),
        })
    }
}
