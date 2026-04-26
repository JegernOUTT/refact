use std::collections::{HashMap, HashSet};
use serde_json::Value;
use serde::{Serialize, Deserialize};
use crate::call_validation::{ChatMessage, ChatContent, ContextFile, SamplingParameters};
use crate::nicer_logs::first_n_chars;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionStrength {
    Absent,
    Low,
    Medium,
    High,
}

pub(crate) fn remove_invalid_tool_calls_and_tool_calls_results(messages: &mut Vec<ChatMessage>) {
    let tool_call_ids: HashSet<_> = messages
        .iter()
        .filter(|m| (m.role == "tool" || m.role == "diff") && !m.tool_call_id.is_empty())
        .map(|m| &m.tool_call_id)
        .cloned()
        .collect();
    messages.retain(|m| {
        if let Some(tool_calls) = &m.tool_calls {
            let should_retain = tool_calls.iter().all(|tc| tool_call_ids.contains(&tc.id));
            if !should_retain {
                tracing::warn!(
                    "removing assistant message with unanswered tool tool_calls: {:?}",
                    tool_calls
                );
            }
            should_retain
        } else {
            true
        }
    });

    let tool_call_ids: HashSet<_> = messages
        .iter()
        .filter_map(|x| x.tool_calls.clone())
        .flatten()
        .map(|x| x.id)
        .collect();
    messages.retain(|m| {
        let is_tool_result = m.role == "tool" || m.role == "diff";
        if is_tool_result && !m.tool_call_id.is_empty() && !tool_call_ids.contains(&m.tool_call_id)
        {
            tracing::warn!("removing tool result with no tool_call: {:?}", m);
            false
        } else {
            true
        }
    });

    // Remove duplicate tool results - keep only the last occurrence of each tool_call_id
    // Anthropic API requires exactly one tool_result per tool_use
    // For file edit operations, "diff" role typically comes after "tool" and contains cleaner output
    // Only applies to actual tool results (role == "tool" or "diff"), not context_file markers
    let mut last_occurrence: HashMap<String, usize> = HashMap::new();
    for (i, m) in messages.iter().enumerate() {
        let is_tool_result = m.role == "tool" || m.role == "diff";
        if is_tool_result && !m.tool_call_id.is_empty() {
            last_occurrence.insert(m.tool_call_id.clone(), i);
        }
    }
    let indices_to_keep: HashSet<usize> = last_occurrence.values().cloned().collect();
    let mut current_idx = 0usize;
    messages.retain(|m| {
        let idx = current_idx;
        current_idx += 1;
        let is_tool_result = m.role == "tool" || m.role == "diff";
        if m.tool_call_id.is_empty() || !is_tool_result {
            true
        } else if indices_to_keep.contains(&idx) {
            true
        } else {
            tracing::warn!(
                "removing duplicate tool result (role={}) for tool_call_id: {}",
                m.role,
                m.tool_call_id
            );
            false
        }
    });
}

/// Determines if two file contents have a duplication relationship (one contains the other).
/// Returns true if either content is substantially contained in the other.
pub(crate) fn is_content_duplicate(
    current_content: &str,
    current_line1: usize,
    current_line2: usize,
    first_content: &str,
    first_line1: usize,
    first_line2: usize,
) -> bool {
    let lines_overlap = first_line1 <= current_line2 && first_line2 >= current_line1;
    // If line ranges don't overlap at all, it's definitely not a duplicate
    if !lines_overlap {
        return false;
    }
    // Consider empty contents are not duplicate
    if current_content.is_empty() || first_content.is_empty() {
        return false;
    }
    // Check if either content is entirely contained in the other (symmetric check)
    if first_content.contains(current_content) || current_content.contains(first_content) {
        return true;
    }
    // Check for substantial line overlap (either direction)
    let first_lines: HashSet<&str> = first_content
        .lines()
        .filter(|x| !x.starts_with("..."))
        .collect();
    let current_lines: HashSet<&str> = current_content
        .lines()
        .filter(|x| !x.starts_with("..."))
        .collect();
    let intersect_count = first_lines.intersection(&current_lines).count();

    // Either all of current's lines are in first, OR all of first's lines are in current
    let current_in_first = !current_lines.is_empty() && intersect_count >= current_lines.len();
    let first_in_current = !first_lines.is_empty() && intersect_count >= first_lines.len();

    current_in_first || first_in_current
}

