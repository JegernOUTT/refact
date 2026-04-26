use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, ContextFile};
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::trajectories::maybe_save_trajectory;
use crate::chat::history_limit::compress_duplicate_context_files;
use crate::chat::history_limit::remove_invalid_tool_calls_and_tool_calls_results;
use crate::chat::trajectory_ops::TOOLS_TO_PRESERVE;
use crate::chat::types::SessionState;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

const TOOL_OUTPUT_TRUNCATE_LIMIT: usize = 200;
const MAX_PER_MESSAGE_ENTRIES: usize = 200;
const MAX_CONTEXT_ENTRIES: usize = 200;
const MAX_TOOL_OUTPUT_ENTRIES: usize = 200;

fn should_preserve_tool(name: &str) -> bool {
    TOOLS_TO_PRESERVE.iter().any(|t| *t == name)
}

fn approx_tokens_for_len(len: usize) -> usize {
    len / 4 + 10
}

fn approx_tokens_for_message(msg: &ChatMessage) -> usize {
    let content_len = match &msg.content {
        ChatContent::SimpleText(text) => text.len(),
        ChatContent::Multimodal(elements) => elements.len() * 100,
        ChatContent::ContextFiles(files) => files.iter().map(|cf| cf.file_content.len()).sum(),
    };
    approx_tokens_for_len(content_len)
}

fn extract_context_files(message: &ChatMessage) -> Vec<ContextFile> {
    match &message.content {
        ChatContent::ContextFiles(files) => files.clone(),
        ChatContent::SimpleText(text) => serde_json::from_str(text).unwrap_or_default(),
        _ => vec![],
    }
}

fn is_memory_path(path: &str) -> bool {
    path.contains("/.refact/knowledge/")
        || path.contains("/.refact/trajectories/")
        || path.contains("/.refact/tasks/")
}

