use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use refact_core::chat_types::{ChatContent, ChatMessage, ContextFile, SamplingParameters};
use refact_core::custom_error::first_n_chars;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextPressure {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone)]
pub struct ContextBudgetReport {
    pub used_tokens_estimate: usize,
    pub effective_n_ctx: usize,
    pub remaining_estimate: isize,
    pub pressure: ContextPressure,
}

pub fn pressure_for_used_tokens(used_tokens: usize, effective_n_ctx: usize) -> ContextPressure {
    if effective_n_ctx == 0 {
        return ContextPressure::Low;
    }

    let pct_used = used_tokens.saturating_mul(100) / effective_n_ctx;
    if pct_used < 70 {
        ContextPressure::Low
    } else if pct_used < 85 {
        ContextPressure::Medium
    } else if pct_used < 95 {
        ContextPressure::High
    } else {
        ContextPressure::Critical
    }
}

pub fn compute_context_budget(
    messages: &[ChatMessage],
    effective_n_ctx: usize,
) -> ContextBudgetReport {
    let used_tokens_estimate = crate::trajectory_ops::approx_token_count(messages);
    let remaining_estimate = (effective_n_ctx as isize) - (used_tokens_estimate as isize);
    let pressure = pressure_for_used_tokens(used_tokens_estimate, effective_n_ctx);
    ContextBudgetReport {
        used_tokens_estimate,
        effective_n_ctx,
        remaining_estimate,
        pressure,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionStrength {
    Absent,
    Low,
    Medium,
    High,
}

pub fn remove_invalid_tool_calls_and_tool_calls_results(messages: &mut Vec<ChatMessage>) {
    let tool_call_ids: HashSet<_> = messages
        .iter()
        .filter(|m| {
            (m.role == "tool" || m.role == "diff" || m.role == "context_file")
                && !m.tool_call_id.is_empty()
        })
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
        let is_tool_result = m.role == "tool" || m.role == "diff" || m.role == "context_file";
        if is_tool_result && !m.tool_call_id.is_empty() && !tool_call_ids.contains(&m.tool_call_id)
        {
            tracing::warn!("removing tool result with no tool_call: {:?}", m);
            false
        } else {
            true
        }
    });

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

/// Relocate tool/diff/context_file results so each assistant's results follow it
/// contiguously. Operates on wire-preparation copies only; transcripts that are
/// already contiguous come out byte-identical.
pub fn relocate_tool_results_after_their_calls(messages: &mut Vec<ChatMessage>) {
    let declares_call = |message: &ChatMessage, call_id: &str| -> bool {
        message.role == "assistant"
            && message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|call| call.id == call_id))
    };
    let has_any_calls = messages
        .iter()
        .any(|message| message.tool_calls.as_ref().is_some_and(|c| !c.is_empty()));
    if !has_any_calls {
        return;
    }
    // Anchor each result to the nearest declaring assistant (preceding preferred, to
    // tolerate reused tool-call ids), falling back to the nearest following one.
    let owner_for = |messages: &[ChatMessage], idx: usize, call_id: &str| -> Option<usize> {
        messages[..idx]
            .iter()
            .rposition(|message| declares_call(message, call_id))
            .or_else(|| {
                messages[idx + 1..]
                    .iter()
                    .position(|message| declares_call(message, call_id))
                    .map(|offset| idx + 1 + offset)
            })
    };
    let mut owners: Vec<Option<usize>> = vec![None; messages.len()];
    for idx in 0..messages.len() {
        let message = &messages[idx];
        if matches!(message.role.as_str(), "tool" | "diff" | "context_file")
            && !message.tool_call_id.is_empty()
        {
            owners[idx] = owner_for(messages, idx, &message.tool_call_id);
        }
    }
    if owners.iter().all(|owner| owner.is_none()) {
        return;
    }
    let mut results_by_owner: HashMap<usize, Vec<ChatMessage>> = HashMap::new();
    let mut kept: Vec<(usize, ChatMessage)> = Vec::with_capacity(messages.len());
    for (idx, message) in messages.drain(..).enumerate() {
        match owners[idx] {
            Some(owner_idx) => {
                results_by_owner.entry(owner_idx).or_default().push(message);
            }
            None => kept.push((idx, message)),
        }
    }
    for (original_idx, message) in kept {
        messages.push(message);
        if let Some(results) = results_by_owner.remove(&original_idx) {
            messages.extend(results);
        }
    }
}

