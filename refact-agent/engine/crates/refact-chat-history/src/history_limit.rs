use std::collections::{BTreeMap, HashMap, HashSet};
use serde_json::{json, Value};
use serde::{Serialize, Deserialize};
use refact_core::chat_types::{ChatMessage, ChatContent, ContextFile, SamplingParameters};
use refact_core::custom_error::first_n_chars;
use uuid::Uuid;

use crate::compression_exemption::{event_source, event_subkind, exemption_for, CompressionExemption};

#[derive(Debug, Clone, PartialEq)]
pub struct Tier0CompactReport {
    pub context_files_deduped: usize,
    pub context_files_elided: usize,
    pub tool_outputs_truncated: usize,
    pub tokens_saved_estimate: usize,
}

fn extract_context_files_from_content(content: &ChatContent) -> Vec<ContextFile> {
    match content {
        ChatContent::ContextFiles(files) => files.clone(),
        ChatContent::SimpleText(text) => serde_json::from_str(text).unwrap_or_default(),
        _ => vec![],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactAggression {
    Standard,
    Aggressive,
}

impl CompactAggression {
    fn keep_recent_event_count(self) -> usize {
        match self {
            CompactAggression::Standard => 3,
            CompactAggression::Aggressive => 3,
        }
    }
}

#[derive(Debug)]
struct EventHistorySummary {
    insert_at: usize,
    source: String,
    subkind: String,
    count: usize,
}

fn estimated_message_tokens(msg: &ChatMessage) -> usize {
    msg.content.content_text_only().len() / 4 + 10
}

fn cutoff_excluding_never(messages: &[ChatMessage], preserve_last_n: usize) -> usize {
    if preserve_last_n == 0 {
        return messages.len();
    }
    let mut kept = 0usize;
    for (idx, msg) in messages.iter().enumerate().rev() {
        if exemption_for(msg) == CompressionExemption::Never {
            continue;
        }
        kept += 1;
        if kept >= preserve_last_n {
            return idx;
        }
    }
    0
}

fn event_history_attr_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn event_history_summary_message(summary: EventHistorySummary) -> ChatMessage {
    let source = summary.source;
    let subkind = summary.subkind;
    let count = summary.count;
    let escaped_source = event_history_attr_escape(&source);
    let mut extra = serde_json::Map::new();
    extra.insert(
        "event".to_string(),
        json!({
            "subkind": "summarization_marker",
            "source": source.clone(),
            "payload": {
                "summarized_subkind": subkind.clone(),
                "count": count,
            },
        }),
    );
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "event".to_string(),
        content: ChatContent::SimpleText(format!(
            "<event-history source=\"{}\">{} earlier {} events</event-history>",
            escaped_source, count, subkind
        )),
        extra,
        ..Default::default()
    }
}

fn compact_event_messages(
    messages: &mut Vec<ChatMessage>,
    preserve_last_n: usize,
    keep_recent_n: usize,
) -> usize {
    if messages.is_empty() {
        return 0;
    }

    let window_start = messages.len().saturating_sub(preserve_last_n);
    let mut keep = vec![true; messages.len()];
    let mut keep_recent_seen: HashMap<(String, String), usize> = HashMap::new();
    let mut summaries: HashMap<(String, String), EventHistorySummary> = HashMap::new();
    let mut most_recent_mode_switch_seen = false;
    let mut tokens_saved_estimate = 0usize;

    for (idx, msg) in messages.iter().enumerate().rev() {
        match exemption_for(msg) {
            CompressionExemption::Never | CompressionExemption::PreserveAnchor => {}
            CompressionExemption::KeepRecentN => {
                let source = event_source(msg).to_string();
                let subkind = event_subkind(msg).unwrap_or("event").to_string();
                let key = (source.clone(), subkind.clone());
                let seen = keep_recent_seen.entry(key.clone()).or_insert(0);
                if *seen < keep_recent_n {
                    *seen += 1;
                    continue;
                }
                keep[idx] = false;
                tokens_saved_estimate += estimated_message_tokens(msg);
                summaries
                    .entry(key)
                    .and_modify(|summary| {
                        summary.insert_at = summary.insert_at.min(idx);
                        summary.count += 1;
                    })
                    .or_insert(EventHistorySummary {
                        insert_at: idx,
                        source,
                        subkind,
                        count: 1,
                    });
            }
            CompressionExemption::PreserveWindow => {
                if idx < window_start {
                    keep[idx] = false;
                    tokens_saved_estimate += estimated_message_tokens(msg);
                }
            }
            CompressionExemption::DropOnAge => {
                if event_subkind(msg) == Some("mode_switch") && !most_recent_mode_switch_seen {
                    most_recent_mode_switch_seen = true;
                    continue;
                }
                if idx < window_start {
                    keep[idx] = false;
                    tokens_saved_estimate += estimated_message_tokens(msg);
                }
            }
        }
    }

    if keep.iter().all(|keep_message| *keep_message) && summaries.is_empty() {
        return 0;
    }

    let mut summaries: Vec<EventHistorySummary> = summaries.into_values().collect();
    summaries.sort_by(|left, right| {
        left.insert_at
            .cmp(&right.insert_at)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.subkind.cmp(&right.subkind))
    });
    let mut summaries_by_insert: BTreeMap<usize, Vec<ChatMessage>> = BTreeMap::new();
    for summary in summaries {
        summaries_by_insert
            .entry(summary.insert_at)
            .or_default()
            .push(event_history_summary_message(summary));
    }

    let old_messages = std::mem::take(messages);
    let mut compacted = Vec::with_capacity(old_messages.len());
    for (idx, msg) in old_messages.into_iter().enumerate() {
        if let Some(summary_messages) = summaries_by_insert.remove(&idx) {
            compacted.extend(summary_messages);
        }
        if keep[idx] {
            compacted.push(msg);
        }
    }
    for (_, summary_messages) in summaries_by_insert {
        compacted.extend(summary_messages);
    }
    *messages = compacted;

    tokens_saved_estimate
}

