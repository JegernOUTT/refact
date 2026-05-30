use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use refact_core::chat_types::{ChatContent, ChatMessage, ContextFile};

const UI_ONLY_MARKER: &str = "_ui_only";

pub fn is_ui_only_message(msg: &ChatMessage) -> bool {
    msg.extra.get(UI_ONLY_MARKER).and_then(|v| v.as_bool()) == Some(true)
}

pub fn sanitize_message_for_new_thread(m: &ChatMessage) -> ChatMessage {
    let extra = if is_ui_only_message(m) {
        m.extra.clone()
    } else {
        preserve_hidden_role_extra(m)
    };

    ChatMessage {
        message_id: m.message_id.clone(),
        role: m.role.clone(),
        content: m.content.clone(),
        tool_calls: m.tool_calls.clone(),
        tool_call_id: m.tool_call_id.clone(),
        tool_failed: m.tool_failed,
        preserve: m.preserve,
        finish_reason: None,
        reasoning_content: None,
        usage: None,
        checkpoints: vec![],
        thinking_blocks: None,
        citations: vec![],
        server_content_blocks: vec![],
        summarized_range: m.summarized_range,
        summarization_tier: m.summarization_tier.clone(),
        summarized_token_estimate: m.summarized_token_estimate,
        extra,
        output_filter: None,
    }
}

fn preserve_hidden_role_extra(msg: &ChatMessage) -> serde_json::Map<String, serde_json::Value> {
    match msg.role.as_str() {
        "plan" => preserve_extra_key(&msg.extra, "plan"),
        "event" => preserve_extra_key(&msg.extra, "event"),
        COMPRESSION_REPORT_ROLE => preserve_extra_key(&msg.extra, "compression_report"),
        _ => serde_json::Map::new(),
    }
}