/// Stage 0: Compress duplicate ContextFiles based on content comparison - keeping the LARGEST occurrence
pub(crate) fn compress_duplicate_context_files(
    messages: &mut Vec<ChatMessage>,
) -> Result<(usize, Vec<bool>), String> {
    #[derive(Debug, Clone)]
    struct ContextFileInfo {
        msg_idx: usize,
        cf_idx: usize,
        file_name: String,
        content: String,
        line1: usize,
        line2: usize,
        content_len: usize,
        is_compressed: bool,
    }

    // First pass: collect information about all context files
    let mut preserve_messages = vec![false; messages.len()];
    let mut all_files: Vec<ContextFileInfo> = Vec::new();
    for (msg_idx, msg) in messages.iter().enumerate() {
        if msg.role != "context_file" {
            continue;
        }
        let context_files: Vec<ContextFile> = match &msg.content {
            ChatContent::ContextFiles(files) => files.clone(),
            ChatContent::SimpleText(text) => match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        "Stage 0: Failed to parse ContextFile JSON at index {}: {}. Skipping.",
                        msg_idx,
                        e
                    );
                    continue;
                }
            },
            _ => {
                tracing::warn!(
                    "Stage 0: Unexpected content type for context_file at index {}. Skipping.",
                    msg_idx
                );
                continue;
            }
        };
        for (cf_idx, cf) in context_files.iter().enumerate() {
            all_files.push(ContextFileInfo {
                msg_idx,
                cf_idx,
                file_name: cf.file_name.clone(),
                content: cf.file_content.clone(),
                line1: cf.line1,
                line2: cf.line2,
                content_len: cf.file_content.len(),
                is_compressed: false,
            });
        }
    }

    // Group occurrences by file name
    let mut files_by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, file) in all_files.iter().enumerate() {
        files_by_name
            .entry(file.file_name.clone())
            .or_insert_with(Vec::new)
            .push(i);
    }

    // Process each file's occurrences - keep the LARGEST one (prefer earlier if tied)
    for (filename, indices) in &files_by_name {
        if indices.len() <= 1 {
            continue;
        }

        // Find the index with the largest content; if tied, prefer earlier message (smaller msg_idx)
        let best_idx = *indices
            .iter()
            .max_by(|&&a, &&b| {
                let size_cmp = all_files[a].content_len.cmp(&all_files[b].content_len);
                if size_cmp == std::cmp::Ordering::Equal {
                    // When sizes equal, prefer EARLIER occurrence (smaller msg_idx)
                    all_files[b].msg_idx.cmp(&all_files[a].msg_idx)
                } else {
                    size_cmp
                }
            })
            .unwrap();
        let best_msg_idx = all_files[best_idx].msg_idx;
        preserve_messages[best_msg_idx] = true;

        tracing::info!(
            "Stage 0: File {} - preserving best occurrence at message index {} ({} bytes)",
            filename,
            best_msg_idx,
            all_files[best_idx].content_len
        );

        // Mark all other occurrences that are duplicates (subsets) of the best one for compression
        for &curr_idx in indices {
            if curr_idx == best_idx {
                continue;
            }
            let current_msg_idx = all_files[curr_idx].msg_idx;
            let content_is_duplicate = is_content_duplicate(
                &all_files[curr_idx].content,
                all_files[curr_idx].line1,
                all_files[curr_idx].line2,
                &all_files[best_idx].content,
                all_files[best_idx].line1,
                all_files[best_idx].line2,
            );
            if content_is_duplicate {
                all_files[curr_idx].is_compressed = true;
                tracing::info!("Stage 0: Marking for compression - duplicate/subset of file {} at message index {} ({} bytes)", 
                    filename, current_msg_idx, all_files[curr_idx].content_len);
            } else {
                tracing::info!("Stage 0: Not compressing - unique content of file {} at message index {} (non-overlapping)", 
                    filename, current_msg_idx);
            }
        }
    }

    // Apply compressions to messages
    let mut compressed_count = 0;
    let mut modified_messages: HashSet<usize> = HashSet::new();
    for file in &all_files {
        if file.is_compressed && !modified_messages.contains(&file.msg_idx) {
            let context_files: Vec<ContextFile> = match &messages[file.msg_idx].content {
                ChatContent::ContextFiles(files) => files.clone(),
                ChatContent::SimpleText(text) => serde_json::from_str(text).unwrap_or_default(),
                _ => vec![],
            };

            let mut remaining_files = Vec::new();
            let mut compressed_files = Vec::new();

            for (cf_idx, cf) in context_files.iter().enumerate() {
                if all_files
                    .iter()
                    .any(|f| f.msg_idx == file.msg_idx && f.cf_idx == cf_idx && f.is_compressed)
                {
                    compressed_files.push(format!("{}", cf.file_name));
                } else {
                    remaining_files.push(cf.clone());
                }
            }

            if !compressed_files.is_empty() {
                let compressed_files_str = compressed_files.join(", ");
                if remaining_files.is_empty() {
                    let summary = format!("💿 Duplicate files compressed: '{}' files were shown earlier in the conversation history. Do not ask for these files again.", compressed_files_str);
                    messages[file.msg_idx].content = ChatContent::SimpleText(summary);
                    messages[file.msg_idx].role = "cd_instruction".to_string();
                    tracing::info!(
                        "Stage 0: Fully compressed ContextFile at index {}: all {} files removed",
                        file.msg_idx,
                        compressed_files.len()
                    );
                } else {
                    let new_content = serde_json::to_string(&remaining_files)
                        .expect("serialization of filtered ContextFiles failed");
                    messages[file.msg_idx].content = ChatContent::SimpleText(new_content);
                    tracing::info!("Stage 0: Partially compressed ContextFile at index {}: {} files removed, {} files kept", 
                                  file.msg_idx, compressed_files.len(), remaining_files.len());
                }

                compressed_count += compressed_files.len();
                modified_messages.insert(file.msg_idx);
            }
        }
    }

    Ok((compressed_count, preserve_messages))
}

