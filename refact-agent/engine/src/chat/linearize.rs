use crate::call_validation::{ChatContent, ChatMessage};
use crate::scratchpads::multimodality::MultimodalElement;

const TOOL_APPENDABLE_ROLES: &[&str] = &["context_file", "plain_text", "cd_instruction"];
const TOOL_ROLES: &[&str] = &["tool", "diff"];
const MERGE_SEPARATOR: &str = "\n\n";

fn is_tool_appendable(role: &str) -> bool {
    TOOL_APPENDABLE_ROLES.contains(&role)
}

fn is_tool_role(role: &str) -> bool {
    TOOL_ROLES.contains(&role)
}

fn content_to_elements(content: &ChatContent) -> Vec<MultimodalElement> {
    match content {
        ChatContent::SimpleText(text) => {
            if text.is_empty() {
                vec![]
            } else {
                vec![MultimodalElement {
                    m_type: "text".to_string(),
                    m_content: text.clone(),
                }]
            }
        }
        ChatContent::Multimodal(elements) => {
            elements.iter().filter(|el| {
                !(el.is_text() && el.m_content.is_empty())
            }).cloned().collect()
        }
        ChatContent::ContextFiles(_) => {
            let text = content.content_text_only();
            if text.is_empty() {
                vec![]
            } else {
                vec![MultimodalElement {
                    m_type: "text".to_string(),
                    m_content: text,
                }]
            }
        }
    }
}

fn elements_to_content(elements: Vec<MultimodalElement>) -> ChatContent {
    if elements.is_empty() {
        return ChatContent::SimpleText(String::new());
    }

    if elements.iter().any(|el| !el.is_text()) {
        ChatContent::Multimodal(elements)
    } else {
        let text = elements
            .iter()
            .map(|el| el.m_content.as_str())
            .collect::<Vec<_>>()
            .join(MERGE_SEPARATOR);
        ChatContent::SimpleText(text)
    }
}

fn merge_user_like_group(group: Vec<ChatMessage>) -> ChatMessage {
    debug_assert!(!group.is_empty());

    if group.len() == 1 {
        let mut msg = group.into_iter().next().unwrap();
        if msg.role != "user" {
            msg.role = "user".to_string();
        }
        return msg;
    }

    let mut all_elements: Vec<MultimodalElement> = Vec::new();

    for msg in &group {
        let elements = content_to_elements(&msg.content);
        if elements.is_empty() {
            continue;
        }
        if !all_elements.is_empty() {
            let last_is_text = all_elements.last().map_or(false, |el| el.is_text());
            let next_is_text = elements.first().map_or(false, |el| el.is_text());
            if last_is_text && next_is_text {
                if let Some(last) = all_elements.last_mut() {
                    last.m_content.push_str(MERGE_SEPARATOR);
                    last.m_content.push_str(&elements[0].m_content);
                    all_elements.extend(elements.into_iter().skip(1));
                    continue;
                }
            }
        }
        all_elements.extend(elements);
    }

    let mut merged = group[0].clone();
    merged.role = "user".to_string();
    merged.content = elements_to_content(all_elements);
    merged.tool_calls = None;
    merged.tool_call_id = String::new();
    merged.thinking_blocks = None;
    merged.reasoning_content = None;
    merged
}

/// Appends content from a tool-appendable message (context_file, plain_text, cd_instruction)
/// into an existing tool/diff message's text content.
fn append_to_tool_message(tool_msg: &mut ChatMessage, appendable: &ChatMessage) {
    let extra_text = match &appendable.content {
        ChatContent::SimpleText(text) => text.clone(),
        ChatContent::ContextFiles(_) => appendable.content.content_text_only(),
        ChatContent::Multimodal(elements) => {
            elements.iter()
                .filter(|el| el.is_text())
                .map(|el| el.m_content.as_str())
                .collect::<Vec<_>>()
                .join(MERGE_SEPARATOR)
        }
    };
    if extra_text.is_empty() {
        return;
    }
    match &mut tool_msg.content {
        ChatContent::SimpleText(text) => {
            if !text.is_empty() {
                text.push_str(MERGE_SEPARATOR);
            }
            text.push_str(&extra_text);
        }
        _ => {
            let existing = tool_msg.content.content_text_only();
            let mut combined = existing;
            if !combined.is_empty() {
                combined.push_str(MERGE_SEPARATOR);
            }
            combined.push_str(&extra_text);
            tool_msg.content = ChatContent::SimpleText(combined);
        }
    }
}