fn parse_bool(args: &HashMap<String, Value>, key: &str) -> bool {
    match args.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

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

pub struct ToolCompressChatProbe {
    pub config_path: String,
}

pub struct ToolCompressChatApply {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCompressChatProbe {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "compress_chat_probe".to_string(),
            display_name: "Compress Chat (Probe)".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Analyze the current chat and report token distribution plus potential compression gains.".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let sessions = gcx.read().await.chat_sessions.clone();
        let session_arc =
            get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;
        let messages = {
            let session = session_arc.lock().await;
            session.messages.clone()
        };

        if messages.is_empty() {
            return Err("Cannot probe an empty chat".to_string());
        }

        let mut role_tokens: HashMap<String, usize> = HashMap::new();
        let mut per_message: Vec<Value> = Vec::new();
        let mut total_tokens = 0usize;

        let mut context_occurrences: HashMap<String, usize> = HashMap::new();
        let mut context_token_map: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, msg) in messages.iter().enumerate() {
            let content_len = match &msg.content {
                ChatContent::SimpleText(text) => text.len(),
                ChatContent::Multimodal(elements) => elements.len() * 100,
                ChatContent::ContextFiles(files) => {
                    files.iter().map(|cf| cf.file_content.len()).sum()
                }
            };
            let tokens = approx_tokens_for_len(content_len);
            total_tokens += tokens;
            *role_tokens.entry(msg.role.clone()).or_insert(0) += tokens;
            per_message.push(json!({
                "index": idx,
                "role": msg.role,
                "tokens": tokens,
                "chars": content_len,
            }));

            if msg.role == "context_file" {
                for cf in extract_context_files(msg) {
                    *context_occurrences.entry(cf.file_name.clone()).or_insert(0) += 1;
                    context_token_map
                        .entry(cf.file_name.clone())
                        .or_default()
                        .push(approx_tokens_for_len(cf.file_content.len()));
                }
            }
        }

        let mut context_files: Vec<Value> = Vec::new();
        let mut memory_tokens = 0usize;
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role != "context_file" {
                continue;
            }
            for cf in extract_context_files(msg) {
                let tokens = approx_tokens_for_len(cf.file_content.len());
                let is_memory = is_memory_path(&cf.file_name);
                if is_memory {
                    memory_tokens += tokens;
                }
                let occurrences = context_occurrences.get(&cf.file_name).copied().unwrap_or(1);
                let file_name = cf.file_name.clone();
                context_files.push(json!({
                    "index": idx,
                    "file_name": file_name,
                    "tokens": tokens,
                    "chars": cf.file_content.len(),
                    "is_memory": is_memory,
                    "occurrences": occurrences,
                }));
            }
        }

        let mut tool_call_names: HashMap<String, String> = HashMap::new();
        for msg in &messages {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    tool_call_names.insert(tc.id.clone(), tc.function.name.clone());
                }
            }
        }

        let mut tool_outputs: Vec<Value> = Vec::new();
        let mut tool_output_tokens = 0usize;
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role != "tool" && msg.role != "diff" {
                continue;
            }
            let tokens = approx_tokens_for_message(msg);
            let tool_name = tool_call_names
                .get(&msg.tool_call_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            if !should_preserve_tool(&tool_name) {
                tool_output_tokens += tokens;
            }
            tool_outputs.push(json!({
                "index": idx,
                "tool_call_id": msg.tool_call_id,
                "tool_name": tool_name,
                "role": msg.role,
                "tokens": tokens,
                "chars": msg.content.content_text_only().len(),
            }));
        }

        let mut context_messages: Vec<Value> = Vec::new();
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role != "context_file" {
                continue;
            }
            context_messages.push(json!({
                "index": idx,
                "tool_call_id": msg.tool_call_id,
                "tokens": approx_tokens_for_message(msg),
                "chars": msg.content.content_text_only().len(),
            }));
        }

        let mut per_message_truncated = false;
        if per_message.len() > MAX_PER_MESSAGE_ENTRIES {
            let head = MAX_PER_MESSAGE_ENTRIES / 2;
            let tail = MAX_PER_MESSAGE_ENTRIES - head;
            let mut trimmed = Vec::with_capacity(MAX_PER_MESSAGE_ENTRIES);
            trimmed.extend_from_slice(&per_message[..head]);
            trimmed.extend_from_slice(&per_message[per_message.len().saturating_sub(tail)..]);
            per_message = trimmed;
            per_message_truncated = true;
        }

        let mut context_messages_truncated = false;
        if context_messages.len() > MAX_CONTEXT_ENTRIES {
            let head = MAX_CONTEXT_ENTRIES / 2;
            let tail = MAX_CONTEXT_ENTRIES - head;
            let mut trimmed = Vec::with_capacity(MAX_CONTEXT_ENTRIES);
            trimmed.extend_from_slice(&context_messages[..head]);
            trimmed.extend_from_slice(
                &context_messages[context_messages.len().saturating_sub(tail)..],
            );
            context_messages = trimmed;
            context_messages_truncated = true;
        }

        let mut context_files_truncated = false;
        if context_files.len() > MAX_CONTEXT_ENTRIES {
            context_files.sort_by_key(|v| v.get("tokens").and_then(|x| x.as_u64()).unwrap_or(0));
            context_files.reverse();
            context_files.truncate(MAX_CONTEXT_ENTRIES);
            context_files_truncated = true;
        }

        let mut tool_outputs_truncated = false;
        if tool_outputs.len() > MAX_TOOL_OUTPUT_ENTRIES {
            tool_outputs.sort_by_key(|v| v.get("tokens").and_then(|x| x.as_u64()).unwrap_or(0));
            tool_outputs.reverse();
            tool_outputs.truncate(MAX_TOOL_OUTPUT_ENTRIES);
            tool_outputs_truncated = true;
        }

        let mut duplicate_context_tokens = 0usize;
        for tokens in context_token_map.values() {
            if tokens.len() > 1 {
                let max_val = tokens.iter().copied().max().unwrap_or(0);
                let total: usize = tokens.iter().sum();
                duplicate_context_tokens += total.saturating_sub(max_val);
            }
        }

        let mut project_info_tokens = 0usize;
        let first_system_idx = messages.iter().position(|m| m.role == "system");
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == "system" && Some(idx) != first_system_idx {
                let text = msg.content.content_text_only().to_lowercase();
                if text.contains("project") || text.contains("workspace") {
                    project_info_tokens += approx_tokens_for_message(msg);
                }
            }
        }

        let role_tokens_json = serde_json::to_value(&role_tokens).unwrap_or_else(|_| json!({}));

        let result = json!({
            "type": "compress_chat_probe",
            "messages_count": messages.len(),
            "total_tokens": total_tokens,
            "role_tokens": role_tokens_json,
            "per_message": per_message,
            "context_files": context_files,
            "context_messages": context_messages,
            "tool_outputs": tool_outputs,
            "per_message_truncated": per_message_truncated,
            "context_files_truncated": context_files_truncated,
            "context_messages_truncated": context_messages_truncated,
            "tool_outputs_truncated": tool_outputs_truncated,
            "potential_gains": {
                "duplicate_context_tokens": duplicate_context_tokens,
                "tool_output_tokens": tool_output_tokens,
                "memory_tokens": memory_tokens,
                "project_info_tokens": project_info_tokens,
            }
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()),
                ),
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        None
    }
}

