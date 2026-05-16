pub use refact_chat_history::trajectory_ops::*;

use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use refact_core::chat_types::ChatMessage;
use crate::global_context::GlobalContext;

pub async fn handoff_select(
    messages: &[ChatMessage],
    opts: &HandoffOptions,
    gcx: Arc<ARwLock<GlobalContext>>,
    generate_summary: bool,
    trajectory_id: &str,
) -> Result<(Vec<ChatMessage>, TransformStats, Option<String>), String> {
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
        use refact_core::chat_types::ChatContent;
        let all_files: Vec<refact_core::chat_types::ContextFile> = messages
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
            use refact_core::chat_types::ChatContent;
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
                    if TOOLS_TO_PRESERVE.iter().any(|t| *t == tc.function.name) {
                        preserved_tool_ids.insert(tc.id.clone());
                    }
                }
            }
        }

        for msg in messages.iter() {
            if let Some(ref tool_calls) = msg.tool_calls {
                let preserved_calls: Vec<_> = tool_calls
                    .iter()
                    .filter(|tc| TOOLS_TO_PRESERVE.iter().any(|t| *t == tc.function.name))
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

    let (conversation, excluded) = handoff_conversation_and_excluded(
        messages,
        opts,
        system_prefix_len,
        start_idx,
        &edited_tool_ids,
    );

    let mut llm_summary: Option<String> = None;
    let mut summary_msg: Option<ChatMessage> = None;

    if opts.llm_summary_for_excluded && generate_summary {
        let excluded_sanitized = sanitize_messages_for_new_thread(&excluded);

        if !excluded_sanitized.is_empty() {
            let summary =
                crate::agentic::compress_trajectory::compress_trajectory(gcx, &excluded_sanitized)
                    .await
                    .map_err(|e| format!("Failed to generate summary: {}", e))?;
            use refact_core::chat_types::ChatContent;
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

    use refact_core::chat_types::ChatContent;
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
    use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction, ContextFile};

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
        assert_eq!(
            roles(&selected),
            vec!["system", "user", "assistant", "user"]
        );
    }

    #[tokio::test]
    async fn test_handoff_preserved_tool_pair_included() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "research"),
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
    async fn test_handoff_delegate_preserved() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "delegate"),
            make_tool_msg("tc1", "delegate results"),
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
    async fn test_handoff_plan_preserved() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "plan"),
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
    async fn test_handoff_review_preserved() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q"),
            make_assistant_with_tool_call("tc1", "review"),
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

        let cf_count = selected.iter().filter(|m| m.role == "context_file").count();
        assert_eq!(
            cf_count, 1,
            "All context files should be bundled into one message"
        );

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
            make_assistant_with_tool_call("tc1", "patch"),
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
            make_assistant_with_tool_call("tc1", "patch"),
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
        assert_eq!(
            roles(&selected),
            vec!["system", "user", "assistant", "user"]
        );
    }

    #[tokio::test]
    async fn test_handoff_preserved_tools_before_conversation() {
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q1"),
            make_assistant_with_tool_call("tc1", "research"),
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
        let messages = vec![
            make_system_msg("s"),
            make_context_file_msg("file.rs", "content"),
            make_user_msg("q1"),
            make_assistant_with_tool_call("tc1", "delegate"),
            make_tool_msg("tc1", "delegate results"),
            make_assistant_msg("after delegate"),
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
        let messages = vec![
            make_system_msg("s"),
            make_user_msg("q1"),
            make_assistant_with_tool_call("tc1", "research"),
            make_tool_msg("tc1", "research 1"),
            make_assistant_msg("a1"),
            make_assistant_with_tool_call("tc2", "plan"),
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
}