pub fn is_content_duplicate(
    current_content: &str,
    current_line1: usize,
    current_line2: usize,
    first_content: &str,
    first_line1: usize,
    first_line2: usize,
) -> bool {
    let lines_overlap = first_line1 <= current_line2 && first_line2 >= current_line1;
    if !lines_overlap {
        return false;
    }
    if current_content.is_empty() || first_content.is_empty() {
        return false;
    }
    if first_content.contains(current_content) || current_content.contains(first_content) {
        return true;
    }
    let first_lines: HashSet<&str> = first_content
        .lines()
        .filter(|x| !x.starts_with("..."))
        .collect();
    let current_lines: HashSet<&str> = current_content
        .lines()
        .filter(|x| !x.starts_with("..."))
        .collect();
    let intersect_count = first_lines.intersection(&current_lines).count();

    let current_in_first = !current_lines.is_empty() && intersect_count >= current_lines.len();
    let first_in_current = !first_lines.is_empty() && intersect_count >= first_lines.len();

    current_in_first || first_in_current
}

pub fn compress_duplicate_context_files(
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

    let mut files_by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, file) in all_files.iter().enumerate() {
        files_by_name
            .entry(file.file_name.clone())
            .or_insert_with(Vec::new)
            .push(i);
    }

    for (filename, indices) in &files_by_name {
        if indices.len() <= 1 {
            continue;
        }

        // A newer re-attachment of the same path with the same line range supersedes
        // older copies: the file may have changed, so the freshest view is the truth.
        let mut newest_by_range: HashMap<(usize, usize), usize> = HashMap::new();
        for &i in indices {
            let range_key = (all_files[i].line1, all_files[i].line2);
            let entry = newest_by_range.entry(range_key).or_insert(i);
            if all_files[i].msg_idx > all_files[*entry].msg_idx {
                *entry = i;
            }
        }
        for &i in indices {
            let range_key = (all_files[i].line1, all_files[i].line2);
            if newest_by_range.get(&range_key) != Some(&i) {
                all_files[i].is_compressed = true;
                tracing::info!(
                    "Stage 0: Marking for compression - superseded by newer attachment of {} at message index {}",
                    filename,
                    all_files[i].msg_idx
                );
            }
        }

        let Some(best_idx) = indices
            .iter()
            .copied()
            .filter(|&i| !all_files[i].is_compressed)
            .max_by(|&a, &b| {
                let size_cmp = all_files[a].content_len.cmp(&all_files[b].content_len);
                if size_cmp == std::cmp::Ordering::Equal {
                    all_files[a].msg_idx.cmp(&all_files[b].msg_idx)
                } else {
                    size_cmp
                }
            })
        else {
            continue;
        };
        let best_msg_idx = all_files[best_idx].msg_idx;
        preserve_messages[best_msg_idx] = true;

        tracing::info!(
            "Stage 0: File {} - preserving best occurrence at message index {} ({} bytes)",
            filename,
            best_msg_idx,
            all_files[best_idx].content_len
        );

        for &curr_idx in indices {
            if curr_idx == best_idx || all_files[curr_idx].is_compressed {
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
                    let summary = format!(" Duplicate files compressed: '{}' files were shown earlier in the conversation history. Do not ask for these files again.", compressed_files_str);
                    messages[file.msg_idx].content = ChatContent::SimpleText(summary);
                    if messages[file.msg_idx].tool_call_id.is_empty() {
                        messages[file.msg_idx].role = "cd_instruction".to_string();
                    }
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
    let high_budget_tools = vec!["write"];
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
                        format!("{tokens_msg} Try to make changes one by one (ie using `patch()`).")
                    } else {
                        format!("{tokens_msg} Change your strategy.")
                    }
                } else {
                    "".to_string()
                };

                let incorrect_reasons_concat = incorrect_reasons.join("\n");
                message.role = "cd_instruction".to_string();
                message.content = ChatContent::SimpleText(format!(" Previous tool calls are not valid: {incorrect_reasons_concat}.\n{extra_message}"));
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
    if messages.is_empty() {
        return Err("Invalid chat history: no messages present".to_string());
    }
    let has_prompt_anchor = messages.iter().any(|msg| {
        matches!(
            msg.role.as_str(),
            "system" | "user" | "event" | "plan" | "goal"
        )
    });
    if !has_prompt_anchor {
        return Err(
            "Invalid chat history: must have at least one message of role 'system', 'user', 'event', 'plan', or 'goal'"
                .to_string(),
        );
    }

    if !matches!(
        messages[0].role.as_str(),
        "system" | "user" | "event" | "plan" | "goal"
    ) {
        return Err(format!(
            "Invalid chat history: first message must be 'system', 'user', 'event', 'plan', or 'goal', got '{}'",
            messages[0].role
        ));
    }

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

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                if !tool_calls.is_empty() {
                    for tc in tool_calls {
                        let mut found = false;
                        for later_msg in messages.iter().skip(idx + 1) {
                            if (later_msg.role == "tool"
                                || later_msg.role == "diff"
                                || later_msg.role == "context_file")
                                && later_msg.tool_call_id == tc.id
                            {
                                found = true;
                                break;
                            }
                            if matches!(later_msg.role.as_str(), "user" | "assistant" | "system") {
                                break;
                            }
                        }
                        if !found {
                            return Err(format!(
                                "Assistant message at index {} has a tool call id '{}' that is unresponded before the next conversation turn (no contiguous tool message with that id)",
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

pub fn validate_chat_history(messages: &Vec<ChatMessage>) -> Result<Vec<ChatMessage>, String> {
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
    relocate_tool_results_after_their_calls(&mut mutable_messages);
    validate_chat_history_owned(mutable_messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

    fn make_context_file_msg(filename: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: filename.to_string(),
                file_content: content.to_string(),
                line1: 1,
                line2: 10,
                ..Default::default()
            }]),
            ..Default::default()
        }
    }

    fn plain_user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn assistant_declaring_call(call_id: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("calling".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn tool_result_msg(call_id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: call_id.to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn orphaned_context_file_results_are_removed() {
        let mut answering_dead_call = make_context_file_msg("src/a.rs", "content a");
        answering_dead_call.tool_call_id = "dead-call".to_string();
        let free_context = make_context_file_msg("src/b.rs", "content b");
        let mut messages = vec![plain_user_msg("q"), answering_dead_call, free_context];

        remove_invalid_tool_calls_and_tool_calls_results(&mut messages);

        assert_eq!(messages.len(), 2);
        assert!(messages
            .iter()
            .all(|message| message.tool_call_id.is_empty()));
    }

    #[test]
    fn relocate_moves_stray_results_back_to_their_call() {
        let mut messages = vec![
            plain_user_msg("q"),
            assistant_declaring_call("tc-1"),
            plain_user_msg("interleaved"),
            tool_result_msg("tc-1", "result"),
        ];

        relocate_tool_results_after_their_calls(&mut messages);

        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "user"]);
    }

    #[test]
    fn relocate_keeps_contiguous_transcripts_identical() {
        let original = vec![
            plain_user_msg("q"),
            assistant_declaring_call("tc-1"),
            tool_result_msg("tc-1", "result"),
            plain_user_msg("next"),
        ];
        let mut messages = original.clone();

        relocate_tool_results_after_their_calls(&mut messages);

        assert_eq!(
            serde_json::to_value(&messages).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
    }

    #[test]
    fn validation_rejects_result_separated_by_user_turn() {
        let mut answering = make_context_file_msg("src/a.rs", "content");
        answering.tool_call_id = "tc-1".to_string();
        let messages = vec![
            plain_user_msg("q"),
            assistant_declaring_call("tc-1"),
            plain_user_msg("interleaved"),
            answering,
        ];

        assert!(validate_chat_history(&messages).is_err());
    }

    #[test]
    fn fix_and_limit_repairs_result_separated_by_user_turn() {
        let mut answering = make_context_file_msg("src/a.rs", "content");
        answering.tool_call_id = "tc-1".to_string();
        let messages = vec![
            plain_user_msg("q"),
            assistant_declaring_call("tc-1"),
            plain_user_msg("interleaved"),
            answering,
        ];
        let mut sampling = SamplingParameters::default();

        let fixed = fix_and_limit_messages_history(&messages, &mut sampling)
            .expect("relocation must repair the pair");

        let roles: Vec<&str> = fixed.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "context_file", "user"]);
    }

    #[test]
    fn newer_same_range_attachment_supersedes_older_copy() {
        let older = make_context_file_msg("src/main.rs", "fn main() {}\nfn helper() {}\n");
        let newer = make_context_file_msg("src/main.rs", "fn main() {}\n");
        let mut messages = vec![older, plain_user_msg("edited the file"), newer];

        let (compressed_count, _) =
            compress_duplicate_context_files(&mut messages).expect("dedup must succeed");

        assert_eq!(compressed_count, 1);
        assert!(!messages[0]
            .content
            .content_text_only()
            .contains("fn helper()"));
        assert!(messages[2]
            .content
            .content_text_only()
            .contains("fn main()"));
    }

    fn make_tool_msg(tool_call_id: &str, content: &str, failed: Option<bool>) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_failed: failed,
            ..Default::default()
        }
    }

    fn make_user_msg_basic(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_event_msg_basic(content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "system_notice",
                "source": "test.history_limit",
                "payload": {},
            }),
        );
        ChatMessage {
            role: "event".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn make_goal_msg_basic(content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "goal".to_string(),
            serde_json::json!({
                "version": 1,
                "active": true,
                "status": "active",
            }),
        );
        ChatMessage {
            role: "goal".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn validate_chat_history_allows_event_first_history() {
        let mut sampling = SamplingParameters::default();
        let messages = vec![make_event_msg_basic("synthetic prompt")];

        let result = fix_and_limit_messages_history(&messages, &mut sampling).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "event");
    }

    #[test]
    fn validate_chat_history_allows_goal_first_history() {
        let mut sampling = SamplingParameters::default();
        let messages = vec![make_goal_msg_basic("finish the task")];

        let result = fix_and_limit_messages_history(&messages, &mut sampling).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "goal");
    }

    #[test]
    fn test_compute_context_budget_low_pressure() {
        let messages = vec![make_user_msg_basic("hello world")];
        let report = compute_context_budget(&messages, 10_000);
        assert_eq!(report.pressure, ContextPressure::Low);
        assert!(report.used_tokens_estimate > 0);
        assert_eq!(report.effective_n_ctx, 10_000);
        assert!(report.remaining_estimate > 0);
    }

    #[test]
    fn test_compute_context_budget_medium_pressure() {
        // 2800 chars -> 2800/4+10 = 710 tokens; 710/1000 = 71% -> Medium
        let text = "x".repeat(2_800);
        let messages = vec![make_user_msg_basic(&text)];
        let report = compute_context_budget(&messages, 1_000);
        assert_eq!(report.pressure, ContextPressure::Medium);
    }

    #[test]
    fn test_compute_context_budget_high_pressure() {
        // 3400 chars -> 3400/4+10 = 860 tokens; 860/1000 = 86% -> High
        let text = "x".repeat(3_400);
        let messages = vec![make_user_msg_basic(&text)];
        let report = compute_context_budget(&messages, 1_000);
        assert_eq!(report.pressure, ContextPressure::High);
    }

    #[test]
    fn test_compute_context_budget_critical_pressure() {
        // 3800 chars -> 3800/4+10 = 960 tokens; 960/1000 = 96% -> Critical
        let text = "x".repeat(3_800);
        let messages = vec![make_user_msg_basic(&text)];
        let report = compute_context_budget(&messages, 1_000);
        assert_eq!(report.pressure, ContextPressure::Critical);
    }

    #[test]
    fn test_compute_context_budget_zero_n_ctx() {
        let messages = vec![make_user_msg_basic("hello")];
        let report = compute_context_budget(&messages, 0);
        assert_eq!(report.pressure, ContextPressure::Low);
        assert_eq!(report.effective_n_ctx, 0);
    }

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
    fn test_context_file_with_matching_id_satisfies_tool_call() {
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
        assert_eq!(
            messages.len(),
            2,
            "assistant with context_file response should be kept, got roles: {:?}",
            messages.iter().map(|m| &m.role).collect::<Vec<_>>()
        );
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[1].role, "context_file");
    }

    #[test]
    fn test_validate_accepts_context_file_as_tool_call_response() {
        let messages = vec![
            make_user_msg_basic("question"),
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_read".to_string(),
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
                tool_call_id: "call_read".to_string(),
                content: ChatContent::SimpleText("file content".to_string()),
                ..Default::default()
            },
        ];
        let mut sampling = SamplingParameters::default();
        let result = fix_and_limit_messages_history(&messages, &mut sampling);
        assert!(
            result.is_ok(),
            "context_file should satisfy tool call: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn test_replace_broken_tool_call_messages_converts_garbage_args_to_cd_instruction() {
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_1".to_string(),
                index: Some(0),
                function: ChatToolFunction {
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
            content: ChatContent::SimpleText("hello".to_string()),
            ..Default::default()
        }];
        let mut sampling = SamplingParameters::default();
        let result = fix_and_limit_messages_history(&messages, &mut sampling).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
    }
    #[test]
    fn test_dedup_keeps_context_file_role_for_tool_call_responses() {
        let mut answered = make_context_file_msg("src/dup.rs", "line1\nline2\nline3");
        answered.tool_call_id = "call_read".to_string();
        let standalone = make_context_file_msg("src/dup.rs", "line1\nline2\nline3\nline4");
        let mut messages = vec![answered, standalone];

        let (count, _) = compress_duplicate_context_files(&mut messages).unwrap();

        assert_eq!(count, 1);
        assert_eq!(messages[0].role, "context_file");
        assert_eq!(messages[0].tool_call_id, "call_read");
        assert!(messages[0]
            .content
            .content_text_only()
            .contains("Duplicate files compressed"));
        assert_eq!(messages[1].role, "context_file");
    }

    #[test]
    fn test_dedup_swaps_role_for_unanswered_duplicates() {
        let small = make_context_file_msg("src/dup.rs", "line1\nline2\nline3");
        let large = make_context_file_msg("src/dup.rs", "line1\nline2\nline3\nline4");
        let mut messages = vec![small, large];

        let (count, _) = compress_duplicate_context_files(&mut messages).unwrap();

        assert_eq!(count, 1);
        assert_eq!(messages[0].role, "cd_instruction");
    }

    #[test]
    fn test_compute_context_budget_counts_tool_call_arguments() {
        let plain = vec![make_user_msg_basic("short")];
        let mut with_args = plain.clone();
        with_args.push(ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_big".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "write".to_string(),
                    arguments: "x".repeat(40_000),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        });

        let baseline = compute_context_budget(&plain, 100_000).used_tokens_estimate;
        let with_args_estimate = compute_context_budget(&with_args, 100_000).used_tokens_estimate;

        assert!(with_args_estimate > baseline + 9_000);
    }
}