#[async_trait]
impl Tool for ToolCompressChatApply {
    fn tool_description(&self) -> ToolDesc {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "drop_context_files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of context file names to drop entirely"
                },
                "drop_memories": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Memory/knowledge file paths to drop"
                },
                "drop_context_messages": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Context-file message tool_call_id values to drop entirely"
                },
                "drop_all_memories": {
                    "type": "boolean",
                    "description": "Drop all memory/knowledge context files"
                },
                "truncate_tool_outputs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Tool call IDs to truncate"
                },
                "drop_tool_outputs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Tool call IDs to drop (replaced with a short placeholder)"
                },
                "dedup_context_files": {
                    "type": "boolean",
                    "description": "Deduplicate repeated context files"
                },
                "drop_project_information": {
                    "type": "boolean",
                    "description": "Drop system/project info messages"
                }
            },
            "required": []
        });

        ToolDesc {
            name: "compress_chat_apply".to_string(),
            display_name: "Compress Chat (Apply)".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Apply selective compression to the current chat using explicit drop/truncate lists.".to_string(),
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
        let drop_context_files = parse_string_list(args, "drop_context_files");
        let drop_memories = parse_string_list(args, "drop_memories");
        let drop_all_memories = parse_bool(args, "drop_all_memories");
        let truncate_tool_outputs = parse_string_list(args, "truncate_tool_outputs");
        let drop_tool_outputs = parse_string_list(args, "drop_tool_outputs");
        let drop_context_messages = parse_string_list(args, "drop_context_messages");
        let dedup_context_files = parse_bool(args, "dedup_context_files");
        let drop_project_information = parse_bool(args, "drop_project_information");

        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let sessions = gcx.read().await.chat_sessions.clone();
        let session_arc =
            get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;

        let (
            before_tokens,
            before_count,
            active_start,
            mut head_messages,
            tail_messages,
            tool_call_names,
        ) = {
            let session = session_arc.lock().await;

            if matches!(session.runtime.state, SessionState::Generating) {
                return Err("Cannot compress while generating".to_string());
            }

            let before_tokens = session
                .messages
                .iter()
                .map(approx_tokens_for_message)
                .sum::<usize>();
            let before_count = session.messages.len();
            let active_start = session
                .messages
                .iter()
                .rposition(|m| {
                    m.role == "assistant"
                        && m.tool_calls
                            .as_ref()
                            .map(|tcs| tcs.iter().any(|tc| tc.id == *tool_call_id))
                            .unwrap_or(false)
                })
                .unwrap_or(session.messages.len());

            if active_start >= session.messages.len() {
                return Err("Active tool call not found in session".to_string());
            }

            let tool_call_names: HashMap<String, String> = session
                .messages
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .flatten()
                .map(|tc| (tc.id.clone(), tc.function.name.clone()))
                .collect();

            (
                before_tokens,
                before_count,
                active_start,
                session.messages[..active_start].to_vec(),
                session.messages[active_start..].to_vec(),
                tool_call_names,
            )
        };

        let drop_context_files: HashSet<String> = drop_context_files.into_iter().collect();
        let drop_memories: HashSet<String> = drop_memories.into_iter().collect();
        let drop_context_messages: HashSet<String> = drop_context_messages.into_iter().collect();
        let truncate_tool_outputs: HashSet<String> = truncate_tool_outputs.into_iter().collect();
        let drop_tool_outputs: HashSet<String> = drop_tool_outputs.into_iter().collect();

        let mut context_files_dropped = 0usize;
        let mut context_messages_dropped = 0usize;
        let mut memory_dropped = 0usize;
        let mut tool_truncated = 0usize;
        let mut tool_dropped = 0usize;
        let mut project_info_dropped = 0usize;
        let mut dedup_count = 0usize;

        if drop_project_information {
            let first_system_idx = head_messages.iter().position(|m| m.role == "system");
            let mut idx = 0usize;
            head_messages.retain(|msg| {
                let keep = if msg.role != "system" {
                    true
                } else if Some(idx) == first_system_idx {
                    true
                } else {
                    let text = msg.content.content_text_only().to_lowercase();
                    if text.contains("project") || text.contains("workspace") {
                        project_info_dropped += 1;
                        false
                    } else {
                        true
                    }
                };
                idx += 1;
                keep
            });
        }

        // Modify context files
        let mut updated_head: Vec<ChatMessage> = Vec::with_capacity(head_messages.len());
        for msg in head_messages.into_iter() {
            if msg.role != "context_file" {
                updated_head.push(msg);
                continue;
            }
            if !msg.tool_call_id.is_empty() && drop_context_messages.contains(&msg.tool_call_id) {
                context_messages_dropped += 1;
                continue;
            }

            let mut files = extract_context_files(&msg);
            if files.is_empty() {
                updated_head.push(msg);
                continue;
            }

            let mut remaining: Vec<ContextFile> = Vec::new();
            for cf in files.drain(..) {
                let is_memory = is_memory_path(&cf.file_name);
                if drop_context_files.contains(&cf.file_name) {
                    context_files_dropped += 1;
                    continue;
                }
                if drop_all_memories && is_memory {
                    memory_dropped += 1;
                    continue;
                }
                if drop_memories.contains(&cf.file_name) {
                    memory_dropped += 1;
                    continue;
                }
                remaining.push(cf);
            }

            if remaining.is_empty() {
                context_messages_dropped += 1;
                continue;
            }

            let mut new_msg = msg.clone();
            new_msg.content = ChatContent::ContextFiles(remaining);
            updated_head.push(new_msg);
        }

        head_messages = updated_head;

        if dedup_context_files {
            if let Ok((count, _)) = compress_duplicate_context_files(&mut head_messages) {
                dedup_count = count;
            }
        }

        // Modify tool outputs
        for msg in head_messages.iter_mut() {
            if msg.role != "tool" && msg.role != "diff" {
                continue;
            }
            if msg.tool_call_id.is_empty() {
                continue;
            }
            if drop_tool_outputs.contains(&msg.tool_call_id) {
                msg.content = ChatContent::SimpleText(
                    "Tool result removed by compress_chat_apply".to_string(),
                );
                tool_dropped += 1;
                continue;
            }
            if truncate_tool_outputs.contains(&msg.tool_call_id) {
                if let Some(name) = tool_call_names.get(&msg.tool_call_id) {
                    if should_preserve_tool(name) {
                        continue;
                    }
                }
                let content = msg.content.content_text_only();
                if content.len() > TOOL_OUTPUT_TRUNCATE_LIMIT {
                    let preview: String =
                        content.chars().take(TOOL_OUTPUT_TRUNCATE_LIMIT).collect();
                    msg.content =
                        ChatContent::SimpleText(format!("Tool result compressed: {}...", preview));
                    tool_truncated += 1;
                }
            }
        }

        head_messages.extend(tail_messages);
        let active_call_id = tool_call_id.clone();
        let active_msg = head_messages
            .iter()
            .enumerate()
            .find(|(_, msg)| {
                msg.role == "assistant"
                    && msg
                        .tool_calls
                        .as_ref()
                        .map(|tcs| tcs.iter().any(|tc| tc.id == active_call_id))
                        .unwrap_or(false)
            })
            .map(|(idx, msg)| (idx, msg.clone()));

        remove_invalid_tool_calls_and_tool_calls_results(&mut head_messages);

        if let Some((active_idx, active_msg)) = active_msg {
            let still_present = head_messages.iter().any(|msg| {
                msg.role == "assistant"
                    && msg
                        .tool_calls
                        .as_ref()
                        .map(|tcs| tcs.iter().any(|tc| tc.id == active_call_id))
                        .unwrap_or(false)
            });
            if !still_present {
                head_messages.insert(active_idx.min(head_messages.len()), active_msg);
            }
        }

        let after_tokens = head_messages
            .iter()
            .map(approx_tokens_for_message)
            .sum::<usize>();
        let after_count = head_messages.len();

        if head_messages.first().map(|m| m.role.as_str()).unwrap_or("") != "system"
            && head_messages.first().map(|m| m.role.as_str()).unwrap_or("") != "user"
        {
            return Err(format!(
                "compress_chat_apply would produce an invalid chat history: first message has role '{}', expected 'system' or 'user'. Compression aborted.",
                head_messages.first().map(|m| m.role.as_str()).unwrap_or("(empty)")
            ));
        }

        {
            let mut session = session_arc.lock().await;
            session.messages = head_messages;
            session.increment_version();
            let snapshot = session.snapshot();
            session.emit(snapshot);
        }

        maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

        let result = json!({
            "type": "compress_chat_apply",
            "before_message_count": before_count,
            "after_message_count": after_count,
            "before_tokens": before_tokens,
            "after_tokens": after_tokens,
            "context_files_dropped": context_files_dropped,
            "context_messages_dropped": context_messages_dropped,
            "memories_dropped": memory_dropped,
            "tool_outputs_truncated": tool_truncated,
            "tool_outputs_dropped": tool_dropped,
            "project_info_dropped": project_info_dropped,
            "dedup_context_files": dedup_count,
            "active_tail_start": active_start,
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

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        None
    }
}