fn preserve_extra_key(
    extra: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> serde_json::Map<String, serde_json::Value> {
    extra
        .get(key)
        .map(|value| serde_json::Map::from_iter([(key.to_string(), value.clone())]))
        .unwrap_or_default()
}

pub fn sanitize_messages_for_new_thread(msgs: &[ChatMessage]) -> Vec<ChatMessage> {
    msgs.iter()
        .filter(|msg| !is_ui_only_message(msg))
        .map(sanitize_message_for_new_thread)
        .collect()
}

fn is_valid_tool_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn generate_valid_tool_id() -> String {
    format!(
        "call_{}",
        Uuid::new_v4().to_string().replace("-", "")[..24].to_string()
    )
}

pub fn sanitize_messages_for_model_switch(msgs: &mut Vec<ChatMessage>) {
    msgs.retain(|msg| !is_ui_only_message(msg));

    for msg in msgs.iter_mut() {
        msg.thinking_blocks = None;
        msg.server_content_blocks = Vec::new();
    }

    let mut id_mapping: HashMap<String, String> = HashMap::new();

    for msg in msgs.iter() {
        if let Some(tool_calls) = &msg.tool_calls {
            for tc in tool_calls {
                if !is_valid_tool_id(&tc.id) && !id_mapping.contains_key(&tc.id) {
                    id_mapping.insert(tc.id.clone(), generate_valid_tool_id());
                }
            }
        }
        if !msg.tool_call_id.is_empty()
            && !is_valid_tool_id(&msg.tool_call_id)
            && !id_mapping.contains_key(&msg.tool_call_id)
        {
            id_mapping.insert(msg.tool_call_id.clone(), generate_valid_tool_id());
        }
    }

    for msg in msgs.iter_mut() {
        msg.usage = None;
        msg.extra = preserve_hidden_role_extra(msg);
        msg.finish_reason = None;
        msg.reasoning_content = None;

        if let Some(tool_calls) = &mut msg.tool_calls {
            for tc in tool_calls.iter_mut() {
                if let Some(new_id) = id_mapping.get(&tc.id) {
                    tc.id = new_id.clone();
                }
            }
        }
        if let Some(new_id) = id_mapping.get(&msg.tool_call_id) {
            msg.tool_call_id = new_id.clone();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompressOptions {
    #[serde(default)]
    pub dedup_and_compress_context: bool,
    #[serde(default)]
    pub drop_all_context: bool,
    #[serde(default)]
    pub compress_non_agentic_tools: bool,
    #[serde(default)]
    pub drop_all_memories: bool,
    #[serde(default)]
    pub drop_project_information: bool,
    #[serde(default)]
    pub strip_metering: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HandoffOptions {
    #[serde(default)]
    pub include_last_user_plus: bool,
    #[serde(default)]
    pub include_all_opened_context: bool,
    #[serde(default)]
    pub include_all_edited_context: bool,
    #[serde(default)]
    pub include_agentic_tools: bool,
    #[serde(default)]
    pub llm_summary_for_excluded: bool,
    #[serde(default)]
    pub include_all_user_assistant_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransformStats {
    pub before_message_count: usize,
    pub after_message_count: usize,
    pub before_approx_tokens: usize,
    pub after_approx_tokens: usize,
    pub context_messages_modified: usize,
    pub tool_messages_modified: usize,
}

pub const COMPRESSION_REPORT_ROLE: &str = "compression_report";
pub const COMPRESSION_REPORT_KIND: &str = "chat_compression_report";

pub fn build_compression_report_message(
    context_files_removed: usize,
    context_messages_dropped: usize,
    tool_results_truncated: usize,
    tokens_before: usize,
    tokens_after: usize,
) -> ChatMessage {
    let estimated_tokens_saved = tokens_before.saturating_sub(tokens_after);
    let reduction_percent = if tokens_before > 0 {
        (estimated_tokens_saved * 100) / tokens_before
    } else {
        0
    };
    let mut extra = serde_json::Map::new();
    extra.insert(
        "compression_report".to_string(),
        serde_json::json!({
            "kind": COMPRESSION_REPORT_KIND,
            "context_files_removed": context_files_removed,
            "context_messages_dropped": context_messages_dropped,
            "tool_results_truncated": tool_results_truncated,
            "tokens_before": tokens_before,
            "tokens_after": tokens_after,
            "estimated_tokens_saved": estimated_tokens_saved,
            "reduction_percent": reduction_percent,
        }),
    );
    ChatMessage {
        role: COMPRESSION_REPORT_ROLE.to_string(),
        content: ChatContent::SimpleText(format!(
            "## Chat compression report\n\n- Context files removed: {}\n- Context messages dropped: {}\n- Tool outputs truncated: {}\n- Tokens before: {}\n- Tokens after: {}\n- Estimated tokens saved: {}\n- Reduction: {}%",
            context_files_removed,
            context_messages_dropped,
            tool_results_truncated,
            tokens_before,
            tokens_after,
            estimated_tokens_saved,
            reduction_percent
        )),
        summarization_tier: Some("tier2_reactive".to_string()),
        summarized_token_estimate: Some(estimated_tokens_saved),
        extra,
        ..Default::default()
    }
}

fn compression_report_insert_index(messages: &[ChatMessage], affected_boundary: usize) -> usize {
    let mut insert_idx = affected_boundary.min(messages.len());
    let leading_system_prefix_len = messages
        .iter()
        .take_while(|message| message.role == "system")
        .count();
    insert_idx = insert_idx.max(leading_system_prefix_len);

    if let Some(first_user_idx) = messages.iter().position(|message| message.role == "user") {
        insert_idx = insert_idx.max(first_user_idx + 1);
    } else if insert_idx == 0 {
        if let Some(first_anchor_idx) = messages
            .iter()
            .position(|message| matches!(message.role.as_str(), "system" | "event" | "plan"))
        {
            insert_idx = first_anchor_idx + 1;
        }
    }

    insert_idx.min(messages.len())
}

fn note_affected_boundary(boundary: &mut Option<usize>, candidate: usize) {
    *boundary = Some(boundary.map_or(candidate, |current| current.min(candidate)));
}

fn first_changed_boundary(before: &[ChatMessage], after: &[ChatMessage]) -> Option<usize> {
    let common_len = before.len().min(after.len());
    for idx in 0..common_len {
        if serde_json::to_value(&before[idx]).ok() != serde_json::to_value(&after[idx]).ok() {
            return Some(idx);
        }
    }
    (before.len() != after.len()).then_some(common_len)
}

pub const TOOLS_TO_PRESERVE: &[&str] = &["research", "delegate", "plan", "review"];

fn should_preserve_tool(name: &str) -> bool {
    TOOLS_TO_PRESERVE.iter().any(|t| *t == name)
}

fn should_preserve_message(msg: &ChatMessage, tool_call_names: &HashMap<String, String>) -> bool {
    msg.preserve == Some(true)
        || tool_call_names
            .get(&msg.tool_call_id)
            .map_or(false, |name| should_preserve_tool(name))
}

fn normalize_path_text(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

fn memory_path_marker_present(text: &str) -> bool {
    let normalized = normalize_path_text(text);
    normalized.contains(".refact/knowledge/")
        || normalized.contains(".refact/trajectories/")
        || normalized.contains(".refact/tasks/")
        || normalized.ends_with(".refact/knowledge")
        || normalized.ends_with(".refact/trajectories")
        || normalized.ends_with(".refact/tasks")
}

fn is_memory_path(path: &str) -> bool {
    let normalized = normalize_path_text(path);
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    parts.windows(2).any(|parts| {
        parts[0] == ".refact" && matches!(parts[1], "knowledge" | "trajectories" | "tasks")
    })
}

fn filter_memory_context_files(files: &[ContextFile]) -> (Vec<ContextFile>, usize) {
    let remaining: Vec<_> = files
        .iter()
        .filter(|cf| !is_memory_path(&cf.file_name))
        .cloned()
        .collect();
    let removed = files.len() - remaining.len();
    (remaining, removed)
}

fn simple_text_contains_memory_context_path(text: &str) -> bool {
    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
        return files.iter().any(|cf| is_memory_path(&cf.file_name));
    }

    text.lines().any(|line| {
        let trimmed = line.trim();
        let normalized = normalize_path_text(trimmed);
        let has_context_path_label = normalized.contains("file_name")
            || normalized.starts_with("FILE ")
            || normalized.starts_with("file:")
            || normalized.starts_with("path:")
            || normalized.starts_with("- file:")
            || normalized.starts_with("- path:");
        has_context_path_label && memory_path_marker_present(&normalized)
    })
}

pub fn handoff_conversation_and_excluded(
    messages: &[ChatMessage],
    opts: &HandoffOptions,
    system_prefix_len: usize,
    start_idx: usize,
    edited_tool_ids: &HashSet<String>,
) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
    let mut conversation: Vec<ChatMessage> = Vec::new();
    let mut selected_indices: HashSet<usize> = HashSet::new();

    for (i, msg) in messages.iter().enumerate().skip(system_prefix_len) {
        let should_include = if opts.include_all_user_assistant_only {
            matches!(msg.role.as_str(), "user" | "assistant")
        } else {
            match msg.role.as_str() {
                "user" => i >= start_idx,
                "assistant" => {
                    if i >= start_idx {
                        if let Some(ref tool_calls) = msg.tool_calls {
                            let has_non_preserved = tool_calls.iter().any(|tc| {
                                !should_preserve_tool(&tc.function.name)
                                    && !edited_tool_ids.contains(&tc.id)
                            });
                            has_non_preserved || tool_calls.is_empty()
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                }
                "system" => false,
                "context_file" => false,
                "diff" => false,
                "tool" => false,
                _ => i >= start_idx,
            }
        };

        if should_include {
            selected_indices.insert(i);
            if opts.include_all_user_assistant_only && msg.role == "assistant" {
                let mut clean_msg = msg.clone();
                clean_msg.tool_calls = None;
                clean_msg.tool_call_id = String::new();
                clean_msg.tool_failed = None;
                conversation.push(clean_msg);
            } else {
                conversation.push(msg.clone());
            }
        }
    }

    let excluded = messages
        .iter()
        .enumerate()
        .skip(system_prefix_len)
        .filter(|(idx, _)| !selected_indices.contains(idx))
        .map(|(_, msg)| msg.clone())
        .collect();

    (conversation, excluded)
}

pub fn approx_token_count(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_len = match &m.content {
                ChatContent::SimpleText(s) => s.len(),
                ChatContent::Multimodal(v) => v.iter().map(|_| 100).sum(),
                ChatContent::ContextFiles(v) => v.iter().map(|cf| cf.file_content.len()).sum(),
            };
            content_len / 4 + 10
        })
        .sum()
}

pub fn compress_in_place(
    messages: &mut Vec<ChatMessage>,
    opts: &CompressOptions,
) -> Result<TransformStats, String> {
    let before_count = messages.len();
    let before_tokens = approx_token_count(messages);
    let mut context_modified = 0;
    let mut context_messages_dropped = 0;
    let mut tool_modified = 0;
    let mut affected_boundary = None;

    if opts.drop_all_context {
        let mut kept_before = 0usize;
        messages.retain(|m| {
            if m.role == "context_file" {
                context_modified += 1;
                context_messages_dropped += 1;
                note_affected_boundary(&mut affected_boundary, kept_before);
                false
            } else {
                kept_before += 1;
                true
            }
        });
    } else if opts.dedup_and_compress_context {
        let before_dedup = messages.clone();
        let result = crate::history_limit::compress_duplicate_context_files(messages);
        if let Ok((count, _)) = result {
            context_modified = count;
            if count > 0 {
                if let Some(boundary) = first_changed_boundary(&before_dedup, messages) {
                    note_affected_boundary(&mut affected_boundary, boundary);
                }
            }
        }
    }

    if opts.drop_all_memories {
        for (idx, msg) in messages.iter_mut().enumerate() {
            if msg.role != "context_file" {
                continue;
            }
            match &msg.content {
                ChatContent::ContextFiles(files) => {
                    let (remaining, removed) = filter_memory_context_files(files);
                    if removed > 0 {
                        context_modified += removed;
                        note_affected_boundary(&mut affected_boundary, idx);
                        msg.content = ChatContent::ContextFiles(remaining);
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
                        let (remaining, removed) = filter_memory_context_files(&files);
                        if removed > 0 {
                            context_modified += removed;
                            note_affected_boundary(&mut affected_boundary, idx);
                            msg.content = ChatContent::SimpleText(
                                serde_json::to_string(&remaining).map_err(|e| {
                                    format!("Failed to serialize context files: {}", e)
                                })?,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        let mut kept_before = 0usize;
        messages.retain(|m| {
            if m.role != "context_file" {
                kept_before += 1;
                return true;
            }
            match &m.content {
                ChatContent::ContextFiles(files) => {
                    if files.is_empty() {
                        context_messages_dropped += 1;
                        note_affected_boundary(&mut affected_boundary, kept_before);
                        false
                    } else {
                        kept_before += 1;
                        true
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
                        if files.is_empty() {
                            context_messages_dropped += 1;
                            note_affected_boundary(&mut affected_boundary, kept_before);
                            false
                        } else {
                            kept_before += 1;
                            true
                        }
                    } else if simple_text_contains_memory_context_path(text) {
                        context_modified += 1;
                        context_messages_dropped += 1;
                        note_affected_boundary(&mut affected_boundary, kept_before);
                        false
                    } else {
                        kept_before += 1;
                        true
                    }
                }
                _ => {
                    kept_before += 1;
                    true
                }
            }
        });
    }
    if opts.drop_project_information {
        let first_system_idx = messages.iter().position(|m| m.role == "system");
        let mut idx = 0usize;
        let mut kept_before = 0usize;
        messages.retain(|msg| {
            let keep = if msg.role != "system" {
                true
            } else if Some(idx) == first_system_idx {
                true
            } else {
                let text = msg.content.content_text_only().to_lowercase();
                if text.contains("project") || text.contains("workspace") {
                    context_modified += 1;
                    note_affected_boundary(&mut affected_boundary, kept_before);
                    false
                } else {
                    true
                }
            };
            idx += 1;
            if keep {
                kept_before += 1;
            }
            keep
        });
    }

    if opts.compress_non_agentic_tools {
        let tool_call_names: std::collections::HashMap<String, String> = messages
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .flatten()
            .map(|tc| (tc.id.clone(), tc.function.name.clone()))
            .collect();

        for (idx, msg) in messages.iter_mut().enumerate() {
            if msg.role == "tool" && !msg.tool_call_id.is_empty() {
                if should_preserve_message(msg, &tool_call_names) {
                    continue;
                }
                let content_text = msg.content.content_text_only();
                if content_text.len() > 500 {
                    let preview: String = content_text.chars().take(200).collect();
                    msg.content =
                        ChatContent::SimpleText(format!("Tool result compressed: {}...", preview));
                    tool_modified += 1;
                    note_affected_boundary(&mut affected_boundary, idx);
                }
            }
        }
    }

    crate::history_limit::remove_invalid_tool_calls_and_tool_calls_results(messages);

    if opts.strip_metering {
        messages.retain(|msg| !is_ui_only_message(msg));
        for msg in messages.iter_mut() {
            msg.usage = None;
            msg.extra = preserve_hidden_role_extra(msg);
        }
    }

    let after_tokens_pre = approx_token_count(messages);

    if let Some(boundary) = affected_boundary {
        let report = build_compression_report_message(
            context_modified,
            context_messages_dropped,
            tool_modified,
            before_tokens,
            after_tokens_pre,
        );
        let insert_idx = compression_report_insert_index(messages, boundary);
        messages.insert(insert_idx, report);
    }

    let after_tokens = approx_token_count(messages);
    Ok(TransformStats {
        before_message_count: before_count,
        after_message_count: messages.len(),
        before_approx_tokens: before_tokens,
        after_approx_tokens: after_tokens,
        context_messages_modified: context_modified,
        tool_messages_modified: tool_modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{ChatToolCall, ChatToolFunction, ChatUsage};

    fn make_user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_assistant_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_tool_msg(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_context_file(filename: &str, content: &str) -> ContextFile {
        ContextFile {
            file_name: filename.to_string(),
            file_content: content.to_string(),
            line1: 1,
            line2: 100,
            file_rev: None,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 0.0,
            skip_pp: false,
        }
    }

    fn make_context_file_msg(filename: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![make_context_file(filename, content)]),
            ..Default::default()
        }
    }

    fn make_context_file_simple_text_msg(files: Vec<ContextFile>) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText(serde_json::to_string(&files).unwrap()),
            ..Default::default()
        }
    }

    fn with_message_id(mut message: ChatMessage, message_id: &str) -> ChatMessage {
        message.message_id = message_id.to_string();
        message
    }

    fn make_assistant_with_tool_call(tool_call_id: &str, tool_name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: tool_name.to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn make_system_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_ui_only_msg(content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(UI_ONLY_MARKER.to_string(), serde_json::Value::Bool(true));
        ChatMessage {
            role: "error".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn make_plan_msg() -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "plan".to_string(),
            serde_json::json!({
                "mode": "agent",
                "version": 1,
                "created_at_ms": 123,
                "supersedes": null,
            }),
        );
        extra.insert("unrelated".to_string(), serde_json::json!("strip me"));
        ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText("base plan".to_string()),
            preserve: Some(true),
            extra,
            ..Default::default()
        }
    }

    fn make_plan_delta_event() -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "plan_delta",
                "source": "tool.set_plan",
                "payload": {"seq": 1},
            }),
        );
        extra.insert("unrelated".to_string(), serde_json::json!("strip me"));
        ChatMessage {
            role: "event".to_string(),
            content: ChatContent::SimpleText("delta".to_string()),
            extra,
            ..Default::default()
        }
    }

    fn assert_only_hidden_plan_extra(message: &ChatMessage) {
        assert_eq!(message.role, "plan");
        assert_eq!(message.extra["plan"]["version"], serde_json::json!(1));
        assert_eq!(message.extra.len(), 1);
        assert!(!message.extra.contains_key("unrelated"));
    }

    fn assert_only_hidden_event_extra(message: &ChatMessage) {
        assert_eq!(message.role, "event");
        assert_eq!(
            message.extra["event"]["subkind"],
            serde_json::json!("plan_delta")
        );
        assert_eq!(message.extra.len(), 1);
        assert!(!message.extra.contains_key("unrelated"));
    }

    fn compression_report(messages: &[ChatMessage]) -> &ChatMessage {
        messages
            .iter()
            .find(|msg| msg.role == COMPRESSION_REPORT_ROLE)
            .expect("expected compression_report message")
    }

    fn compression_report_index(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .position(|msg| msg.role == COMPRESSION_REPORT_ROLE)
            .expect("expected compression_report message")
    }

    fn compression_report_count(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .filter(|msg| msg.role == COMPRESSION_REPORT_ROLE)
            .count()
    }

    #[test]
    fn sanitize_messages_for_model_switch_drops_ui_only_messages() {
        let mut messages = vec![
            make_user_msg("visible"),
            make_ui_only_msg("context_length_exceeded"),
            make_assistant_msg("response"),
        ];

        sanitize_messages_for_model_switch(&mut messages);

        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|msg| !is_ui_only_message(msg)));
        assert!(messages.iter().all(|msg| !msg
            .content
            .content_text_only()
            .contains("context_length_exceeded")));
    }

    #[test]
    fn sanitize_messages_for_new_thread_does_not_make_ui_only_model_visible() {
        let messages = vec![
            make_user_msg("visible"),
            make_ui_only_msg("legacy diagnostic report"),
            make_assistant_msg("response"),
        ];

        let sanitized = sanitize_messages_for_new_thread(&messages);

        assert_eq!(sanitized.len(), 2);
        assert!(sanitized.iter().all(|msg| !is_ui_only_message(msg)));
        assert!(sanitized.iter().all(|msg| !msg
            .content
            .content_text_only()
            .contains("legacy diagnostic report")));
    }

    #[test]
    fn sanitize_messages_for_new_thread_preserves_plan_and_plan_delta_extra() {
        let messages = vec![make_plan_msg(), make_plan_delta_event()];

        let sanitized = sanitize_messages_for_new_thread(&messages);

        assert_eq!(sanitized.len(), 2);
        assert_only_hidden_plan_extra(&sanitized[0]);
        assert_only_hidden_event_extra(&sanitized[1]);
    }

    #[test]
    fn sanitize_message_for_new_thread_preserves_full_ui_only_extra() {
        let mut message = make_ui_only_msg("diagnostic");
        message
            .extra
            .insert("details".to_string(), serde_json::json!({"code": 1}));

        let sanitized = sanitize_message_for_new_thread(&message);

        assert_eq!(sanitized.extra, message.extra);
    }

    #[test]
    fn sanitize_messages_for_model_switch_preserves_hidden_role_extra_only() {
        let mut messages = vec![make_plan_msg(), make_plan_delta_event()];

        sanitize_messages_for_model_switch(&mut messages);

        assert_eq!(messages.len(), 2);
        assert_only_hidden_plan_extra(&messages[0]);
        assert_only_hidden_event_extra(&messages[1]);
    }

    #[test]
    fn compress_in_place_strip_metering_preserves_hidden_role_extra_only() {
        let mut messages = vec![make_plan_msg(), make_plan_delta_event()];
        let opts = CompressOptions {
            strip_metering: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let persisted: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role != COMPRESSION_REPORT_ROLE)
            .collect();
        assert_eq!(persisted.len(), 2);
        assert_only_hidden_plan_extra(persisted[0]);
        assert_only_hidden_event_extra(persisted[1]);
    }

    #[test]
    fn compress_in_place_strip_metering_drops_ui_only_message() {
        let mut messages = vec![
            make_user_msg("visible"),
            make_ui_only_msg("context_length_exceeded"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            strip_metering: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let persisted: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role != COMPRESSION_REPORT_ROLE)
            .collect();
        assert_eq!(persisted.len(), 2);
        assert!(persisted.iter().all(|msg| !is_ui_only_message(msg)));
        assert!(persisted.iter().all(|msg| msg.extra.is_empty()));
        assert!(messages.iter().all(|msg| !msg
            .content
            .content_text_only()
            .contains("context_length_exceeded")));
    }

    #[test]
    fn compress_in_place_no_strip_keeps_ui_only() {
        let mut messages = vec![
            make_user_msg("visible"),
            make_ui_only_msg("context_length_exceeded"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            strip_metering: false,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let persisted: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role != COMPRESSION_REPORT_ROLE)
            .collect();
        assert_eq!(persisted.len(), 3);
        assert_eq!(
            persisted
                .iter()
                .filter(|msg| is_ui_only_message(msg))
                .count(),
            1
        );
        assert!(messages.iter().any(|msg| msg
            .content
            .content_text_only()
            .contains("context_length_exceeded")));
    }

    #[test]
    fn test_compress_drop_all_context() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_context_file_msg("test.rs", "fn main() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.before_message_count, 3);
        assert_eq!(stats.after_message_count, 3);
        assert_eq!(stats.context_messages_modified, 1);
        assert!(messages
            .iter()
            .filter(|m| m.role != COMPRESSION_REPORT_ROLE)
            .all(|m| m.role != "context_file"));
        assert_eq!(messages[1].role, COMPRESSION_REPORT_ROLE);
    }

    #[test]
    fn test_compress_non_agentic_tools() {
        let long_content = "x".repeat(1000);
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("tc1", "some_tool"),
            make_tool_msg("tc1", &long_content),
        ];
        let opts = CompressOptions {
            compress_non_agentic_tools: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.tool_messages_modified, 1);
        let tool_msg = messages.iter().find(|m| m.role == "tool").unwrap();
        assert!(tool_msg.content.content_text_only().contains("compressed"));
    }

    #[test]
    fn test_compress_preserves_agentic_tools() {
        let long_content = "x".repeat(1000);
        for tool_name in &["research", "delegate", "plan", "review"] {
            let mut messages = vec![
                make_user_msg("hello"),
                make_assistant_with_tool_call("tc1", tool_name),
                make_tool_msg("tc1", &long_content),
            ];
            let opts = CompressOptions {
                compress_non_agentic_tools: true,
                ..Default::default()
            };
            let stats = compress_in_place(&mut messages, &opts).unwrap();
            assert_eq!(
                stats.tool_messages_modified, 0,
                "Tool {} should be preserved",
                tool_name
            );
            let tool_msg = messages.iter().find(|m| m.role == "tool").unwrap();
            assert!(!tool_msg.content.content_text_only().contains("compressed"));
        }
    }

    #[test]
    fn test_compress_compresses_cat_tool() {
        let long_content = "x".repeat(1000);
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("tc1", "cat"),
            make_tool_msg("tc1", &long_content),
        ];
        let opts = CompressOptions {
            compress_non_agentic_tools: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.tool_messages_modified, 1);
        let tool_msg = messages.iter().find(|m| m.role == "tool").unwrap();
        assert!(tool_msg.content.content_text_only().contains("compressed"));
    }

    #[test]
    fn test_compress_preserves_flagged_tool() {
        let long_content = "x".repeat(1000);
        let mut preserved = make_tool_msg("tc1", &long_content);
        preserved.preserve = Some(true);
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("tc1", "cat"),
            preserved,
        ];
        let opts = CompressOptions {
            compress_non_agentic_tools: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.tool_messages_modified, 0);
        let tool_msg = messages.iter().find(|m| m.role == "tool").unwrap();
        assert_eq!(tool_msg.content.content_text_only(), long_content);
    }

    #[test]
    fn test_handoff_include_last_user_plus_sync() {
        let messages = vec![
            make_user_msg("first question"),
            make_assistant_msg("first answer"),
            make_user_msg("second question"),
            make_assistant_msg("second answer"),
        ];

        let last_user_idx = messages.iter().rposition(|m| m.role == "user").unwrap();
        let selected: Vec<ChatMessage> = messages[last_user_idx..].to_vec();

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].content.content_text_only(), "second question");
        assert_eq!(selected[1].content.content_text_only(), "second answer");
    }

    #[test]
    fn test_should_preserve_tool() {
        assert!(should_preserve_tool("research"));
        assert!(should_preserve_tool("delegate"));
        assert!(should_preserve_tool("plan"));
        assert!(should_preserve_tool("review"));
        assert!(!should_preserve_tool("cat"));
        assert!(!should_preserve_tool("shell"));
        assert!(!should_preserve_tool("unknown_tool"));
        assert!(!should_preserve_tool(""));
    }

    #[test]
    fn test_approx_token_count() {
        let messages = vec![make_user_msg("hello world")];
        let count = approx_token_count(&messages);
        assert!(count > 0);
    }

    #[test]
    fn test_transform_stats_default() {
        let stats = TransformStats::default();
        assert_eq!(stats.before_message_count, 0);
        assert_eq!(stats.after_message_count, 0);
    }

    #[test]
    fn test_compress_options_default() {
        let opts = CompressOptions::default();
        assert!(!opts.dedup_and_compress_context);
        assert!(!opts.drop_all_context);
        assert!(!opts.compress_non_agentic_tools);
        assert!(!opts.drop_all_memories);
        assert!(!opts.drop_project_information);
    }

    #[test]
    fn test_compression_report_not_added_for_noop_compress() {
        let mut messages = vec![make_user_msg("hello"), make_assistant_msg("response")];
        let opts = CompressOptions::default();
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        assert_eq!(stats.before_message_count, 2);
        assert_eq!(stats.after_message_count, 2);
        assert_eq!(compression_report_count(&messages), 0);
        assert!(messages.iter().all(|msg| msg.role != "cd_instruction"));
    }

    #[test]
    fn test_compression_report_metadata_fields_exist() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("tc1", "cat"),
            make_tool_msg("tc1", &"x".repeat(1000)),
            make_context_file_msg("test.rs", "fn main() {}"),
        ];
        let opts = CompressOptions {
            drop_all_context: true,
            compress_non_agentic_tools: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let report = compression_report(&messages);
        assert_eq!(report.summarization_tier.as_deref(), Some("tier2_reactive"));
        let metadata = &report.extra["compression_report"];
        assert_eq!(metadata["kind"], serde_json::json!(COMPRESSION_REPORT_KIND));
        assert_eq!(metadata["context_files_removed"], serde_json::json!(1));
        assert_eq!(metadata["context_messages_dropped"], serde_json::json!(1));
        assert_eq!(metadata["tool_results_truncated"], serde_json::json!(1));
        assert!(report
            .content
            .content_text_only()
            .contains("- Context messages dropped: 1"));
        assert!(metadata["tokens_before"].as_u64().unwrap() > 0);
        assert!(metadata["tokens_after"].as_u64().unwrap() > 0);
        assert!(metadata["reduction_percent"].as_u64().unwrap() <= 100);
        assert_eq!(
            report.summarized_token_estimate,
            metadata["estimated_tokens_saved"]
                .as_u64()
                .map(|value| value as usize)
        );
    }

    #[test]
    fn test_compression_report_preserves_tail_after_boundary() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_context_file_msg("test.rs", "fn main() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let roles: Vec<_> = messages.iter().map(|msg| msg.role.as_str()).collect();
        assert_eq!(roles, vec!["user", COMPRESSION_REPORT_ROLE, "assistant"]);
    }

    #[test]
    fn test_compression_report_does_not_split_leading_system_messages() {
        let mut messages = vec![
            make_system_msg("root prompt"),
            make_system_msg("workspace details"),
            make_user_msg("hello"),
            make_context_file_msg("test.rs", "fn main() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let report_idx = compression_report_index(&messages);
        assert_eq!(report_idx, 3);
        let roles: Vec<_> = messages.iter().map(|msg| msg.role.as_str()).collect();
        assert_eq!(
            roles,
            vec![
                "system",
                "system",
                "user",
                COMPRESSION_REPORT_ROLE,
                "assistant"
            ]
        );
    }

    #[test]
    fn test_compression_report_is_not_before_first_user() {
        let mut messages = vec![
            make_system_msg("root prompt"),
            make_user_msg("hello"),
            make_context_file_msg("test.rs", "fn main() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let report_idx = compression_report_index(&messages);
        let first_user_idx = messages.iter().position(|msg| msg.role == "user").unwrap();
        assert!(report_idx > first_user_idx);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
    }

    #[test]
    fn test_compression_report_inserted_near_removed_context_not_tail() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_context_file_msg("test.rs", "fn main() {}"),
            make_assistant_msg("first response"),
            make_user_msg("second request"),
            make_assistant_msg("second response"),
        ];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };

        compress_in_place(&mut messages, &opts).unwrap();

        let report_idx = compression_report_index(&messages);
        assert_eq!(report_idx, 1);
        assert!(report_idx < messages.len() - 1);
        assert_eq!(
            messages.last().unwrap().content.content_text_only(),
            "second response"
        );
    }

    #[test]
    fn test_drop_all_memories() {
        fn make_multi_context_file_msg(files: Vec<(&str, &str)>) -> ChatMessage {
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(
                    files
                        .into_iter()
                        .map(|(name, content)| make_context_file(name, content))
                        .collect(),
                ),
                ..Default::default()
            }
        }

        let mut messages = vec![
            make_user_msg("hello"),
            make_context_file_msg(
                "/home/user/.refact/knowledge/2026-01-01_mem.md",
                "some memory",
            ),
            make_multi_context_file_msg(vec![
                ("/home/user/.refact/knowledge/other.md", "knowledge"),
                ("regular.rs", "fn main() {}"),
            ]),
            make_context_file_msg("src/lib.rs", "pub fn foo() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_memories: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        assert_eq!(stats.context_messages_modified, 2);

        assert!(!messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files
                    .iter()
                    .any(|f| f.file_name.contains(".refact/knowledge/2026"))
            } else {
                false
            }
        }));

        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|f| f.file_name == "regular.rs")
            } else {
                false
            }
        }));

        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|f| f.file_name == "src/lib.rs")
            } else {
                false
            }
        }));
    }

    #[test]
    fn test_handoff_excluded_selection_with_empty_message_ids() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("first question"),
            make_assistant_msg("first answer"),
            make_user_msg("second question"),
            make_assistant_msg("second answer"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let start_idx = messages.iter().rposition(|m| m.role == "user").unwrap();
        let (conversation, excluded) =
            handoff_conversation_and_excluded(&messages, &opts, 1, start_idx, &HashSet::new());

        let conversation_text: Vec<_> = conversation
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();
        let excluded_text: Vec<_> = excluded
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();

        assert_eq!(conversation_text, vec!["second question", "second answer"]);
        assert_eq!(excluded_text, vec!["first question", "first answer"]);
    }

    #[test]
    fn test_handoff_excluded_selection_with_duplicate_message_ids() {
        let messages = vec![
            with_message_id(make_system_msg("s"), "system-id"),
            with_message_id(make_user_msg("first question"), "duplicate-id"),
            with_message_id(make_assistant_msg("first answer"), "duplicate-id"),
            with_message_id(make_user_msg("second question"), "duplicate-id"),
            with_message_id(make_assistant_msg("second answer"), "duplicate-id"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let start_idx = messages.iter().rposition(|m| m.role == "user").unwrap();
        let (conversation, excluded) =
            handoff_conversation_and_excluded(&messages, &opts, 1, start_idx, &HashSet::new());

        let conversation_text: Vec<_> = conversation
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();
        let excluded_text: Vec<_> = excluded
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();

        assert_eq!(conversation_text, vec!["second question", "second answer"]);
        assert_eq!(excluded_text, vec!["first question", "first answer"]);
    }

    #[test]
    fn test_drop_all_memories_removes_absolute_relative_and_windows_paths() {
        let mut messages = vec![
            make_context_file_msg("/repo/.refact/knowledge/memory.md", "memory"),
            make_context_file_msg(".refact/trajectories/chat.json", "trajectory"),
            make_context_file_msg(
                r#"C:\Users\user\repo\.refact\tasks\task-id\memories\note.md"#,
                "task memory",
            ),
            make_context_file_msg("src/lib.rs", "pub fn lib() {}"),
        ];
        let opts = CompressOptions {
            drop_all_memories: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        assert_eq!(stats.context_messages_modified, 3);
        assert!(!messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|file| is_memory_path(&file.file_name))
            } else {
                false
            }
        }));
        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|file| file.file_name == "src/lib.rs")
            } else {
                false
            }
        }));
    }

    #[test]
    fn test_drop_all_memories_removes_context_file_simple_text_with_memory_paths() {
        let serialized_memory = make_context_file_simple_text_msg(vec![make_context_file(
            ".refact/knowledge/preference.md",
            "memory",
        )]);
        let embedded_memory = ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText(
                r#"[{"file_name":"C:\\repo\\.refact\\tasks\\task-id\\memo.md","file_content":"memo"}]"#.to_string(),
            ),
            ..Default::default()
        };
        let source = make_context_file_simple_text_msg(vec![make_context_file(
            "src/main.rs",
            "fn main() {}",
        )]);
        let mut messages = vec![serialized_memory, embedded_memory, source];
        let opts = CompressOptions {
            drop_all_memories: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        assert_eq!(stats.context_messages_modified, 2);
        assert_eq!(
            messages.iter().filter(|m| m.role == "context_file").count(),
            1
        );
        let context_msg = messages.iter().find(|m| m.role == "context_file").unwrap();
        let files: Vec<ContextFile> =
            serde_json::from_str(&context_msg.content.content_text_only()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name, "src/main.rs");
    }

    #[test]
    fn test_drop_all_memories_keeps_non_memory_source_context_files() {
        let mut messages = vec![
            make_context_file_msg("src/lib.rs", "pub fn lib() {}"),
            make_context_file_msg("tests/.refact_fixture/tasks/example.rs", "fixture"),
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::SimpleText(
                    "file: src/mentions_refact.rs\nlet path = \".refact/tasks/not-a-context-path\";"
                        .to_string(),
                ),
                ..Default::default()
            },
        ];
        let opts = CompressOptions {
            drop_all_memories: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        assert_eq!(stats.context_messages_modified, 0);
        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|file| file.file_name == "src/lib.rs")
            } else {
                false
            }
        }));
        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files
                    .iter()
                    .any(|file| file.file_name == "tests/.refact_fixture/tasks/example.rs")
            } else {
                false
            }
        }));
        assert!(messages.iter().any(|m| {
            m.role == "context_file"
                && matches!(&m.content, ChatContent::SimpleText(text) if text.contains("mentions_refact.rs"))
        }));
    }

    #[test]
    fn test_drop_project_information() {
        let mut messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText(
                    "You are an agent. Workspace: /home/user/project".to_string(),
                ),
                ..Default::default()
            },
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText("Project structure: ...".to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText("You are an assistant".to_string()),
                ..Default::default()
            },
            make_user_msg("hello"),
        ];
        let opts = CompressOptions {
            drop_project_information: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        assert_eq!(stats.context_messages_modified, 1);

        assert!(messages
            .iter()
            .any(|m| m.role == "system" && m.content.content_text_only().contains("Workspace")));

        assert!(messages
            .iter()
            .any(|m| m.role == "system" && m.content.content_text_only().contains("assistant")));
    }

    #[test]
    fn test_handoff_options_default() {
        let opts = HandoffOptions::default();
        assert!(!opts.include_last_user_plus);
        assert!(!opts.include_all_opened_context);
        assert!(!opts.include_all_edited_context);
        assert!(!opts.include_agentic_tools);
        assert!(!opts.llm_summary_for_excluded);
    }

    #[test]
    fn test_compress_preserves_user_assistant_without_noop_report() {
        let mut messages = vec![make_user_msg("hello"), make_assistant_msg("response")];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.after_message_count, 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(compression_report_count(&messages), 0);
    }

    #[test]
    fn test_compress_empty_messages_is_noop_without_report() {
        let mut messages: Vec<ChatMessage> = vec![];
        let opts = CompressOptions::default();
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.before_message_count, 0);
        assert_eq!(stats.after_message_count, 0);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_is_valid_tool_id() {
        assert!(is_valid_tool_id("call_abc123"));
        assert!(is_valid_tool_id("toolu_def456"));
        assert!(is_valid_tool_id("abc-def_123"));
        assert!(is_valid_tool_id("A"));
        assert!(!is_valid_tool_id(""));
        assert!(!is_valid_tool_id("call.123"));
        assert!(!is_valid_tool_id("call:123"));
        assert!(!is_valid_tool_id("call/123"));
        assert!(!is_valid_tool_id("call 123"));
    }

    #[test]
    fn test_generate_valid_tool_id() {
        let id = generate_valid_tool_id();
        assert!(id.starts_with("call_"));
        assert!(is_valid_tool_id(&id));
        assert_eq!(id.len(), 29);
    }

    #[test]
    fn test_sanitize_messages_for_model_switch_strips_metadata() {
        let mut messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("hello".to_string()),
            usage: Some(ChatUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                metering_usd: None,
            }),
            finish_reason: Some("stop".to_string()),
            reasoning_content: Some("thinking...".to_string()),
            extra: {
                let mut map = serde_json::Map::new();
                map.insert("cache".to_string(), serde_json::json!(true));
                map
            },
            ..Default::default()
        }];

        sanitize_messages_for_model_switch(&mut messages);

        assert!(messages[0].usage.is_none());
        assert!(messages[0].finish_reason.is_none());
        assert!(messages[0].reasoning_content.is_none());
        assert!(messages[0].extra.is_empty());
        assert_eq!(messages[0].content.content_text_only(), "hello");
    }

    #[test]
    fn test_sanitize_messages_for_model_switch_normalizes_tool_ids() {
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "gemini.call.123".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("result".to_string()),
                tool_call_id: "gemini.call.123".to_string(),
                ..Default::default()
            },
        ];

        sanitize_messages_for_model_switch(&mut messages);

        let new_id = &messages[0].tool_calls.as_ref().unwrap()[0].id;
        assert!(is_valid_tool_id(new_id));
        assert!(new_id.starts_with("call_"));
        assert_eq!(messages[1].tool_call_id, *new_id);
    }

    #[test]
    fn test_sanitize_messages_for_model_switch_preserves_valid_ids() {
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_valid123".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("result".to_string()),
                tool_call_id: "call_valid123".to_string(),
                ..Default::default()
            },
        ];

        sanitize_messages_for_model_switch(&mut messages);

        assert_eq!(
            messages[0].tool_calls.as_ref().unwrap()[0].id,
            "call_valid123"
        );
        assert_eq!(messages[1].tool_call_id, "call_valid123");
    }

    #[test]
    fn test_sanitize_messages_for_model_switch_multiple_invalid_ids() {
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "bad:id:1".to_string(),
                        index: None,
                        function: ChatToolFunction {
                            name: "tool1".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    },
                    ChatToolCall {
                        id: "bad.id.2".to_string(),
                        index: None,
                        function: ChatToolFunction {
                            name: "tool2".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("result1".to_string()),
                tool_call_id: "bad:id:1".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("result2".to_string()),
                tool_call_id: "bad.id.2".to_string(),
                ..Default::default()
            },
        ];

        sanitize_messages_for_model_switch(&mut messages);

        let tc = messages[0].tool_calls.as_ref().unwrap();
        assert!(is_valid_tool_id(&tc[0].id));
        assert!(is_valid_tool_id(&tc[1].id));
        assert_ne!(tc[0].id, tc[1].id);
        assert_eq!(messages[1].tool_call_id, tc[0].id);
        assert_eq!(messages[2].tool_call_id, tc[1].id);
    }
}