fn replace_broken_tool_call_messages(
    messages: &mut Vec<ChatMessage>,
    sampling_parameters: &mut SamplingParameters,
    new_max_new_tokens: usize,
) {
    let high_budget_tools = vec!["create_textdoc"];
    let last_index_assistant = messages
        .iter()
        .rposition(|msg| msg.role == "assistant")
        .unwrap_or(0);
    for (i, message) in messages.iter_mut().enumerate() {
        if let Some(tool_calls) = &mut message.tool_calls {
            let incorrect_reasons = tool_calls
                .iter()
                .map(|tc| {
                    match serde_json::from_str::<HashMap<String, Value>>(&tc.function.arguments) {
                        Ok(_) => None,
                        Err(err) => Some(format!(
                            "broken {}({}): {}",
                            tc.function.name,
                            first_n_chars(&tc.function.arguments, 100),
                            err
                        )),
                    }
                })
                .filter_map(|x| x)
                .collect::<Vec<_>>();
            let has_high_budget_tools = tool_calls
                .iter()
                .any(|tc| high_budget_tools.contains(&tc.function.name.as_str()));
            if !incorrect_reasons.is_empty() {
                // Only increase max_new_tokens if this is the last message and it was truncated due to "length"
                let extra_message = if i == last_index_assistant
                    && message.finish_reason == Some("length".to_string())
                {
                    tracing::warn!(
                        "increasing `max_new_tokens` from {} to {}",
                        sampling_parameters.max_new_tokens,
                        new_max_new_tokens
                    );
                    let tokens_msg = if sampling_parameters.max_new_tokens < new_max_new_tokens {
                        sampling_parameters.max_new_tokens = new_max_new_tokens;
                        format!("The message was stripped (finish_reason=`length`), the tokens budget was too small for the tool calls. Increasing `max_new_tokens` to {new_max_new_tokens}.")
                    } else {
                        "The message was stripped (finish_reason=`length`), the tokens budget cannot fit those tool calls.".to_string()
                    };
                    if has_high_budget_tools {
                        format!("{tokens_msg} Try to make changes one by one (ie using `update_textdoc()`).")
                    } else {
                        format!("{tokens_msg} Change your strategy.")
                    }
                } else {
                    "".to_string()
                };

                let incorrect_reasons_concat = incorrect_reasons.join("\n");
                message.role = "cd_instruction".to_string();
                message.content = ChatContent::SimpleText(format!("💿 Previous tool calls are not valid: {incorrect_reasons_concat}.\n{extra_message}"));
                message.tool_calls = None;
                tracing::warn!(
                    "tool calls are broken, converting the tool call message to the `cd_instruction`:\n{:?}",
                    message.content.content_text_only()
                );
            }
        }
    }
}

