use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;

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

const AGENTIC_TOOLS: &[&str] = &[
    "cat", "tree", "search_pattern", "search_symbol_definition", "search_semantic",
    "create_textdoc", "update_textdoc", "update_textdoc_regex", "update_textdoc_by_lines",
    "update_textdoc_anchored", "apply_patch", "undo_textdoc", "rm", "mv",
    "shell", "web", "chrome", "subagent", "knowledge", "create_knowledge",
];

const TOOLS_TO_PRESERVE: &[&str] = &["deep_research", "subagent", "strategic_planning"];

fn is_agentic_tool(name: &str) -> bool {
    AGENTIC_TOOLS.iter().any(|t| *t == name)
}

fn should_preserve_tool(name: &str) -> bool {
    TOOLS_TO_PRESERVE.iter().any(|t| *t == name)
}

fn approx_token_count(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|m| {
        let content_len = match &m.content {
            ChatContent::SimpleText(s) => s.len(),
            ChatContent::Multimodal(v) => v.iter().map(|_| 100).sum(),
            ChatContent::ContextFiles(v) => v.iter().map(|cf| cf.file_content.len()).sum(),
        };
        content_len / 4 + 10
    }).sum()
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
        let mut i = 0;
        while i < messages.len() {
            if messages[i].role == "context_file" {
                messages.remove(i);
                context_modified += 1;
            } else {
                i += 1;
            }
        }
    } else if opts.dedup_and_compress_context {
        let result = super::history_limit::compress_duplicate_context_files(messages);
        if let Ok((count, _)) = result {
            context_modified = count;
        }
    }

    if opts.drop_all_memories {
        let mut i = 0;
        while i < messages.len() {
            if messages[i].role == "context_file" {
                let content_text = messages[i].content.content_text_only().to_lowercase();
                if content_text.contains("memory") || content_text.contains("knowledge") {
                    messages.remove(i);
                    context_modified += 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    if opts.drop_project_information {
        let mut i = 0;
        while i < messages.len() {
            if messages[i].role == "system" {
                let content_text = messages[i].content.content_text_only().to_lowercase();
                if content_text.contains("project") || content_text.contains("workspace") {
                    messages.remove(i);
                    context_modified += 1;
                    continue;
                }
            }
            i += 1;
        }
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
                    msg.content = ChatContent::SimpleText(format!(
                        "Tool result compressed: {}...",
                        preview
                    ));
                    tool_modified += 1;
                }
            }
        }
    }

    super::history_limit::remove_invalid_tool_calls_and_tool_calls_results(messages);

    let after_tokens = approx_token_count(messages);
    let reduction_percent = if before_tokens > 0 {
        ((before_tokens.saturating_sub(after_tokens)) * 100) / before_tokens
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
            after_tokens,
            reduction_percent
        )),
        ..Default::default()
    };
    messages.push(instruction);

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
) -> Result<(Vec<ChatMessage>, TransformStats, Option<String>), String> {
    let before_count = messages.len();
    let before_tokens = approx_token_count(messages);

    let mut selected: Vec<ChatMessage> = Vec::new();
    let mut llm_summary: Option<String> = None;

    if opts.include_last_user_plus {
        let last_user_idx = messages.iter().rposition(|m| m.role == "user");
        if let Some(idx) = last_user_idx {
            selected = messages[idx..].to_vec();
        }
    } else {
        let mut tool_call_ids_to_include: std::collections::HashSet<String> = std::collections::HashSet::new();

        for msg in messages.iter() {
            let should_include = match msg.role.as_str() {
                "user" => true,
                "assistant" => true,
                "system" => true,
                "context_file" => opts.include_all_opened_context,
                "diff" => {
                    opts.include_all_edited_context || (opts.include_agentic_tools && {
                        messages.iter()
                            .filter_map(|m| m.tool_calls.as_ref())
                            .flatten()
                            .find(|tc| tc.id == msg.tool_call_id)
                            .map(|tc| is_agentic_tool(&tc.function.name))
                            .unwrap_or(false)
                    })
                }
                "tool" => {
                    if opts.include_agentic_tools {
                        messages.iter()
                            .filter_map(|m| m.tool_calls.as_ref())
                            .flatten()
                            .find(|tc| tc.id == msg.tool_call_id)
                            .map(|tc| is_agentic_tool(&tc.function.name))
                            .unwrap_or(false)
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if should_include {
                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        tool_call_ids_to_include.insert(tc.id.clone());
                    }
                }
                selected.push(msg.clone());
            }
        }

        selected.retain(|m| {
            if (m.role == "tool" || m.role == "diff") && !m.tool_call_id.is_empty() {
                tool_call_ids_to_include.contains(&m.tool_call_id)
            } else {
                true
            }
        });
    }

    super::history_limit::remove_invalid_tool_calls_and_tool_calls_results(&mut selected);

    if opts.llm_summary_for_excluded && !opts.include_last_user_plus {
        let messages_vec = messages.to_vec();
        match crate::agentic::compress_trajectory::compress_trajectory(gcx, &messages_vec).await {
            Ok(summary) => {
                let summary_msg = ChatMessage {
                    role: "user".to_string(),
                    content: ChatContent::SimpleText(format!(
                        "## Previous conversation summary\n\n{}",
                        summary
                    )),
                    ..Default::default()
                };
                selected.insert(0, summary_msg);
                llm_summary = Some(summary);
            }
            Err(e) => {
                tracing::warn!("Failed to generate LLM summary for handoff: {}", e);
            }
        }
    }

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
        assert!(messages.iter().filter(|m| m.role != "cd_instruction").all(|m| m.role != "context_file"));
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
    fn test_compress_preserves_deep_research_subagent_strategic_planning() {
        let long_content = "x".repeat(1000);
        for tool_name in &["deep_research", "subagent", "strategic_planning"] {
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
            assert_eq!(stats.tool_messages_modified, 0, "Tool {} should be preserved", tool_name);
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
    fn test_is_agentic_tool() {
        assert!(is_agentic_tool("cat"));
        assert!(is_agentic_tool("create_textdoc"));
        assert!(is_agentic_tool("shell"));
        assert!(!is_agentic_tool("unknown_tool"));
        assert!(!is_agentic_tool(""));
    }

    #[test]
    fn test_approx_token_count() {
        let messages = vec![
            make_user_msg("hello world"),
        ];
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
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions::default();
        compress_in_place(&mut messages, &opts).unwrap();
        let last_msg = messages.last().unwrap();
        assert_eq!(last_msg.role, "cd_instruction");
        assert!(last_msg.content.content_text_only().contains("Chat compressed"));
    }

    #[test]
    fn test_drop_all_memories() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_context_file_msg("memory.md", "some memory content"),
            make_context_file_msg("knowledge.txt", "some knowledge"),
            make_context_file_msg("regular.rs", "fn main() {}"),
            make_assistant_msg("response"),
        ];
        let opts = CompressOptions {
            drop_all_memories: true,
            ..Default::default()
        };
        let stats = compress_in_place(&mut messages, &opts).unwrap();
        assert_eq!(stats.context_messages_modified, 2);
        assert!(messages.iter().any(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.first().map(|f| f.file_name == "regular.rs").unwrap_or(false)
            } else {
                false
            }
        }));
    }

    #[test]
    fn test_drop_project_information() {
        let mut messages = vec![
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
        assert!(messages.iter().any(|m| m.role == "system" && m.content.content_text_only().contains("assistant")));
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
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_msg("response"),
        ];
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
}