pub fn tier0_deterministic_compact(
    messages: &mut Vec<ChatMessage>,
    preserve_last_n: usize,
) -> Tier0CompactReport {
    tier0_deterministic_compact_with(messages, preserve_last_n, CompactAggression::Standard)
}

pub fn tier0_deterministic_compact_with(
    messages: &mut Vec<ChatMessage>,
    preserve_last_n: usize,
    aggression: CompactAggression,
) -> Tier0CompactReport {
    let mut context_files_deduped = 0usize;
    let mut context_files_elided = 0usize;
    let mut tool_outputs_truncated = 0usize;
    let mut tokens_saved_estimate = 0usize;

    tokens_saved_estimate += compact_event_messages(
        messages,
        preserve_last_n,
        aggression.keep_recent_event_count(),
    );

    let mut last_occurrence: HashMap<String, usize> = HashMap::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role != "context_file" {
            continue;
        }
        for cf in extract_context_files_from_content(&msg.content) {
            last_occurrence.insert(cf.file_name, i);
        }
    }

    for (i, msg) in messages.iter_mut().enumerate() {
        if msg.role != "context_file" {
            continue;
        }
        let mut files = extract_context_files_from_content(&msg.content);
        let mut modified = false;
        for cf in &mut files {
            if let Some(&last_idx) = last_occurrence.get(&cf.file_name) {
                if last_idx > i {
                    let original_tokens = cf.file_content.len() / 4 + 1;
                    tokens_saved_estimate += original_tokens;
                    context_files_deduped += 1;
                    cf.file_content = format!(
                        "\u{1f4ce} {} \u{2014} superseded by newer version in message #{}",
                        cf.file_name,
                        last_idx + 1
                    );
                    modified = true;
                }
            }
        }
        if modified {
            msg.content = ChatContent::ContextFiles(files);
        }
    }

    let (truncate_threshold, tool_preserve_last_n, ctx_file_keep_lines) = match aggression {
        CompactAggression::Standard => (200usize, preserve_last_n, None),
        CompactAggression::Aggressive => (80usize, preserve_last_n.min(2), Some(40usize)),
    };

    let tool_cutoff = cutoff_excluding_never(messages, tool_preserve_last_n);
    for (i, msg) in messages.iter_mut().enumerate() {
        if i >= tool_cutoff {
            break;
        }
        if msg.role != "tool" {
            continue;
        }
        if msg.preserve == Some(true) {
            continue;
        }
        if msg.tool_failed == Some(true) {
            continue;
        }
        let content_text = msg.content.content_text_only();
        if content_text.len() <= truncate_threshold {
            continue;
        }
        let estimated_tokens = content_text.len() / 4;
        tokens_saved_estimate += estimated_tokens;
        tool_outputs_truncated += 1;
        msg.content = ChatContent::SimpleText(format!(
            "[tool output truncated, was ~{} tokens]",
            estimated_tokens
        ));
    }

    if let Some(keep_lines) = ctx_file_keep_lines {
        let ctx_cutoff = cutoff_excluding_never(messages, preserve_last_n.min(4));
        for (i, msg) in messages.iter_mut().enumerate() {
            if i >= ctx_cutoff {
                break;
            }
            if msg.role != "context_file" {
                continue;
            }
            if msg.preserve == Some(true) {
                continue;
            }
            let mut files = extract_context_files_from_content(&msg.content);
            let mut modified = false;
            for cf in &mut files {
                let line_count = cf.file_content.lines().count();
                if line_count <= keep_lines {
                    continue;
                }
                let original_tokens = cf.file_content.len() / 4 + 1;
                let head: Vec<&str> = cf.file_content.lines().take(keep_lines / 2).collect();
                let tail: Vec<&str> = cf
                    .file_content
                    .lines()
                    .rev()
                    .take(keep_lines / 2)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let trimmed_tokens = (head.iter().map(|s| s.len()).sum::<usize>()
                    + tail.iter().map(|s| s.len()).sum::<usize>())
                    / 4
                    + 1;
                tokens_saved_estimate += original_tokens.saturating_sub(trimmed_tokens);
                context_files_elided += 1;
                cf.file_content = format!(
                    "{}\n...\n[\u{2702}\u{fe0f} {} lines elided under aggressive compaction]\n...\n{}",
                    head.join("\n"),
                    line_count.saturating_sub(head.len() + tail.len()),
                    tail.join("\n"),
                );
                modified = true;
            }
            if modified {
                msg.content = ChatContent::ContextFiles(files);
            }
        }
    }

    Tier0CompactReport {
        context_files_deduped,
        context_files_elided,
        tool_outputs_truncated,
        tokens_saved_estimate,
    }
}

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