/// Merges consecutive user-like messages and folds tool-appendable messages
/// (context_file, plain_text, cd_instruction) into preceding tool/diff messages
/// for cache-friendly LLM requests. Idempotent and deterministic.
///
/// Rules:
/// - context_file/plain_text/cd_instruction after tool/diff → appended to last tool msg
/// - consecutive user-like messages → merged into single "user" message
/// - real "user" message after tool → starts a new user group (not folded into tool)
pub fn linearize_thread_for_llm(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut result: Vec<ChatMessage> = Vec::new();
    let mut user_group: Vec<ChatMessage> = Vec::new();

    for msg in messages {
        if is_tool_appendable(&msg.role) {
            if !user_group.is_empty() {
                // Already accumulating user-like messages, keep accumulating
                user_group.push(msg.clone());
            } else if let Some(last) = result.last_mut() {
                if is_tool_role(&last.role) {
                    // Fold into the preceding tool/diff message
                    append_to_tool_message(last, msg);
                } else {
                    // After system/assistant/etc — start a user group
                    user_group.push(msg.clone());
                }
            } else {
                // First message in the thread
                user_group.push(msg.clone());
            }
        } else if msg.role == "user" {
            // Real user message always goes into user group
            user_group.push(msg.clone());
        } else {
            // Non-user-like role (system, assistant, tool, diff)
            if !user_group.is_empty() {
                result.push(merge_user_like_group(std::mem::take(&mut user_group)));
            }
            result.push(msg.clone());
        }
    }

    if !user_group.is_empty() {
        result.push(merge_user_like_group(user_group));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatMessage, ChatContent, ContextFile};
    use crate::scratchpads::multimodality::MultimodalElement;

    fn text_msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn text_msg_with_id(role: &str, text: &str, id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn context_file_msg(files: Vec<(&str, &str, usize, usize)>) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(
                files
                    .into_iter()
                    .map(|(name, content, l1, l2)| ContextFile {
                        file_name: name.to_string(),
                        file_content: content.to_string(),
                        line1: l1,
                        line2: l2,
                        ..Default::default()
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn multimodal_msg(role: &str, elements: Vec<(&str, &str)>) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: ChatContent::Multimodal(
                elements
                    .into_iter()
                    .map(|(t, c)| MultimodalElement {
                        m_type: t.to_string(),
                        m_content: c.to_string(),
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        text_msg("assistant", text)
    }

    fn tool_msg(text: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_no_merge_needed_simple_alternation() {
        let msgs = vec![
            text_msg("system", "You are helpful"),
            text_msg("user", "Hello"),
            assistant_msg("Hi there"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "assistant");
    }

    #[test]
    fn test_merge_consecutive_user_messages() {
        let msgs = vec![
            text_msg("system", "You are helpful"),
            text_msg("user", "First part"),
            text_msg("user", "Second part"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(
            result[1].content.content_text_only(),
            "First part\n\nSecond part"
        );
    }

    #[test]
    fn test_merge_context_file_with_user() {
        let msgs = vec![
            text_msg("system", "You are helpful"),
            context_file_msg(vec![("src/main.rs", "fn main() {}", 1, 1)]),
            text_msg("user", "Fix the bug"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        let text = result[1].content.content_text_only();
        assert!(text.contains("src/main.rs:1-1"));
        assert!(text.contains("fn main() {}"));
        assert!(text.contains("Fix the bug"));
    }

    #[test]
    fn test_merge_multiple_context_files_and_user() {
        let msgs = vec![
            text_msg("system", "System prompt"),
            context_file_msg(vec![("a.rs", "aaa", 1, 3)]),
            context_file_msg(vec![("b.rs", "bbb", 1, 5)]),
            text_msg("plain_text", "Some plain text"),
            text_msg("user", "Do something"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "user");
        let text = result[1].content.content_text_only();
        assert!(text.contains("a.rs:1-3"));
        assert!(text.contains("b.rs:1-5"));
        assert!(text.contains("Some plain text"));
        assert!(text.contains("Do something"));
    }

    #[test]
    fn test_merge_cd_instruction_with_user() {
        let msgs = vec![
            text_msg("system", "System"),
            text_msg("cd_instruction", "cd /project"),
            text_msg("user", "List files"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "user");
        assert_eq!(
            result[1].content.content_text_only(),
            "cd /project\n\nList files"
        );
    }



    #[test]
    fn test_no_merge_across_assistant_boundary() {
        let msgs = vec![
            text_msg("user", "First question"),
            assistant_msg("First answer"),
            context_file_msg(vec![("c.rs", "code", 1, 10)]),
            text_msg("user", "Second question"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 3); // user, assistant, user(merged)
        assert_eq!(result[0].role, "user");
        assert_eq!(result[1].role, "assistant");
        assert_eq!(result[2].role, "user");
        let merged_text = result[2].content.content_text_only();
        assert!(merged_text.contains("c.rs:1-10"));
        assert!(merged_text.contains("Second question"));
    }



    #[test]
    fn test_tool_messages_not_merged() {
        let msgs = vec![
            text_msg("user", "Do something"),
            assistant_msg("Calling tool"),
            tool_msg("Tool result 1", "call_1"),
            tool_msg("Tool result 2", "call_2"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 4);
        assert_eq!(result[2].role, "tool");
        assert_eq!(result[2].tool_call_id, "call_1");
        assert_eq!(result[3].role, "tool");
        assert_eq!(result[3].tool_call_id, "call_2");
    }

    #[test]
    fn test_tool_loop_pattern_preserved() {
        let msgs = vec![
            text_msg("system", "System"),
            text_msg("user", "Fix bug"),
            assistant_msg("Let me check"),
            tool_msg("file contents", "call_1"),
            assistant_msg("Now I'll patch"),
            tool_msg("patch applied", "call_2"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 6);
        // Exact same structure — nothing to merge
        for (i, (orig, lin)) in msgs.iter().zip(result.iter()).enumerate() {
            assert_eq!(orig.role, lin.role, "Role mismatch at index {}", i);
        }
    }



    #[test]
    fn test_idempotency_simple() {
        let msgs = vec![
            text_msg("system", "System"),
            context_file_msg(vec![("a.rs", "aaa", 1, 3)]),
            text_msg("user", "Hello"),
            assistant_msg("Hi"),
        ];
        let first = linearize_thread_for_llm(&msgs);
        let second = linearize_thread_for_llm(&first);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content.content_text_only(), b.content.content_text_only());
        }
    }

    #[test]
    fn test_idempotency_complex() {
        let msgs = vec![
            text_msg("system", "System"),
            context_file_msg(vec![("a.rs", "aaa", 1, 3)]),
            context_file_msg(vec![("b.rs", "bbb", 4, 6)]),
            text_msg("cd_instruction", "cd /tmp"),
            text_msg("user", "Do it"),
            assistant_msg("OK"),
            text_msg("plain_text", "Extra info"),
            text_msg("user", "More"),
        ];
        let first = linearize_thread_for_llm(&msgs);
        let second = linearize_thread_for_llm(&first);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content.content_text_only(), b.content.content_text_only());
        }
    }



    #[test]
    fn test_multimodal_image_preserved() {
        let msgs = vec![
            text_msg("system", "System"),
            multimodal_msg("user", vec![("text", "Look at this"), ("image/png", "base64data")]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        match &result[1].content {
            ChatContent::Multimodal(elements) => {
                assert_eq!(elements.len(), 2);
                assert!(elements[0].is_text());
                assert!(elements[1].is_image());
            }
            _ => panic!("Expected Multimodal content"),
        }
    }

    #[test]
    fn test_merge_text_with_multimodal() {
        let msgs = vec![
            text_msg("system", "System"),
            text_msg("user", "Context info"),
            multimodal_msg("user", vec![("text", "Look at this"), ("image/png", "imgdata")]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        match &result[1].content {
            ChatContent::Multimodal(elements) => {
                // "Context info" + separator + "Look at this" merged into one text, then image
                assert_eq!(elements.len(), 2);
                assert!(elements[0].is_text());
                assert!(elements[0].m_content.contains("Context info"));
                assert!(elements[0].m_content.contains("Look at this"));
                assert!(elements[1].is_image());
                assert_eq!(elements[1].m_content, "imgdata");
            }
            _ => panic!("Expected Multimodal content"),
        }
    }

    #[test]
    fn test_merge_context_file_with_multimodal() {
        let msgs = vec![
            context_file_msg(vec![("x.rs", "code", 1, 5)]),
            multimodal_msg("user", vec![("text", "Describe"), ("image/png", "img")]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 1);
        match &result[0].content {
            ChatContent::Multimodal(elements) => {
                assert!(elements[0].is_text());
                assert!(elements[0].m_content.contains("x.rs:1-5"));
                assert!(elements[0].m_content.contains("Describe"));
                assert!(elements[1].is_image());
            }
            _ => panic!("Expected Multimodal content"),
        }
    }



    #[test]
    fn test_empty_user_message_skipped_in_merge() {
        let msgs = vec![
            text_msg("system", "System"),
            text_msg("user", ""),
            text_msg("user", "Real content"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].content.content_text_only(), "Real content");
    }

    #[test]
    fn test_all_empty_user_messages() {
        let msgs = vec![
            text_msg("system", "System"),
            text_msg("user", ""),
            text_msg("context_file", ""),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "user");
    }



    #[test]
    fn test_empty_input() {
        let result = linearize_thread_for_llm(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_single_user_message() {
        let msgs = vec![text_msg("user", "Hello")];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content.content_text_only(), "Hello");
    }

    #[test]
    fn test_single_context_file_becomes_user() {
        let msgs = vec![context_file_msg(vec![("f.rs", "code", 1, 1)])];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
    }

    #[test]
    fn test_system_not_merged_with_user() {
        let msgs = vec![
            text_msg("system", "System prompt"),
            text_msg("user", "User message"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
    }

    #[test]
    fn test_diff_messages_not_merged() {
        let msgs = vec![
            assistant_msg("Patching"),
            ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText("diff content".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "diff");
    }

    #[test]
    fn test_message_id_preserved_from_first() {
        let msgs = vec![
            text_msg_with_id("user", "First", "msg-001"),
            text_msg_with_id("user", "Second", "msg-002"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message_id, "msg-001");
    }



    #[test]
    fn test_deterministic_output() {
        let msgs = vec![
            text_msg("system", "System"),
            context_file_msg(vec![
                ("a.rs", "fn a() {}", 1, 1),
                ("b.rs", "fn b() {}", 1, 1),
            ]),
            text_msg("cd_instruction", "cd /project"),
            text_msg("user", "Fix everything"),
        ];

        // Run multiple times, output must be identical
        let result1 = linearize_thread_for_llm(&msgs);
        let result2 = linearize_thread_for_llm(&msgs);
        let result3 = linearize_thread_for_llm(&msgs);

        for i in 0..result1.len() {
            assert_eq!(result1[i].content.content_text_only(), result2[i].content.content_text_only());
            assert_eq!(result2[i].content.content_text_only(), result3[i].content.content_text_only());
        }
    }



    #[test]
    fn test_realistic_agentic_flow() {
        // Simulates: system + project context + knowledge + user question
        // then tool loop with strict alternation
        let msgs = vec![
            text_msg("system", "You are a coding assistant"),
            context_file_msg(vec![("project/README.md", "# Project", 1, 1)]),
            context_file_msg(vec![("src/lib.rs", "pub mod auth;", 1, 1)]),
            text_msg("user", "Fix the auth bug"),
            assistant_msg("Let me look at the auth module"),
            tool_msg("pub fn login() { ... }", "call_1"),
            assistant_msg("I see the issue, let me patch it"),
            tool_msg("Patch applied successfully", "call_2"),
        ];
        let result = linearize_thread_for_llm(&msgs);

        // Expected: system, user(merged 3), assistant, tool, assistant, tool
        assert_eq!(result.len(), 6);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "assistant");
        assert_eq!(result[3].role, "tool");
        assert_eq!(result[4].role, "assistant");
        assert_eq!(result[5].role, "tool");

        // The merged user message should contain all context + question
        let user_text = result[1].content.content_text_only();
        assert!(user_text.contains("project/README.md"));
        assert!(user_text.contains("src/lib.rs"));
        assert!(user_text.contains("Fix the auth bug"));
    }


    /// 197x: user→user (conversation continuation / handoff messages)
    #[test]
    fn test_real_user_user_handoff() {
        let msgs = vec![
            text_msg("system", "You are Refact Agent"),
            context_file_msg(vec![("knowledge.md", "prior context", 1, 4)]),
            text_msg("user", "## Previous conversation summary\n\n<analysis>User requested auth fix</analysis>"),
            text_msg("user", "The previous trajectory abc-123. Continue from where you stopped."),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2); // system + merged user
        assert_eq!(result[1].role, "user");
        let text = result[1].content.content_text_only();
        assert!(text.contains("knowledge.md"));
        assert!(text.contains("Previous conversation summary"));
        assert!(text.contains("Continue from where you stopped"));
    }

    /// 55x: cf, cf, user, user (context files + multi-part user input)
    #[test]
    fn test_real_cf_cf_user_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            context_file_msg(vec![("AGENTS.md", "agent config", 1, 10)]),
            context_file_msg(vec![("knowledge.md", "cached knowledge", 1, 4)]),
            text_msg("user", "## Previous conversation summary\n\nUser worked on providers"),
            text_msg("user", "The previous trajectory xyz-789. Continue."),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        let text = result[1].content.content_text_only();
        assert!(text.contains("AGENTS.md"));
        assert!(text.contains("knowledge.md"));
        assert!(text.contains("Previous conversation"));
        assert!(text.contains("Continue"));
    }

    /// 36x: cf, cf only (context-only without final user message)
    #[test]
    fn test_real_cf_cf_no_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            context_file_msg(vec![("file1.rs", "code1", 1, 10)]),
            context_file_msg(vec![("file2.rs", "code2", 1, 5)]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2); // system + merged user
        assert_eq!(result[1].role, "user");
        let text = result[1].content.content_text_only();
        assert!(text.contains("file1.rs"));
        assert!(text.contains("file2.rs"));
    }

    /// 7x: cf, cf, user, user, cd_instruction (mode transition instruction)
    #[test]
    fn test_real_cf_cf_user_user_cd_instruction() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            context_file_msg(vec![("knowledge.md", "cached", 1, 4)]),
            context_file_msg(vec![("prepare.rs", "code", 1, 100)]),
            text_msg("user", "## Summary\n\nUser wants providers page"),
            text_msg("user", "Continue from trajectory abc."),
            text_msg("cd_instruction", "💿 Now confirm the plan with the user"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "user");
        let text = result[1].content.content_text_only();
        assert!(text.contains("knowledge.md"));
        assert!(text.contains("prepare.rs"));
        assert!(text.contains("Summary"));
        assert!(text.contains("Continue from trajectory"));
        assert!(text.contains("💿 Now confirm the plan"));
    }

    /// 3x: user, cf, user (interleaved — user asks, context injected, user continues)
    #[test]
    fn test_real_interleaved_user_cf_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "avoid comments though"),
            context_file_msg(vec![("trajectory.json", "{}", 1, 100)]),
            text_msg("user", "also add tests to backend"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        let text = result[1].content.content_text_only();
        assert!(text.contains("avoid comments"));
        assert!(text.contains("trajectory.json"));
        assert!(text.contains("also add tests"));
    }

    /// 2x: context_file with string content (not ContextFiles variant!)
    #[test]
    fn test_real_context_file_as_string() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("context_file", "some pre-formatted file content here"),
            text_msg("user", "explain this"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        let text = result[1].content.content_text_only();
        assert!(text.contains("some pre-formatted file content"));
        assert!(text.contains("explain this"));
    }

    /// 4x: 9+ context_files in a row (heavy context injection)
    #[test]
    fn test_real_many_context_files() {
        let mut msgs = vec![text_msg("system", "system prompt")];
        for i in 0..9 {
            msgs.push(context_file_msg(vec![
                (&format!("file{i}.rs"), &format!("content {i}"), 1, 10),
            ]));
        }
        msgs.push(text_msg("user", "Fix everything"));
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2); // system + one merged user
        let text = result[1].content.content_text_only();
        for i in 0..9 {
            assert!(text.contains(&format!("file{i}.rs")), "Missing file{i}.rs");
        }
        assert!(text.contains("Fix everything"));
    }

    /// 235x: tool → context_file, user → assistant (mid-conversation context injection)
    #[test]
    fn test_real_tool_then_cf_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "Find the bug"),
            assistant_msg("Let me search"),
            tool_msg("found: auth.rs has issue", "call_1"),
            context_file_msg(vec![("auth.rs", "fn login() {}", 1, 5)]),
            text_msg("user", "Fix that function"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, tool(+cf), user
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "assistant");
        assert_eq!(result[3].role, "tool");
        let tool_text = result[3].content.content_text_only();
        assert!(tool_text.contains("found: auth.rs has issue"));
        assert!(tool_text.contains("auth.rs"));
        assert_eq!(result[4].role, "user");
        assert_eq!(result[4].content.content_text_only(), "Fix that function");
    }

    /// tool → cf, cd, cf, user: cf+cd+cf fold into tool, user stays separate
    #[test]
    fn test_real_tool_then_cf_cd_cf_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "start"),
            assistant_msg("calling tool"),
            tool_msg("tool output here", "call_1"),
            context_file_msg(vec![("file1.rs", "code1", 1, 10)]),
            text_msg("cd_instruction", "💿 Review complete"),
            context_file_msg(vec![("file2.rs", "code2", 1, 5)]),
            text_msg("user", "now fix it"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, tool(+cf+cd+cf), user
        assert_eq!(result.len(), 5);
        assert_eq!(result[3].role, "tool");
        let tool_text = result[3].content.content_text_only();
        assert!(tool_text.contains("tool output here"));
        assert!(tool_text.contains("file1.rs"));
        assert!(tool_text.contains("💿 Review complete"));
        assert!(tool_text.contains("file2.rs"));
        assert_eq!(result[4].role, "user");
        assert_eq!(result[4].content.content_text_only(), "now fix it");
    }

    /// 4x: plain_text role with directory tree content
    #[test]
    fn test_real_plain_text_directory_tree() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "show me the project"),
            assistant_msg("Here's the tree"),
            text_msg("plain_text", "/\n  home/\n    svakhreev/\n      projects/\n        refact/"),
            text_msg("user", "now explain the structure"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, merged(plain_text+user)
        assert_eq!(result.len(), 4);
        assert_eq!(result[3].role, "user");
        let text = result[3].content.content_text_only();
        assert!(text.contains("home/"));
        assert!(text.contains("now explain"));
    }

    /// 16x: trailing cf, cf, user with no assistant after (END of thread)
    #[test]
    fn test_real_trailing_sequence_no_assistant() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            context_file_msg(vec![("k1.md", "knowledge 1", 1, 4)]),
            context_file_msg(vec![("k2.md", "knowledge 2", 1, 4)]),
            text_msg("user", "Start working on the task"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "user");
        let text = result[1].content.content_text_only();
        assert!(text.contains("k1.md"));
        assert!(text.contains("k2.md"));
        assert!(text.contains("Start working"));
    }

    /// 280x: system → cf, cf, cf, user → assistant (most common initial pattern)
    #[test]
    fn test_real_system_3cf_user_assistant() {
        let msgs = vec![
            text_msg("system", "You are Refact Agent"),
            context_file_msg(vec![("AGENTS.md", "agent guidelines", 1, 50)]),
            context_file_msg(vec![("knowledge1.md", "prior work", 1, 4)]),
            context_file_msg(vec![("knowledge2.md", "more context", 1, 4)]),
            text_msg("user", "implement the feature"),
            assistant_msg("I'll start by analyzing the codebase"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 3); // system, merged user, assistant
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "assistant");
        let text = result[1].content.content_text_only();
        assert!(text.contains("AGENTS.md"));
        assert!(text.contains("knowledge1.md"));
        assert!(text.contains("knowledge2.md"));
        assert!(text.contains("implement the feature"));
    }

    /// 87x: tool → user, user → assistant (multi-user after tool)
    #[test]
    fn test_real_tool_then_user_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "start"),
            assistant_msg("checking"),
            tool_msg("result data", "call_1"),
            text_msg("user", "## Previous conversation summary\n\nWorked on auth"),
            text_msg("user", "Continue from trajectory abc."),
            assistant_msg("continuing"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, tool, merged(user+user), assistant
        assert_eq!(result.len(), 6);
        assert_eq!(result[4].role, "user");
        let text = result[4].content.content_text_only();
        assert!(text.contains("Previous conversation"));
        assert!(text.contains("Continue from trajectory"));
    }

    /// Complex real-world: mixed cf, user, cf, user, cd_instruction sequence
    #[test]
    fn test_real_complex_mixed_sequence() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            context_file_msg(vec![("AGENTS.md", "config", 1, 10)]),
            context_file_msg(vec![("knowledge.md", "cached", 1, 4)]),
            text_msg("user", "## Previous conversation\n\nWorked on UI"),
            text_msg("user", "Continue from trajectory xyz."),
            context_file_msg(vec![("new_knowledge.md", "fresh context", 1, 4)]),
            text_msg("user", "one more crazy example"),
            text_msg("user", "just make sure that this madness is really fixed"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2); // system + one big merged user
        let text = result[1].content.content_text_only();
        assert!(text.contains("AGENTS.md"));
        assert!(text.contains("knowledge.md"));
        assert!(text.contains("Previous conversation"));
        assert!(text.contains("Continue from trajectory"));
        assert!(text.contains("new_knowledge.md"));
        assert!(text.contains("one more crazy example"));
        assert!(text.contains("madness is really fixed"));
    }

    /// 1x: diff → context_file, user → assistant (7x in real data)
    #[test]
    fn test_real_diff_then_cf_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "start"),
            assistant_msg("making changes"),
            ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText("applied patch".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            context_file_msg(vec![("updated.rs", "new code", 1, 10)]),
            text_msg("user", "looks good, continue"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, diff(+cf), user
        assert_eq!(result.len(), 5);
        assert_eq!(result[3].role, "diff");
        let diff_text = result[3].content.content_text_only();
        assert!(diff_text.contains("applied patch"));
        assert!(diff_text.contains("updated.rs"));
        assert_eq!(result[4].role, "user");
        assert_eq!(result[4].content.content_text_only(), "looks good, continue");
    }

    /// cd_instruction alone (4x: cd_instruction, user)
    #[test]
    fn test_real_cd_instruction_then_user() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "start"),
            assistant_msg("done"),
            tool_msg("result", "call_1"),
            text_msg("cd_instruction", "💿 Review complete. Present findings to the user."),
            text_msg("user", "what did you find?"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, tool(+cd), user
        assert_eq!(result.len(), 5);
        let tool_text = result[3].content.content_text_only();
        assert!(tool_text.contains("result"));
        assert!(tool_text.contains("💿 Review complete"));
        assert_eq!(result[4].role, "user");
        assert_eq!(result[4].content.content_text_only(), "what did you find?");
    }

    /// Multimodal user messages (406x: user with list content in real data)
    #[test]
    fn test_real_multimodal_user_with_context_file() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            context_file_msg(vec![("screenshot.md", "description of UI", 1, 5)]),
            multimodal_msg("user", vec![
                ("text", "Here's a screenshot of the bug"),
                ("image/png", "base64encodeddata"),
            ]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 2); // system + merged user
        // Should be Multimodal since it contains an image
        match &result[1].content {
            ChatContent::Multimodal(elements) => {
                // Should have: text from cf, text from user, image from user
                let texts: Vec<_> = elements.iter().filter(|e| e.is_text()).collect();
                let images: Vec<_> = elements.iter().filter(|e| e.is_image()).collect();
                assert!(!texts.is_empty());
                assert_eq!(images.len(), 1);
                let all_text: String = texts.iter().map(|e| e.m_content.as_str()).collect::<Vec<_>>().join(" ");
                assert!(all_text.contains("screenshot.md"));
                assert!(all_text.contains("screenshot of the bug"));
            }
            _ => panic!("Expected Multimodal content when merging text + image"),
        }
    }

    /// Core trajectory pattern: tool → context_file folds into tool
    #[test]
    fn test_tool_cf_folds_into_tool() {
        let msgs = vec![
            text_msg("system", "system prompt"),
            text_msg("user", "do something"),
            assistant_msg("calling tool"),
            tool_msg("tool result", "call_1"),
            context_file_msg(vec![("file.rs", "fn main() {}", 1, 5)]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, tool(+cf)
        assert_eq!(result.len(), 4);
        assert_eq!(result[3].role, "tool");
        let text = result[3].content.content_text_only();
        assert!(text.contains("tool result"));
        assert!(text.contains("file.rs"));
    }

    /// Multiple context_files after tool all fold in
    #[test]
    fn test_tool_multiple_cf_fold() {
        let msgs = vec![
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("found files", "call_1"),
            context_file_msg(vec![("a.rs", "aaa", 1, 3)]),
            context_file_msg(vec![("b.rs", "bbb", 1, 3)]),
            context_file_msg(vec![("c.rs", "ccc", 1, 3)]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // user, assistant, tool(+cf+cf+cf)
        assert_eq!(result.len(), 3);
        assert_eq!(result[2].role, "tool");
        let text = result[2].content.content_text_only();
        assert!(text.contains("found files"));
        assert!(text.contains("a.rs"));
        assert!(text.contains("b.rs"));
        assert!(text.contains("c.rs"));
    }

    /// Repeating tool loop: tool→cf→assistant→tool→cf→assistant
    #[test]
    fn test_repeating_tool_cf_loop() {
        let msgs = vec![
            text_msg("system", "sys"),
            text_msg("user", "start"),
            assistant_msg("step 1"),
            tool_msg("result 1", "call_1"),
            context_file_msg(vec![("f1.rs", "code1", 1, 5)]),
            assistant_msg("step 2"),
            tool_msg("result 2", "call_2"),
            context_file_msg(vec![("f2.rs", "code2", 1, 5)]),
            assistant_msg("done"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, user, assistant, tool(+cf), assistant, tool(+cf), assistant
        assert_eq!(result.len(), 7);
        assert_eq!(result[3].role, "tool");
        assert!(result[3].content.content_text_only().contains("f1.rs"));
        assert_eq!(result[4].role, "assistant");
        assert_eq!(result[5].role, "tool");
        assert!(result[5].content.content_text_only().contains("f2.rs"));
        assert_eq!(result[6].role, "assistant");
    }

    /// Real trajectory pattern: 12x tool→cf→assistant repeating
    #[test]
    fn test_long_tool_cf_loop() {
        let mut msgs = vec![
            text_msg("system", "sys"),
            context_file_msg(vec![("init.rs", "init", 1, 1)]),
            text_msg("user", "fix bugs"),
        ];
        for i in 0..12 {
            msgs.push(assistant_msg(&format!("step {i}")));
            msgs.push(tool_msg(&format!("result {i}"), &format!("call_{i}")));
            msgs.push(context_file_msg(vec![(&format!("f{i}.rs"), &format!("code{i}"), 1, 5)]));
        }
        let result = linearize_thread_for_llm(&msgs);
        // system, user(init.rs+user), then 12x (assistant, tool(+cf)) = 2 + 24 = 26
        assert_eq!(result.len(), 26);
        for i in 0..12 {
            let tool_idx = 3 + i * 2;
            assert_eq!(result[tool_idx].role, "tool", "idx {tool_idx}");
            let text = result[tool_idx].content.content_text_only();
            assert!(text.contains(&format!("result {i}")));
            assert!(text.contains(&format!("f{i}.rs")));
        }
    }

    /// tool → user does NOT fold (user stays separate)
    #[test]
    fn test_tool_then_real_user_no_fold() {
        let msgs = vec![
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("result", "call_1"),
            text_msg("user", "thanks, now do more"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // user, assistant, tool, user
        assert_eq!(result.len(), 4);
        assert_eq!(result[2].role, "tool");
        assert_eq!(result[2].content.content_text_only(), "result");
        assert_eq!(result[3].role, "user");
        assert_eq!(result[3].content.content_text_only(), "thanks, now do more");
    }

    /// tool → cf → user: cf folds into tool, user stays separate
    #[test]
    fn test_tool_cf_then_user_separate() {
        let msgs = vec![
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("result", "call_1"),
            context_file_msg(vec![("x.rs", "code", 1, 5)]),
            text_msg("user", "now fix it"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // user, assistant, tool(+cf), user
        assert_eq!(result.len(), 4);
        assert_eq!(result[2].role, "tool");
        assert!(result[2].content.content_text_only().contains("x.rs"));
        assert_eq!(result[3].role, "user");
        assert_eq!(result[3].content.content_text_only(), "now fix it");
    }

    /// tool_call_id preserved when folding into tool
    #[test]
    fn test_tool_cf_preserves_tool_call_id() {
        let msgs = vec![
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("result", "call_abc123"),
            context_file_msg(vec![("x.rs", "code", 1, 5)]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result[2].role, "tool");
        assert_eq!(result[2].tool_call_id, "call_abc123");
    }

    /// context_file after assistant (not tool) → user group as before
    #[test]
    fn test_cf_after_assistant_becomes_user() {
        let msgs = vec![
            text_msg("system", "sys"),
            assistant_msg("hello"),
            context_file_msg(vec![("x.rs", "code", 1, 5)]),
            text_msg("user", "continue"),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // system, assistant, user(cf+user)
        assert_eq!(result.len(), 3);
        assert_eq!(result[2].role, "user");
        let text = result[2].content.content_text_only();
        assert!(text.contains("x.rs"));
        assert!(text.contains("continue"));
    }

    /// Empty context_file after tool doesn't corrupt tool content
    #[test]
    fn test_tool_empty_cf_no_corruption() {
        let msgs = vec![
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("result", "call_1"),
            text_msg("context_file", ""),
        ];
        let result = linearize_thread_for_llm(&msgs);
        assert_eq!(result.len(), 3);
        assert_eq!(result[2].role, "tool");
        assert_eq!(result[2].content.content_text_only(), "result");
    }

    /// Idempotency with tool→cf folding
    #[test]
    fn test_idempotency_tool_cf() {
        let msgs = vec![
            text_msg("system", "sys"),
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("result", "call_1"),
            context_file_msg(vec![("x.rs", "code", 1, 5)]),
            text_msg("user", "more"),
        ];
        let first = linearize_thread_for_llm(&msgs);
        let second = linearize_thread_for_llm(&first);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content.content_text_only(), b.content.content_text_only());
        }
    }

    /// Multiple tools then cf folds into LAST tool
    #[test]
    fn test_multiple_tools_cf_folds_into_last() {
        let msgs = vec![
            text_msg("user", "go"),
            assistant_msg("ok"),
            tool_msg("result A", "call_1"),
            tool_msg("result B", "call_2"),
            context_file_msg(vec![("x.rs", "code", 1, 5)]),
        ];
        let result = linearize_thread_for_llm(&msgs);
        // user, assistant, tool(A), tool(B+cf)
        assert_eq!(result.len(), 4);
        assert_eq!(result[2].content.content_text_only(), "result A");
        assert!(result[3].content.content_text_only().contains("result B"));
        assert!(result[3].content.content_text_only().contains("x.rs"));
    }
}
