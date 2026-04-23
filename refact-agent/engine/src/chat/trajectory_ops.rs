use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;

pub fn sanitize_message_for_new_thread(m: &ChatMessage) -> ChatMessage {
    ChatMessage {
        message_id: m.message_id.clone(),
        role: m.role.clone(),
        content: m.content.clone(),
        tool_calls: m.tool_calls.clone(),
        tool_call_id: m.tool_call_id.clone(),
        tool_failed: m.tool_failed,
        finish_reason: None,
        reasoning_content: None,
        usage: None,
        checkpoints: vec![],
        thinking_blocks: None,
        citations: vec![],
        server_content_blocks: vec![],
        extra: serde_json::Map::new(),
        output_filter: None,
    }
}

pub fn sanitize_messages_for_new_thread(msgs: &[ChatMessage]) -> Vec<ChatMessage> {
    msgs.iter().map(sanitize_message_for_new_thread).collect()
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

pub fn sanitize_messages_for_model_switch(msgs: &mut [ChatMessage]) {
    // Thinking blocks and server-side content blocks are backend-specific.
    // If a chat switches models/providers, re-sending them can cause provider
    // validation errors (e.g., Anthropic thinking signatures are integrity-checked).
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
        msg.extra = serde_json::Map::new();
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

pub const TOOLS_TO_PRESERVE: &[&str] = &["deep_research", "subagent", "strategic_planning", "code_review"];

fn should_preserve_tool(name: &str) -> bool {
    TOOLS_TO_PRESERVE.iter().any(|t| *t == name)
}

fn is_memory_path(path: &str) -> bool {
    path.contains("/.refact/knowledge/")
        || path.contains("/.refact/trajectories/")
        || path.contains("/.refact/tasks/")
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
    let mut tool_modified = 0;

    if opts.drop_all_context {
        messages.retain(|m| {
            if m.role == "context_file" {
                context_modified += 1;
                false
            } else {
                true
            }
        });
    } else if opts.dedup_and_compress_context {
        let result = super::history_limit::compress_duplicate_context_files(messages);
        if let Ok((count, _)) = result {
            context_modified = count;
        }
    }

    if opts.drop_all_memories {
        for msg in messages.iter_mut() {
            if msg.role != "context_file" {
                continue;
            }
            if let ChatContent::ContextFiles(files) = &msg.content {
                let remaining: Vec<_> = files
                    .iter()
                    .filter(|cf| !is_memory_path(&cf.file_name))
                    .cloned()
                    .collect();
                if remaining.len() < files.len() {
                    context_modified += files.len() - remaining.len();
                    msg.content = ChatContent::ContextFiles(remaining);
                }
            }
        }
        messages.retain(|m| {
            if m.role != "context_file" {
                return true;
            }
            match &m.content {
                ChatContent::ContextFiles(files) => !files.is_empty(),
                _ => true,
            }
        });
    }

    if opts.drop_project_information {
        let first_system_idx = messages.iter().position(|m| m.role == "system");
        let mut idx = 0usize;
        messages.retain(|msg| {
            let keep = if msg.role != "system" {
                true
            } else if Some(idx) == first_system_idx {
                true
            } else {
                let text = msg.content.content_text_only().to_lowercase();
                if text.contains("project") || text.contains("workspace") {
                    context_modified += 1;
                    false
                } else {
                    true
                }
            };
            idx += 1;
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

        for msg in messages.iter_mut() {
            if msg.role == "tool" && !msg.tool_call_id.is_empty() {
                if let Some(name) = tool_call_names.get(&msg.tool_call_id) {
                    if should_preserve_tool(name) {
                        continue;
                    }
                }
                let content_text = msg.content.content_text_only();
                if content_text.len() > 500 {
                    let preview: String = content_text.chars().take(200).collect();
                    msg.content =
                        ChatContent::SimpleText(format!("Tool result compressed: {}...", preview));
                    tool_modified += 1;
                }
            }
        }
    }

    super::history_limit::remove_invalid_tool_calls_and_tool_calls_results(messages);

    if opts.strip_metering {
        for msg in messages.iter_mut() {
            msg.usage = None;
            msg.extra.retain(|key, _| !key.starts_with("metering_coins_"));
        }
    }

    let after_tokens_pre = approx_token_count(messages);
    let reduction_percent = if before_tokens > 0 {
        ((before_tokens.saturating_sub(after_tokens_pre)) * 100) / before_tokens
    } else {
        0
    };

    let instruction = ChatMessage {
        role: "cd_instruction".to_string(),
        content: ChatContent::SimpleText(format!(
            "💿 Chat compressed. {} context files removed, {} tool results truncated. Tokens reduced from ~{} to ~{} (~{}% reduction). You can use the Trajectory panel to further compress or create a handoff.",
            context_modified,
            tool_modified,
            before_tokens,
            after_tokens_pre,
            reduction_percent
        )),
        ..Default::default()
    };
    messages.push(instruction);

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

pub async fn handoff_select(
    messages: &[ChatMessage],
    opts: &HandoffOptions,
    gcx: Arc<ARwLock<GlobalContext>>,
    generate_summary: bool,
    trajectory_id: &str,
) -> Result<(Vec<ChatMessage>, TransformStats, Option<String>), String> {
    use crate::call_validation::ContextFile;

    let before_count = messages.len();
    let before_tokens = approx_token_count(messages);

    let system_prefix_len = messages.iter().take_while(|m| m.role == "system").count();
    let system_prefix: Vec<ChatMessage> =
        messages.iter().take(system_prefix_len).cloned().collect();

    let start_idx = if opts.include_last_user_plus {
        messages
            .iter()
            .rposition(|m| m.role == "user")
            .unwrap_or(messages.len())
    } else {
        messages.len()
    };

    let bundled_context: Option<ChatMessage> = if opts.include_all_opened_context {
        let all_files: Vec<ContextFile> = messages
            .iter()
            .skip(system_prefix_len)
            .filter(|m| m.role == "context_file")
            .filter_map(|m| {
                if let ChatContent::ContextFiles(files) = &m.content {
                    Some(files.clone())
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        if all_files.is_empty() {
            None
        } else {
            Some(ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(all_files),
                ..Default::default()
            })
        }
    } else {
        None
    };

    let mut preserved_tool_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut edited_tool_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut agentic_tool_messages: Vec<ChatMessage> = Vec::new();

    if opts.include_agentic_tools {
        for msg in messages.iter() {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    if should_preserve_tool(&tc.function.name) {
                        preserved_tool_ids.insert(tc.id.clone());
                    }
                }
            }
        }

        for msg in messages.iter() {
            if let Some(ref tool_calls) = msg.tool_calls {
                let preserved_calls: Vec<_> = tool_calls
                    .iter()
                    .filter(|tc| should_preserve_tool(&tc.function.name))
                    .cloned()
                    .collect();

                if !preserved_calls.is_empty() {
                    let mut assistant_msg = msg.clone();
                    assistant_msg.tool_calls = Some(preserved_calls);
                    agentic_tool_messages.push(assistant_msg);
                }
            }

            if (msg.role == "tool" || msg.role == "diff")
                && preserved_tool_ids.contains(&msg.tool_call_id)
            {
                agentic_tool_messages.push(msg.clone());
            }
        }
    }

    if opts.include_agentic_tools && opts.include_all_edited_context {
        for msg in messages.iter() {
            if msg.role == "diff"
                && !msg.tool_call_id.is_empty()
                && !preserved_tool_ids.contains(&msg.tool_call_id)
            {
                edited_tool_ids.insert(msg.tool_call_id.clone());
            }
        }

        for msg in messages.iter() {
            if let Some(ref tool_calls) = msg.tool_calls {
                let edited_calls: Vec<_> = tool_calls
                    .iter()
                    .filter(|tc| edited_tool_ids.contains(&tc.id))
                    .cloned()
                    .collect();

                if !edited_calls.is_empty() {
                    let mut assistant_msg = msg.clone();
                    assistant_msg.tool_calls = Some(edited_calls);
                    agentic_tool_messages.push(assistant_msg);
                }
            }

            if msg.role == "diff" && edited_tool_ids.contains(&msg.tool_call_id) {
                agentic_tool_messages.push(msg.clone());
            }
        }
    }

    let mut conversation: Vec<ChatMessage> = Vec::new();
    for (i, msg) in messages.iter().enumerate().skip(system_prefix_len) {
        let should_include = if opts.include_all_user_assistant_only {
            match msg.role.as_str() {
                "user" | "assistant" => true,
                _ => false,
            }
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

    let mut llm_summary: Option<String> = None;
    let mut summary_msg: Option<ChatMessage> = None;

    if opts.llm_summary_for_excluded && generate_summary {
        let selected_ids: std::collections::HashSet<&str> = conversation
            .iter()
            .map(|m| m.message_id.as_str())
            .collect();
        let excluded: Vec<ChatMessage> = messages
            .iter()
            .skip(system_prefix_len)
            .filter(|m| !selected_ids.contains(m.message_id.as_str()))
            .cloned()
            .collect();
        let excluded_sanitized = sanitize_messages_for_new_thread(&excluded);

        if !excluded_sanitized.is_empty() {
            let summary =
                crate::agentic::compress_trajectory::compress_trajectory(gcx, &excluded_sanitized)
                    .await
                    .map_err(|e| format!("Failed to generate summary: {}", e))?;
            summary_msg = Some(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(format!(
                    "## Previous conversation summary\n\n{}",
                    summary
                )),
                ..Default::default()
            });
            llm_summary = Some(summary);
        }
    }

    let mut selected: Vec<ChatMessage> = Vec::new();
    if !opts.include_all_user_assistant_only {
        selected.extend(system_prefix);
    }
    if let Some(ctx_msg) = bundled_context {
        selected.push(ctx_msg);
    }
    selected.extend(agentic_tool_messages);
    if let Some(msg) = summary_msg {
        selected.push(msg);
    }
    selected.extend(conversation);

    super::history_limit::remove_invalid_tool_calls_and_tool_calls_results(&mut selected);

    let handoff_context_msg = ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(format!(
            "The previous trajectory {}. Continue from where you stopped.",
            trajectory_id
        )),
        ..Default::default()
    };
    selected.push(handoff_context_msg);

    let stats = TransformStats {
        before_message_count: before_count,
        after_message_count: selected.len(),
        before_approx_tokens: before_tokens,
        after_approx_tokens: approx_token_count(&selected),
        context_messages_modified: 0,
        tool_messages_modified: 0,
    };

    Ok((selected, stats, llm_summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction, ContextFile};

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

    fn make_context_file_msg(filename: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: filename.to_string(),
                file_content: content.to_string(),
                line1: 1,
                line2: 100,
                file_rev: None,
                symbols: vec![],
                gradient_type: -1,
                usefulness: 0.0,
                skip_pp: false,
            }]),
            ..Default::default()
        }
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
            .filter(|m| m.role != "cd_instruction")
            .all(|m| m.role != "context_file"));
        assert!(messages.last().unwrap().role == "cd_instruction");
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
        for tool_name in &["deep_research", "subagent", "strategic_planning", "code_review"] {
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
        assert!(should_preserve_tool("deep_research"));
        assert!(should_preserve_tool("subagent"));
        assert!(should_preserve_tool("strategic_planning"));
        assert!(should_preserve_tool("code_review"));
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
    fn test_cd_instruction_added_after_compress() {
        let mut messages = vec![make_user_msg("hello"), make_assistant_msg("response")];
        let opts = CompressOptions::default();
        compress_in_place(&mut messages, &opts).unwrap();
        let last_msg = messages.last().unwrap();
        assert_eq!(last_msg.role, "cd_instruction");
        assert!(last_msg
            .content
            .content_text_only()
            .contains("Chat compressed"));
    }

    #[test]
    fn test_drop_all_memories() {
        use crate::call_validation::ContextFile;

        fn make_multi_context_file_msg(files: Vec<(&str, &str)>) -> ChatMessage {
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(
                    files
                        .into_iter()
                        .map(|(name, content)| ContextFile {
                            file_name: name.to_string(),
                            file_content: content.to_string(),
                            line1: 1,
                            line2: 1,
                            file_rev: None,
                            symbols: vec![],
                            gradient_type: -1,
                            usefulness: 0.0,
                            skip_pp: false,
                        })
                        .collect(),
                ),
                ..Default::default()
            }
        }

        let mut messages = vec![
            make_user_msg("hello"),
            // Pure memory message — should be removed entirely
            make_context_file_msg(
                "/home/user/.refact/knowledge/2026-01-01_mem.md",
                "some memory",
            ),
            // Mixed message: one memory file + one source file
            make_multi_context_file_msg(vec![
                ("/home/user/.refact/knowledge/other.md", "knowledge"),
                ("regular.rs", "fn main() {}"),
            ]),
            // Pure source file — must be preserved
            make_context_file_msg("src/lib.rs", "pub fn foo() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_memories: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();

        // 1 file dropped from pure-memory message + 1 file from the mixed message
        assert_eq!(stats.context_messages_modified, 2);

        // Pure-memory message is gone
        assert!(!messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|f| f.file_name.contains(".refact/knowledge/2026"))
            } else {
                false
            }
        }));

        // regular.rs survives (was bundled with a memory file)
        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|f| f.file_name == "regular.rs")
            } else {
                false
            }
        }));

        // src/lib.rs survives
        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.iter().any(|f| f.file_name == "src/lib.rs")
            } else {
                false
            }
        }));
    }

    #[test]
    fn test_drop_project_information() {
        // The first system message is the main agent prompt and must never be dropped,
        // even if it contains "project" or "workspace". Secondary system messages that
        // contain those words should be removed.
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

        // Only the second system message ("Project structure") should be dropped
        assert_eq!(stats.context_messages_modified, 1);

        // First system message (the main prompt) must survive even though it contains "project"/"workspace"
        assert!(messages
            .iter()
            .any(|m| m.role == "system" && m.content.content_text_only().contains("Workspace")));

        // Third system message (no project/workspace) must also survive
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
    fn test_compress_preserves_user_assistant() {
        let mut messages = vec![make_user_msg("hello"), make_assistant_msg("response")];
        let opts = CompressOptions {
            drop_all_context: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.after_message_count, 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[2].role, "cd_instruction");
    }

    #[test]
    fn test_compress_empty_messages() {
        let mut messages: Vec<ChatMessage> = vec![];
        let opts = CompressOptions::default();
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.before_message_count, 0);
        assert_eq!(stats.after_message_count, 1);
        assert_eq!(messages[0].role, "cd_instruction");
    }

    fn make_system_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn roles(messages: &[ChatMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.role.as_str()).collect()
    }

    fn assert_system_prefix(messages: &[ChatMessage]) {
        let first_non_system = messages
            .iter()
            .position(|m| m.role != "system")
            .unwrap_or(messages.len());
        assert!(
            messages
                .iter()
                .skip(first_non_system)
                .all(|m| m.role != "system"),
            "system messages must be prefix, got: {:?}",
            roles(messages)
        );
    }

    #[tokio::test]
    async fn test_handoff_preserves_system_prefix() {
        let messages = vec![
            make_system_msg("You are an assistant"),
            make_user_msg("first question"),
            make_assistant_msg("first answer"),
            make_user_msg("second question"),
            make_assistant_msg("second answer"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected[0].role, "system");
        assert_eq!(
            selected[0].content.content_text_only(),
            "You are an assistant"
        );
        assert_eq!(selected[1].role, "user");
        assert_eq!(selected[1].content.content_text_only(), "second question");
        assert_eq!(selected[2].role, "assistant");
        assert_eq!(selected[3].role, "user");
    }

    #[tokio::test]
    async fn test_handoff_system_before_context_files() {
        let messages = vec![
            make_system_msg("You are an assistant"),
            make_context_file_msg("test.rs", "fn main() {}"),
            make_user_msg("question"),
            make_assistant_msg("answer"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_all_opened_context: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected[0].role, "system");
        assert_eq!(selected[1].role, "context_file");
        assert_eq!(selected[2].role, "user");
    }

    #[tokio::test]
    async fn test_handoff_multiple_system_messages_preserved() {
        let messages = vec![
            make_system_msg("System prompt 1"),
            make_system_msg("System prompt 2"),
            make_user_msg("question"),
            make_assistant_msg("answer"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected[0].role, "system");
        assert_eq!(selected[0].content.content_text_only(), "System prompt 1");
        assert_eq!(selected[1].role, "system");
        assert_eq!(selected[1].content.content_text_only(), "System prompt 2");
        assert_eq!(selected[2].role, "user");
    }

    #[tokio::test]
    async fn test_handoff_no_system_messages() {
        let messages = vec![make_user_msg("question"), make_assistant_msg("answer")];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected[0].role, "user");
        assert_eq!(selected[1].role, "assistant");
        assert_eq!(selected[2].role, "user");
    }

    #[tokio::test]
    async fn test_handoff_only_system_when_all_options_false() {
        let messages = vec![
            make_system_msg("System prompt"),
            make_user_msg("first question"),
            make_assistant_msg("first answer"),
            make_user_msg("second question"),
            make_assistant_msg("second answer"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: false,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected.len(), 2);
        assert_eq!(roles(&selected), vec!["system", "user"]);
    }

    #[tokio::test]
    async fn test_handoff_mid_chat_system_dropped() {
        let messages = vec![
            make_system_msg("s1"),
            make_user_msg("u1"),
            make_system_msg("s2"),
            make_assistant_msg("a1"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: false,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        let system_count = selected.iter().filter(|m| m.role == "system").count();
        assert_eq!(system_count, 1);
        assert_eq!(selected[0].content.content_text_only(), "s1");
    }

    #[tokio::test]
    async fn test_handoff_non_preserved_tool_removed() {
        // cat is NOT in TOOLS_TO_PRESERVE, so it should be removed
        let messages = vec![
            make_system_msg("s"),
            make_assistant_with_tool_call("tc1", "cat"),
            make_tool_msg("tc1", "tool output"),
            make_user_msg("q"),
            make_assistant_msg("a"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert!(selected.iter().all(|m| m.role != "tool"));
        assert_eq!(roles(&selected), vec!["system", "user", "assistant", "user"]);
    }

    #[tokio::test]
    async fn test_handoff_preserved_tool_pair_included() {
        // deep_research IS in TOOLS_TO_PRESERVE, so it should be preserved
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "deep_research"),
            make_tool_msg("tc1", "research results"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(
            roles(&selected),
            vec!["system", "assistant", "tool", "user", "assistant", "user"]
        );
        assert_eq!(selected[1].tool_calls.as_ref().unwrap()[0].id, "tc1");
        assert_eq!(selected[2].tool_call_id, "tc1");
    }

    #[tokio::test]
    async fn test_handoff_subagent_preserved() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "subagent"),
            make_tool_msg("tc1", "subagent results"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(
            roles(&selected),
            vec!["system", "assistant", "tool", "user", "assistant", "user"]
        );
    }

    #[tokio::test]
    async fn test_handoff_strategic_planning_preserved() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "strategic_planning"),
            make_tool_msg("tc1", "planning results"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(
            roles(&selected),
            vec!["system", "assistant", "tool", "user", "assistant", "user"]
        );
    }

    #[tokio::test]
    async fn test_handoff_code_review_preserved() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "code_review"),
            make_tool_msg("tc1", "code review results"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(
            roles(&selected),
            vec!["system", "assistant", "tool", "user", "assistant", "user"]
        );
    }

    #[tokio::test]
    async fn test_handoff_empty_input() {
        let messages: Vec<ChatMessage> = vec![];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_all_opened_context: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_eq!(roles(&selected), vec!["user"]);
    }

    #[tokio::test]
    async fn test_handoff_only_system_messages() {
        let messages = vec![make_system_msg("s1"), make_system_msg("s2")];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected.len(), 3);
        assert_eq!(roles(&selected), vec!["system", "system", "user"]);
    }

    #[tokio::test]
    async fn test_handoff_context_files_bundled_into_single_message() {
        let messages = vec![
            make_system_msg("s"),
            make_context_file_msg("early.rs", "early"),
            make_user_msg("u1"),
            make_context_file_msg("mid.rs", "mid"),
            make_assistant_msg("a1"),
            make_user_msg("u2"),
            make_context_file_msg("late.rs", "late"),
            make_assistant_msg("a2"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_all_opened_context: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(selected[0].role, "system");

        // Context files should be bundled into a single message
        let cf_count = selected.iter().filter(|m| m.role == "context_file").count();
        assert_eq!(
            cf_count, 1,
            "All context files should be bundled into one message"
        );

        // The bundled message should contain all 3 files
        let cf_msg = selected.iter().find(|m| m.role == "context_file").unwrap();
        if let ChatContent::ContextFiles(files) = &cf_msg.content {
            assert_eq!(files.len(), 3);
            let names: Vec<_> = files.iter().map(|f| f.file_name.as_str()).collect();
            assert!(names.contains(&"early.rs"));
            assert!(names.contains(&"mid.rs"));
            assert!(names.contains(&"late.rs"));
        } else {
            panic!("Expected ContextFiles content");
        }

        // Context file should come before user messages
        let first_cf_idx = selected
            .iter()
            .position(|m| m.role == "context_file")
            .unwrap();
        let first_user_idx = selected.iter().position(|m| m.role == "user").unwrap();
        assert!(first_cf_idx < first_user_idx);
    }

    #[tokio::test]
    async fn test_handoff_single_user_message() {
        let messages = vec![make_system_msg("s"), make_user_msg("only question")];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(roles(&selected), vec!["system", "user", "user"]);
    }

    #[tokio::test]
    async fn test_handoff_diff_messages_with_edited_context() {
        let diff_msg = ChatMessage {
            role: "diff".to_string(),
            tool_call_id: "tc1".to_string(),
            content: ChatContent::SimpleText("diff content".to_string()),
            ..Default::default()
        };
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("u1"),
            make_assistant_with_tool_call("tc1", "update_textdoc"),
            diff_msg,
            make_user_msg("u2"),
            make_assistant_msg("a2"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_all_edited_context: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(
            roles(&selected),
            vec!["system", "assistant", "diff", "user", "assistant", "user"]
        );
    }

    #[tokio::test]
    async fn test_handoff_edited_context_requires_agentic_tools() {
        let diff_msg = ChatMessage {
            role: "diff".to_string(),
            tool_call_id: "tc1".to_string(),
            content: ChatContent::SimpleText("diff content".to_string()),
            ..Default::default()
        };
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("u1"),
            make_assistant_with_tool_call("tc1", "update_textdoc"),
            diff_msg,
            make_user_msg("u2"),
            make_assistant_msg("a2"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_all_edited_context: true,
            include_agentic_tools: false,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(roles(&selected), vec!["system", "user", "assistant", "user"]);
    }

    #[tokio::test]
    async fn test_handoff_preserved_tools_before_conversation() {
        // Test that preserved tools (deep_research) come before the conversation
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q1"),
            make_assistant_with_tool_call("tc1", "deep_research"),
            make_tool_msg("tc1", "research results"),
            make_assistant_msg("after research"),
            make_user_msg("q2"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        assert_eq!(
            roles(&selected),
            vec!["system", "assistant", "tool", "user", "assistant", "user"]
        );

        let tool_idx = selected.iter().position(|m| m.role == "tool").unwrap();
        let user_idx = selected.iter().position(|m| m.role == "user").unwrap();
        assert!(tool_idx < user_idx);
    }

    #[tokio::test]
    async fn test_handoff_context_and_tools_ordering() {
        // Test ordering: system -> context -> agentic_tools -> conversation
        let messages = vec![
            make_system_msg("s"),
            make_context_file_msg("file.rs", "content"),
            make_user_msg("q1"),
            make_assistant_with_tool_call("tc1", "subagent"),
            make_tool_msg("tc1", "subagent results"),
            make_assistant_msg("after subagent"),
            make_user_msg("q2"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_all_opened_context: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);
        // Order: system -> context_file -> assistant(tool_call) -> tool -> user -> assistant -> user
        assert_eq!(
            roles(&selected),
            vec![
                "system",
                "context_file",
                "assistant",
                "tool",
                "user",
                "assistant",
                "user"
            ]
        );

        let cf_idx = selected
            .iter()
            .position(|m| m.role == "context_file")
            .unwrap();
        let tool_idx = selected.iter().position(|m| m.role == "tool").unwrap();
        let user_idx = selected.iter().position(|m| m.role == "user").unwrap();

        assert!(cf_idx < tool_idx);
        assert!(tool_idx < user_idx);
    }

    #[tokio::test]
    async fn test_handoff_multiple_preserved_tools() {
        // Test that multiple preserved tools are all included
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q1"),
            make_assistant_with_tool_call("tc1", "deep_research"),
            make_tool_msg("tc1", "research 1"),
            make_assistant_msg("a1"),
            make_assistant_with_tool_call("tc2", "strategic_planning"),
            make_tool_msg("tc2", "planning 1"),
            make_assistant_msg("a2"),
            make_user_msg("q2"),
            make_assistant_msg("final"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            include_agentic_tools: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (selected, _, _) = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id")
            .await
            .unwrap();

        assert_system_prefix(&selected);

        // Both preserved tool pairs should be included
        let tool_count = selected.iter().filter(|m| m.role == "tool").count();
        assert_eq!(tool_count, 2, "Both preserved tools should be included");

        let tool_ids: Vec<_> = selected
            .iter()
            .filter(|m| m.role == "tool")
            .map(|m| m.tool_call_id.as_str())
            .collect();
        assert!(tool_ids.contains(&"tc1"));
        assert!(tool_ids.contains(&"tc2"));
    }

    #[tokio::test]
    async fn test_handoff_no_summary_when_generate_summary_false() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q1"),
            make_assistant_msg("a1"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            llm_summary_for_excluded: true,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let result = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id").await;
        assert!(result.is_ok(), "Should succeed when generate_summary=false");
        let (_, _, llm_summary) = result.unwrap();
        assert!(
            llm_summary.is_none(),
            "No summary should be generated when generate_summary=false"
        );
    }

    #[tokio::test]
    async fn test_handoff_no_summary_when_option_disabled() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q1"),
            make_assistant_msg("a1"),
        ];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            llm_summary_for_excluded: false,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let result = handoff_select(&messages, &opts, gcx, true, "test-trajectory-id").await;
        assert!(
            result.is_ok(),
            "Should succeed when llm_summary_for_excluded=false"
        );
        let (_, _, llm_summary) = result.unwrap();
        assert!(
            llm_summary.is_none(),
            "No summary should be generated when option is disabled"
        );
    }

    #[tokio::test]
    async fn test_handoff_no_summary_when_empty_messages() {
        let messages = vec![make_system_msg("s")];
        let opts = HandoffOptions {
            include_last_user_plus: true,
            llm_summary_for_excluded: false,
            ..Default::default()
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let result = handoff_select(&messages, &opts, gcx, false, "test-trajectory-id").await;
        assert!(
            result.is_ok(),
            "Should succeed when only system messages exist"
        );
        let (_, _, llm_summary) = result.unwrap();
        assert!(
            llm_summary.is_none(),
            "No summary should be generated when option disabled"
        );
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
            usage: Some(crate::call_validation::ChatUsage {
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