pub fn compute_context_budget(
    messages: &[ChatMessage],
    effective_n_ctx: usize,
) -> ContextBudgetReport {
    let measured_messages: Vec<ChatMessage> = messages
        .iter()
        .filter(|msg| exemption_for(msg) != CompressionExemption::Never)
        .cloned()
        .collect();
    let used_tokens_estimate = crate::trajectory_ops::approx_token_count(&measured_messages);
    let remaining_estimate = (effective_n_ctx as isize) - (used_tokens_estimate as isize);
    let pressure = if effective_n_ctx == 0 {
        ContextPressure::Low
    } else {
        let pct_used = used_tokens_estimate.saturating_mul(100) / effective_n_ctx;
        if pct_used < 70 {
            ContextPressure::Low
        } else if pct_used < 85 {
            ContextPressure::Medium
        } else if pct_used < 95 {
            ContextPressure::High
        } else {
            ContextPressure::Critical
        }
    };
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

        let best_idx = *indices
            .iter()
            .max_by(|&&a, &&b| {
                let size_cmp = all_files[a].content_len.cmp(&all_files[b].content_len);
                if size_cmp == std::cmp::Ordering::Equal {
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
    let has_prompt_anchor = messages
        .iter()
        .any(|msg| matches!(msg.role.as_str(), "system" | "user" | "event" | "plan"));
    if !has_prompt_anchor {
        return Err(
            "Invalid chat history: must have at least one message of role 'system', 'user', 'event', or 'plan'"
                .to_string(),
        );
    }

    if !matches!(messages[0].role.as_str(), "system" | "user" | "event" | "plan") {
        return Err(format!(
            "Invalid chat history: first message must be 'system', 'user', 'event', or 'plan', got '{}'",
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

    #[test]
    fn validate_chat_history_allows_event_first_history() {
        let mut sampling = SamplingParameters::default();
        let messages = vec![make_event_msg_basic("synthetic prompt")];

        let result = fix_and_limit_messages_history(&messages, &mut sampling).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "event");
    }

    #[test]
    fn test_tier0_dedup_context_files_keeps_last() {
        let mut messages = vec![
            make_context_file_msg("src/main.rs", "first version content"),
            make_user_msg_basic("user message"),
            make_context_file_msg("src/main.rs", "second version content"),
            make_user_msg_basic("user message 2"),
            make_context_file_msg("src/main.rs", "third version content"),
        ];
        let report = tier0_deterministic_compact(&mut messages, 0);
        assert_eq!(report.context_files_deduped, 2);
        assert!(report.tokens_saved_estimate > 0);
        let last_ctx = messages
            .iter()
            .filter(|m| m.role == "context_file")
            .last()
            .unwrap();
        let files = extract_context_files_from_content(&last_ctx.content);
        assert_eq!(files[0].file_content, "third version content");
        let first_ctx = messages.iter().find(|m| m.role == "context_file").unwrap();
        let first_files = extract_context_files_from_content(&first_ctx.content);
        assert!(first_files[0].file_content.contains("superseded"));
        assert!(first_files[0].file_content.contains("message #5"));
    }

    #[test]
    fn test_tier0_dedup_different_files_not_deduped() {
        let mut messages = vec![
            make_context_file_msg("src/a.rs", "content a"),
            make_context_file_msg("src/b.rs", "content b"),
        ];
        let report = tier0_deterministic_compact(&mut messages, 0);
        assert_eq!(report.context_files_deduped, 0);
        assert_eq!(report.tokens_saved_estimate, 0);
    }

    #[test]
    fn test_tier0_truncate_old_tool_outputs() {
        let long_output = "x".repeat(500);
        let mut messages = vec![
            make_user_msg_basic("question"),
            make_tool_msg("tc1", &long_output, None),
            make_user_msg_basic("recent question"),
            make_tool_msg("tc2", &long_output, None),
        ];
        let report = tier0_deterministic_compact(&mut messages, 2);
        assert_eq!(report.tool_outputs_truncated, 1);
        assert!(messages[1]
            .content
            .content_text_only()
            .contains("truncated"));
        assert_eq!(messages[3].content.content_text_only(), long_output);
    }

    #[test]
    fn test_tier0_preserves_failed_tool_outputs() {
        let long_output = "x".repeat(500);
        let mut messages = vec![
            make_user_msg_basic("question"),
            make_tool_msg("tc1", &long_output, Some(true)),
            make_user_msg_basic("another"),
        ];
        let report = tier0_deterministic_compact(&mut messages, 0);
        assert_eq!(report.tool_outputs_truncated, 0);
        assert_eq!(messages[1].content.content_text_only(), long_output);
    }

    #[test]
    fn test_tier0_skips_preserved_tool_output() {
        let long_output = "x".repeat(500);
        let mut preserved = make_tool_msg("tc1", &long_output, None);
        preserved.preserve = Some(true);
        let mut messages = vec![
            make_user_msg_basic("question"),
            preserved,
            make_user_msg_basic("another"),
        ];
        let report = tier0_deterministic_compact(&mut messages, 0);
        assert_eq!(report.tool_outputs_truncated, 0);
        assert_eq!(messages[1].content.content_text_only(), long_output);
    }

    #[test]
    fn test_tier0_truncates_unpreserved_tool_output() {
        let long_output = "x".repeat(500);
        let mut messages = vec![
            make_user_msg_basic("question"),
            make_tool_msg("tc1", &long_output, None),
            make_user_msg_basic("another"),
        ];
        let report = tier0_deterministic_compact(&mut messages, 0);
        assert_eq!(report.tool_outputs_truncated, 1);
        assert!(messages[1]
            .content
            .content_text_only()
            .contains("truncated"));
    }

    #[test]
    fn test_tier0_preserves_short_tool_outputs() {
        let short_output = "short output";
        let mut messages = vec![
            make_user_msg_basic("question"),
            make_tool_msg("tc1", short_output, None),
        ];
        let report = tier0_deterministic_compact(&mut messages, 0);
        assert_eq!(report.tool_outputs_truncated, 0);
        assert_eq!(messages[1].content.content_text_only(), short_output);
    }

    #[test]
    fn test_tier0_aggressive_truncates_above_eighty_chars() {
        let medium_output = "x".repeat(120);
        let mut messages = vec![
            make_user_msg_basic("question"),
            make_tool_msg("tc1", &medium_output, None),
        ];

        let mut standard_messages = messages.clone();
        let standard_report = tier0_deterministic_compact_with(
            &mut standard_messages,
            0,
            CompactAggression::Standard,
        );
        let aggressive_report =
            tier0_deterministic_compact_with(&mut messages, 0, CompactAggression::Aggressive);

        assert_eq!(standard_report.tool_outputs_truncated, 0);
        assert_eq!(aggressive_report.tool_outputs_truncated, 1);
        assert!(messages[1]
            .content
            .content_text_only()
            .contains("truncated"));
    }

    #[test]
    fn test_tier0_aggressive_preserves_only_last_two_tool_outputs() {
        let long_output = "x".repeat(500);
        let mut messages = vec![
            make_tool_msg("tc1", &long_output, None),
            make_tool_msg("tc2", &long_output, None),
            make_tool_msg("tc3", &long_output, None),
            make_tool_msg("tc4", &long_output, None),
        ];

        let report =
            tier0_deterministic_compact_with(&mut messages, 4, CompactAggression::Aggressive);

        assert_eq!(report.tool_outputs_truncated, 2);
        assert!(messages[0]
            .content
            .content_text_only()
            .contains("truncated"));
        assert!(messages[1]
            .content
            .content_text_only()
            .contains("truncated"));
        assert_eq!(messages[2].content.content_text_only(), long_output);
        assert_eq!(messages[3].content.content_text_only(), long_output);
    }

    #[test]
    fn test_tier0_aggressive_elides_long_context_files() {
        let long_context = (0..100)
            .map(|idx| format!("line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut messages = vec![make_context_file_msg("src/main.rs", &long_context)];

        let report =
            tier0_deterministic_compact_with(&mut messages, 0, CompactAggression::Aggressive);

        assert_eq!(report.context_files_deduped, 0);
        assert_eq!(report.context_files_elided, 1);
        let files = extract_context_files_from_content(&messages[0].content);
        assert!(files[0]
            .file_content
            .contains("lines elided under aggressive compaction"));
        assert!(files[0].file_content.contains("line 0"));
        assert!(files[0].file_content.contains("line 99"));
    }

    #[test]
    fn test_tier0_serialization_of_summarization_message() {
        use refact_core::chat_types::ChatMessage;
        let msg = ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText("Summary text".to_string()),
            summarized_range: Some((1, 5)),
            summarization_tier: Some("tier0_deterministic".to_string()),
            summarized_token_estimate: Some(1200),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, "summarization");
        assert_eq!(deserialized.summarized_range, Some((1, 5)));
        assert_eq!(
            deserialized.summarization_tier.as_deref(),
            Some("tier0_deterministic")
        );
        assert_eq!(deserialized.summarized_token_estimate, Some(1200));
        assert_eq!(deserialized.content.content_text_only(), "Summary text");
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
    fn test_context_file_with_matching_id_does_not_satisfy_tool_call() {
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
}