fn validate_chat_history_slice(messages: &[ChatMessage]) -> Result<(), String> {
    // 1. Check that there is at least one message (and that at least one is "system" or "user")
    if messages.is_empty() {
        return Err("Invalid chat history: no messages present".to_string());
    }
    let has_system_or_user = messages
        .iter()
        .any(|msg| msg.role == "system" || msg.role == "user");
    if !has_system_or_user {
        return Err(
            "Invalid chat history: must have at least one message of role 'system' or 'user'"
                .to_string(),
        );
    }

    // 2. The first message must be system or user.
    if messages[0].role != "system" && messages[0].role != "user" {
        return Err(format!(
            "Invalid chat history: first message must be 'system' or 'user', got '{}'",
            messages[0].role
        ));
    }

    // 3. For every tool call in any message, verify its function arguments are parseable.
    for (msg_idx, msg) in messages.iter().enumerate() {
        if let Some(tool_calls) = &msg.tool_calls {
            for tc in tool_calls {
                if let Err(e) = tc.function.parse_args() {
                    return Err(format!(
                        "Message at index {} has an unparseable tool call arguments for tool '{}': {} (arguments: {})",
                        msg_idx, tc.function.name, e, tc.function.arguments));
                }
            }
        }
    }

    // 4. For each assistant message with nonempty tool_calls,
    //    check that every tool call id mentioned is later (i.e. at a higher index) answered by a tool message.
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                if !tool_calls.is_empty() {
                    for tc in tool_calls {
                        // Look for a following "tool" message whose tool_call_id equals tc.id
                        let mut found = false;
                        for later_msg in messages.iter().skip(idx + 1) {
                            if (later_msg.role == "tool" || later_msg.role == "diff")
                                && later_msg.tool_call_id == tc.id
                            {
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(format!(
                                "Assistant message at index {} has a tool call id '{}' that is unresponded (no following tool message with that id)",
                                idx, tc.id
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_chat_history(messages: &Vec<ChatMessage>) -> Result<Vec<ChatMessage>, String> {
    validate_chat_history_slice(messages)?;
    Ok(messages.to_vec())
}

fn validate_chat_history_owned(messages: Vec<ChatMessage>) -> Result<Vec<ChatMessage>, String> {
    validate_chat_history_slice(&messages)?;
    Ok(messages)
}

pub fn fix_and_limit_messages_history(
    messages: &Vec<ChatMessage>,
    sampling_parameters_to_patch: &mut SamplingParameters,
) -> Result<Vec<ChatMessage>, String> {
    let mut mutable_messages = messages.clone();
    replace_broken_tool_call_messages(&mut mutable_messages, sampling_parameters_to_patch, 16000);
    remove_invalid_tool_calls_and_tool_calls_results(&mut mutable_messages);
    validate_chat_history_owned(mutable_messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};

    #[test]
    fn test_is_content_duplicate_overlapping_ranges() {
        let content1 = "line1\nline2\nline3";
        let content2 = "line2\nline3";
        assert!(is_content_duplicate(content1, 1, 3, content2, 2, 3));
    }

    #[test]
    fn test_is_content_duplicate_non_overlapping_ranges() {
        let content1 = "line1\nline2";
        let content2 = "line5\nline6";
        assert!(!is_content_duplicate(content1, 1, 2, content2, 5, 6));
    }

    #[test]
    fn test_is_content_duplicate_empty_content() {
        assert!(!is_content_duplicate("", 1, 10, "content", 1, 10));
        assert!(!is_content_duplicate("content", 1, 10, "", 1, 10));
    }

    #[test]
    fn test_is_content_duplicate_substring_containment() {
        let small = "line2\nline3";
        let large = "line1\nline2\nline3\nline4";
        assert!(is_content_duplicate(small, 2, 3, large, 1, 4));
        assert!(is_content_duplicate(large, 1, 4, small, 2, 3));
    }

    #[test]
    fn test_is_content_duplicate_exact_match() {
        let content = "line1\nline2";
        assert!(is_content_duplicate(content, 1, 2, content, 1, 2));
    }

    #[test]
    fn test_is_content_duplicate_ignores_ellipsis_lines() {
        let content1 = "...\nreal_line\n...";
        let content2 = "real_line";
        assert!(is_content_duplicate(content1, 1, 3, content2, 1, 1));
    }

    #[test]
    fn test_remove_invalid_tool_calls_removes_unanswered() {
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_1".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "test".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }];
        remove_invalid_tool_calls_and_tool_calls_results(&mut messages);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_remove_invalid_tool_calls_keeps_answered() {
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    index: Some(0),
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
                tool_call_id: "call_1".to_string(),
                content: ChatContent::SimpleText("result".to_string()),
                ..Default::default()
            },
        ];
        remove_invalid_tool_calls_and_tool_calls_results(&mut messages);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_remove_invalid_tool_calls_removes_orphan_results() {
        let mut messages = vec![ChatMessage {
            role: "tool".to_string(),
            tool_call_id: "nonexistent_call".to_string(),
            content: ChatContent::SimpleText("orphan result".to_string()),
            ..Default::default()
        }];
        remove_invalid_tool_calls_and_tool_calls_results(&mut messages);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_remove_invalid_tool_calls_keeps_last_duplicate() {
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    index: Some(0),
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
                tool_call_id: "call_1".to_string(),
                content: ChatContent::SimpleText("first result".to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                tool_call_id: "call_1".to_string(),
                content: ChatContent::SimpleText("second result (diff)".to_string()),
                ..Default::default()
            },
        ];
        remove_invalid_tool_calls_and_tool_calls_results(&mut messages);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "diff");
    }

    #[test]
    fn test_context_file_with_matching_id_does_not_satisfy_tool_call() {
        // A context_file message carrying the same tool_call_id must NOT count
        // as answering the assistant's tool call — only role=tool/diff qualifies.
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_x".to_string(),
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
                tool_call_id: "call_x".to_string(),
                content: ChatContent::SimpleText("file content".to_string()),
                ..Default::default()
            },
        ];
        remove_invalid_tool_calls_and_tool_calls_results(&mut messages);
        // The assistant message with the unanswered tool call must be removed.
        assert!(
            messages.iter().all(|m| m.role != "assistant"),
            "assistant with unanswered tool call should have been removed, got: {:?}",
            messages.iter().map(|m| &m.role).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_replace_broken_tool_call_messages_converts_garbage_args_to_cd_instruction() {
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                id: "call_1".to_string(),
                index: Some(0),
                function: crate::call_validation::ChatToolFunction {
                    name: "shell".to_string(),
                    arguments: "noise {\"command\":\"pwd\"} tail".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }];
        let mut sampling = SamplingParameters::default();

        replace_broken_tool_call_messages(&mut messages, &mut sampling, 16000);

        assert_eq!(messages[0].role, "cd_instruction");
        assert!(messages[0].tool_calls.is_none());
    }

    #[test]
    fn test_fix_valid_history_returns_correct_content() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: crate::call_validation::ChatContent::SimpleText("hello".to_string()),
            ..Default::default()
        }];
        let mut sampling = SamplingParameters::default();
        let result = fix_and_limit_messages_history(&messages, &mut sampling).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
    }
}
