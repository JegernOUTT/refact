use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, ContextFile};
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use refact_chat_history::history_limit::{compress_duplicate_context_files, compute_context_budget};
use refact_chat_history::trajectory_ops::{
    build_compression_report_message, insert_compression_report_at_boundary, is_memory_path,
    TOOLS_TO_PRESERVE,
};
use refact_core::string_utils::redact_sensitive;
use refact_runtime_api::{ChatSessionUpdate, SessionState};

const TOOL_OUTPUT_TRUNCATE_LIMIT: usize = 200;
const MAX_PER_MESSAGE_ENTRIES: usize = 200;
const MAX_CONTEXT_ENTRIES: usize = 200;
const MAX_TOOL_OUTPUT_ENTRIES: usize = 200;
const AGGRESSIVE_SUMMARY_SKIPPED_REASON: &str = "llm_segment_summarization_required";

fn find_preserve_cutoff(messages: &[crate::call_validation::ChatMessage], turns: usize) -> usize {
    if turns == 0 {
        return messages.len();
    }

    let mut turn_count = 0usize;
    let mut idx = messages.len();
    while idx > 0 {
        idx -= 1;
        if messages[idx].role == "user" {
            turn_count += 1;
            if turn_count >= turns {
                return idx;
            }
        }
    }
    0
}

fn should_preserve_tool(name: &str) -> bool {
    TOOLS_TO_PRESERVE.iter().any(|t| *t == name)
}

fn preserve_cutoff_for(messages: &[ChatMessage], preserve_last_turns: Option<usize>) -> usize {
    preserve_last_turns
        .map(|turns| find_preserve_cutoff(messages, turns))
        .unwrap_or(messages.len())
}

fn aggressive_summary_required_reason(
    messages: &[ChatMessage],
    preserve_cutoff: usize,
) -> Option<&'static str> {
    let cutoff = preserve_cutoff.min(messages.len());
    if crate::chat::summarization::closed_non_user_segments(&messages[..cutoff]).is_empty() {
        None
    } else {
        Some(AGGRESSIVE_SUMMARY_SKIPPED_REASON)
    }
}

#[derive(Default)]
struct CompressChatApplyStats {
    context_files_dropped: usize,
    context_messages_dropped: usize,
    memory_dropped: usize,
    tool_truncated: usize,
    tool_dropped: usize,
    project_info_dropped: usize,
    dedup_count: usize,
    aggressive_summary_skipped_reason: Option<&'static str>,
}

impl CompressChatApplyStats {
    fn context_files_removed(&self) -> usize {
        self.context_files_dropped
            + self.memory_dropped
            + self.project_info_dropped
            + self.dedup_count
    }

    fn has_meaningful_mutation(&self) -> bool {
        self.context_files_dropped > 0
            || self.context_messages_dropped > 0
            || self.memory_dropped > 0
            || self.tool_truncated > 0
            || self.tool_dropped > 0
            || self.project_info_dropped > 0
            || self.dedup_count > 0
    }
}

struct CompressChatApplyRequest<'a> {
    drop_context_files: &'a HashSet<String>,
    drop_memories: &'a HashSet<String>,
    drop_all_memories: bool,
    truncate_tool_outputs: &'a HashSet<String>,
    drop_tool_outputs: &'a HashSet<String>,
    drop_context_messages: &'a HashSet<String>,
    dedup_context_files: bool,
    drop_project_information: bool,
    strength: &'a str,
    preserve_last_turns: Option<usize>,
    target_tokens: Option<usize>,
    tool_call_names: &'a HashMap<String, String>,
}

struct CompressChatApplyOutput {
    messages: Vec<ChatMessage>,
    stats: CompressChatApplyStats,
    before_tokens: usize,
    report: Option<ChatMessage>,
    affected_boundary: Option<usize>,
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

fn insert_current_compression_report(
    messages: &mut Vec<ChatMessage>,
    report: Option<ChatMessage>,
    affected_boundary: Option<usize>,
) -> Option<Value> {
    let report = report?;
    let boundary = affected_boundary.unwrap_or(messages.len());
    let report_idx = insert_compression_report_at_boundary(messages, report, boundary);
    messages[report_idx]
        .extra
        .get("compression_report")
        .cloned()
}

fn tokens_with_tail(
    modifiable_prefix: &[ChatMessage],
    preserved_tail: &[ChatMessage],
    immutable_tail: &[ChatMessage],
) -> usize {
    modifiable_prefix
        .iter()
        .chain(preserved_tail.iter())
        .chain(immutable_tail.iter())
        .map(approx_tokens_for_message)
        .sum()
}

fn remove_invalid_tool_calls_and_tool_calls_results_before(
    modifiable_prefix: &mut Vec<ChatMessage>,
    immutable_tail: &[ChatMessage],
) -> bool {
    let before = serde_json::to_value(&*modifiable_prefix).ok();
    let tool_call_ids: HashSet<String> = modifiable_prefix
        .iter()
        .chain(immutable_tail.iter())
        .filter(|m| (m.role == "tool" || m.role == "diff") && !m.tool_call_id.is_empty())
        .map(|m| m.tool_call_id.clone())
        .collect();

    modifiable_prefix.retain(|m| {
        if let Some(tool_calls) = &m.tool_calls {
            tool_calls.iter().all(|tc| tool_call_ids.contains(&tc.id))
        } else {
            true
        }
    });

    let assistant_tool_call_ids: HashSet<String> = modifiable_prefix
        .iter()
        .chain(immutable_tail.iter())
        .filter_map(|x| x.tool_calls.clone())
        .flatten()
        .map(|x| x.id)
        .collect();

    modifiable_prefix.retain(|m| {
        let is_tool_result = m.role == "tool" || m.role == "diff";
        !(is_tool_result
            && !m.tool_call_id.is_empty()
            && !assistant_tool_call_ids.contains(&m.tool_call_id))
    });

    let mut last_occurrence: HashMap<String, usize> = HashMap::new();
    for (i, m) in modifiable_prefix
        .iter()
        .chain(immutable_tail.iter())
        .enumerate()
    {
        let is_tool_result = m.role == "tool" || m.role == "diff";
        if is_tool_result && !m.tool_call_id.is_empty() {
            last_occurrence.insert(m.tool_call_id.clone(), i);
        }
    }
    let indices_to_keep: HashSet<usize> = last_occurrence.values().cloned().collect();
    let mut current_idx = 0usize;
    modifiable_prefix.retain(|m| {
        let idx = current_idx;
        current_idx += 1;
        let is_tool_result = m.role == "tool" || m.role == "diff";
        m.tool_call_id.is_empty() || !is_tool_result || indices_to_keep.contains(&idx)
    });
    before != serde_json::to_value(&*modifiable_prefix).ok()
}

fn compress_chat_apply_head_output(
    mut head_messages: Vec<ChatMessage>,
    immutable_tail: &[ChatMessage],
    request: &CompressChatApplyRequest,
) -> CompressChatApplyOutput {
    let original_head_messages = head_messages.clone();
    let before_tokens = tokens_with_tail(&head_messages, &[], immutable_tail);
    let preserve_cutoff = preserve_cutoff_for(&head_messages, request.preserve_last_turns);
    let mut preserved_tail = head_messages.split_off(preserve_cutoff.min(head_messages.len()));
    let mut stats = CompressChatApplyStats::default();

    if request.drop_project_information {
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
                    stats.project_info_dropped += 1;
                    false
                } else {
                    true
                }
            };
            idx += 1;
            keep
        });
    }

    let mut updated_head: Vec<ChatMessage> = Vec::with_capacity(head_messages.len());
    for msg in head_messages.into_iter() {
        if msg.role != "context_file" {
            updated_head.push(msg);
            continue;
        }
        if !msg.tool_call_id.is_empty() && request.drop_context_messages.contains(&msg.tool_call_id)
        {
            stats.context_messages_dropped += 1;
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
            if request.drop_context_files.contains(&cf.file_name) {
                stats.context_files_dropped += 1;
                continue;
            }
            if request.drop_all_memories && is_memory {
                stats.memory_dropped += 1;
                continue;
            }
            if request.drop_memories.contains(&cf.file_name) {
                stats.memory_dropped += 1;
                continue;
            }
            remaining.push(cf);
        }

        if remaining.is_empty() {
            stats.context_messages_dropped += 1;
            continue;
        }

        let mut new_msg = msg.clone();
        new_msg.content = ChatContent::ContextFiles(remaining);
        updated_head.push(new_msg);
    }

    head_messages = updated_head;

    if request.dedup_context_files {
        if let Ok((count, _)) = compress_duplicate_context_files(&mut head_messages) {
            stats.dedup_count = count;
        }
    }

    for msg in head_messages.iter_mut() {
        if msg.role != "tool" && msg.role != "diff" {
            continue;
        }
        if msg.tool_call_id.is_empty() {
            continue;
        }
        if request.drop_tool_outputs.contains(&msg.tool_call_id) {
            msg.content =
                ChatContent::SimpleText("Tool result removed by compress_chat_apply".to_string());
            stats.tool_dropped += 1;
            continue;
        }
        if request.truncate_tool_outputs.contains(&msg.tool_call_id) {
            if let Some(name) = request.tool_call_names.get(&msg.tool_call_id) {
                if should_preserve_tool(name) {
                    continue;
                }
            }
            let content = msg.content.content_text_only();
            if content.len() > TOOL_OUTPUT_TRUNCATE_LIMIT {
                let redacted = redact_sensitive(&content);
                let preview: String = redacted.chars().take(TOOL_OUTPUT_TRUNCATE_LIMIT).collect();
                msg.content =
                    ChatContent::SimpleText(format!("Tool result compressed: {}...", preview));
                stats.tool_truncated += 1;
            }
        }
    }

    let mut cleanup_tail = Vec::with_capacity(preserved_tail.len() + immutable_tail.len());
    cleanup_tail.extend_from_slice(&preserved_tail);
    cleanup_tail.extend_from_slice(immutable_tail);
    if remove_invalid_tool_calls_and_tool_calls_results_before(&mut head_messages, &cleanup_tail) {
        stats.tool_dropped += 1;
    }

    if (request.strength == "balanced" || request.strength == "aggressive")
        && !request.dedup_context_files
    {
        let cur_tokens = tokens_with_tail(&head_messages, &preserved_tail, immutable_tail);
        let needs_more = request.target_tokens.map_or(true, |t| cur_tokens > t);
        if needs_more {
            if let Ok((count, _)) = compress_duplicate_context_files(&mut head_messages) {
                stats.dedup_count += count;
            }
        }
    }

    if request.strength == "aggressive" {
        let cur_tokens = tokens_with_tail(&head_messages, &preserved_tail, immutable_tail);
        let needs_more = request.target_tokens.map_or(true, |t| cur_tokens > t);
        if needs_more {
            stats.aggressive_summary_skipped_reason =
                aggressive_summary_required_reason(&head_messages, head_messages.len());
        }
    }

    let report = if stats.has_meaningful_mutation() {
        let after_tokens_pre_report =
            tokens_with_tail(&head_messages, &preserved_tail, immutable_tail);
        Some(build_compression_report_message(
            stats.context_files_removed(),
            stats.context_messages_dropped,
            stats.tool_truncated + stats.tool_dropped,
            before_tokens,
            after_tokens_pre_report,
        ))
    } else {
        None
    };
    head_messages.append(&mut preserved_tail);
    let affected_boundary = report
        .as_ref()
        .and_then(|_| first_changed_boundary(&original_head_messages, &head_messages));
    CompressChatApplyOutput {
        messages: head_messages,
        stats,
        before_tokens,
        report,
        affected_boundary,
    }
}

#[cfg(test)]
fn compress_chat_apply_head_messages(
    head_messages: Vec<ChatMessage>,
    immutable_tail: &[ChatMessage],
    request: &CompressChatApplyRequest,
) -> (Vec<ChatMessage>, CompressChatApplyStats, usize) {
    let CompressChatApplyOutput {
        mut messages,
        stats,
        before_tokens,
        report,
        affected_boundary,
    } = compress_chat_apply_head_output(head_messages, immutable_tail, request);
    insert_current_compression_report(&mut messages, report, affected_boundary);
    (messages, stats, before_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};

    fn user_message(text: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn event_message(text: &str) -> ChatMessage {
        crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "test.compress_chat",
            json!({}),
            text.to_string(),
        )
    }

    fn assistant_message(text: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn system_message(text: &str) -> ChatMessage {
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn context_file_message(
        tool_call_id: &str,
        file_name: &str,
        file_content: &str,
    ) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: file_name.to_string(),
                file_content: file_content.to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }]),
            ..Default::default()
        }
    }

    fn assistant_tool_call_message(tool_call_id: &str, tool_name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
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

    fn tool_message(tool_call_id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn apply_request<'a>(
        drop_context_files: &'a HashSet<String>,
        drop_memories: &'a HashSet<String>,
        truncate_tool_outputs: &'a HashSet<String>,
        drop_tool_outputs: &'a HashSet<String>,
        drop_context_messages: &'a HashSet<String>,
        tool_call_names: &'a HashMap<String, String>,
    ) -> CompressChatApplyRequest<'a> {
        CompressChatApplyRequest {
            drop_context_files,
            drop_memories,
            drop_all_memories: false,
            truncate_tool_outputs,
            drop_tool_outputs,
            drop_context_messages,
            dedup_context_files: false,
            drop_project_information: false,
            strength: "conservative",
            preserve_last_turns: Some(1),
            target_tokens: None,
            tool_call_names,
        }
    }

    fn assert_preserved_tail_unchanged(
        before: &[ChatMessage],
        after: &[ChatMessage],
        turns: usize,
    ) {
        let cutoff = find_preserve_cutoff(before, turns);
        let expected = serde_json::to_string(&before[cutoff..]).unwrap();
        let actual_start = after.len() - (before.len() - cutoff);
        let actual = serde_json::to_string(&after[actual_start..]).unwrap();
        assert_eq!(actual, expected);
    }

    fn compression_report_index(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .position(|message| message.role == "compression_report")
            .expect("expected compression_report message")
    }

    fn compression_report_count(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .filter(|message| message.role == "compression_report")
            .count()
    }

    fn compression_report_metadata(message: &ChatMessage) -> Value {
        message.extra["compression_report"].clone()
    }

    fn compression_report_metadatas(messages: &[ChatMessage]) -> Vec<Value> {
        messages
            .iter()
            .filter(|message| message.role == "compression_report")
            .map(compression_report_metadata)
            .collect()
    }

    fn stable_existing_report_for_manual_context_drop() -> ChatMessage {
        let mut report = build_compression_report_message(1, 1, 0, 0, 0);
        for _ in 0..10 {
            let before = vec![
                user_message("old user"),
                report.clone(),
                context_file_message("old_context", "old.rs", "old context"),
                assistant_message("old assistant"),
            ];
            let after = vec![
                user_message("old user"),
                report.clone(),
                assistant_message("old assistant"),
            ];
            let next = build_compression_report_message(
                1,
                1,
                0,
                before.iter().map(approx_tokens_for_message).sum(),
                after.iter().map(approx_tokens_for_message).sum(),
            );
            if compression_report_metadata(&next) == compression_report_metadata(&report) {
                return report;
            }
            report = next;
        }
        report
    }

    #[test]
    fn preserve_cutoff_zero_turns_makes_all_messages_modifiable() {
        let messages = vec![user_message("one"), assistant_message("two")];

        assert_eq!(find_preserve_cutoff(&messages, 0), messages.len());
    }

    #[test]
    fn preserve_cutoff_no_user_messages_makes_all_messages_preserved() {
        let messages = vec![assistant_message("one"), assistant_message("two")];

        assert_eq!(find_preserve_cutoff(&messages, 2), 0);
    }

    #[test]
    fn preserve_cutoff_fewer_user_turns_than_requested_preserves_all() {
        let messages = vec![user_message("one"), assistant_message("two")];

        assert_eq!(find_preserve_cutoff(&messages, 2), 0);
    }

    #[test]
    fn preserve_cutoff_preserves_requested_tail_turns() {
        let messages = vec![
            user_message("one"),
            assistant_message("two"),
            user_message("three"),
            assistant_message("four"),
        ];

        assert_eq!(find_preserve_cutoff(&messages, 1), 2);
    }

    #[test]
    fn preserve_cutoff_treats_event_notices_as_non_user_turns() {
        let messages = vec![
            event_message("synthetic notice"),
            assistant_message("two"),
            user_message("real user"),
            assistant_message("four"),
        ];

        assert_eq!(find_preserve_cutoff(&messages, 1), 2);
        assert_eq!(find_preserve_cutoff(&messages, 2), 0);
    }

    #[test]
    fn aggressive_summary_required_detects_only_modifiable_segments() {
        let messages = vec![
            user_message("old user"),
            assistant_message("old assistant"),
            user_message("tail user"),
            assistant_message("tail assistant"),
            user_message("active tool user"),
        ];
        let preserve_cutoff = find_preserve_cutoff(&messages, 1);
        let preserved_tail_count = messages.len() - preserve_cutoff;
        let preserved_tail_before = serde_json::to_string(&messages[preserve_cutoff..]).unwrap();

        assert_eq!(
            aggressive_summary_required_reason(&messages, preserve_cutoff),
            Some(AGGRESSIVE_SUMMARY_SKIPPED_REASON),
        );

        let preserved_tail_start = messages.len() - preserved_tail_count;
        let preserved_tail_after =
            serde_json::to_string(&messages[preserved_tail_start..]).unwrap();
        assert_eq!(preserved_tail_after, preserved_tail_before);
    }

    #[test]
    fn aggressive_summary_required_ignores_eligible_segment_only_in_preserved_tail() {
        let messages = vec![
            user_message("old user"),
            user_message("tail user"),
            assistant_message("tail assistant"),
            user_message("active tool user"),
        ];
        let preserve_cutoff = find_preserve_cutoff(&messages, 1);
        let before = serde_json::to_string(&messages).unwrap();

        assert_eq!(
            aggressive_summary_required_reason(&messages, preserve_cutoff),
            None
        );

        assert_eq!(serde_json::to_string(&messages).unwrap(), before);
    }

    #[test]
    fn aggressive_summary_skip_does_not_create_static_placeholder() {
        let messages = vec![
            user_message("old user"),
            assistant_message("old assistant"),
            user_message("middle user"),
            user_message("tail user"),
        ];
        let preserve_cutoff = find_preserve_cutoff(&messages, 1);

        assert_eq!(
            aggressive_summary_required_reason(&messages, preserve_cutoff),
            Some(AGGRESSIVE_SUMMARY_SKIPPED_REASON),
        );
        let removed_placeholder = [
            "Previous non-user chat activity",
            "was summarized by",
            "compress_chat_apply",
        ]
        .join(" ");
        assert!(!messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains(&removed_placeholder)));
        assert!(!messages
            .iter()
            .any(crate::chat::summarization::is_segment_summary));
    }
    #[test]
    fn apply_drop_all_memories_preserves_tail_context_files() {
        let messages = vec![
            user_message("old user"),
            context_file_message("old_memory", "/repo/.refact/knowledge/old.md", "old memory"),
            user_message("tail user"),
            context_file_message(
                "tail_memory",
                "/repo/.refact/knowledge/tail.md",
                "tail memory",
            ),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.drop_all_memories = true;

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert_eq!(stats.memory_dropped, 1);
        assert_preserved_tail_unchanged(&messages, &after, 1);
        let report_idx = compression_report_index(&after);
        assert!(after[report_idx + 1..]
            .iter()
            .any(|message| message.content.content_text_only().contains("tail memory")));
        assert!(after
            .iter()
            .any(|message| message.content.content_text_only().contains("tail memory")));
    }

    #[test]
    fn apply_context_drop_options_preserve_tail_context_file_messages() {
        let messages = vec![
            user_message("old user"),
            context_file_message("old_context", "old.rs", "old context"),
            user_message("tail user"),
            context_file_message("tail_context", "tail.rs", "tail context"),
        ];
        let drop_context_files = HashSet::from(["tail.rs".to_string(), "old.rs".to_string()]);
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages =
            HashSet::from(["tail_context".to_string(), "old_context".to_string()]);
        let tool_call_names = HashMap::new();
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert_eq!(stats.context_messages_dropped, 1);
        assert_preserved_tail_unchanged(&messages, &after, 1);
    }

    #[test]
    fn apply_all_preserved_noop_does_not_prepend_report_before_tail() {
        let messages = vec![
            user_message("tail user"),
            assistant_message("tail assistant"),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert!(!stats.has_meaningful_mutation());
        assert_eq!(compression_report_count(&after), 0);
        assert_eq!(
            serde_json::to_string(&after).unwrap(),
            serde_json::to_string(&messages).unwrap()
        );
    }

    #[test]
    fn apply_noop_modifiable_head_does_not_add_report() {
        let messages = vec![
            user_message("old user"),
            assistant_message("old assistant"),
            user_message("tail user"),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert!(!stats.has_meaningful_mutation());
        assert_eq!(compression_report_count(&after), 0);
        assert_eq!(
            serde_json::to_string(&after).unwrap(),
            serde_json::to_string(&messages).unwrap()
        );
    }

    #[test]
    fn apply_tool_drop_and_truncate_preserve_tail_tool_results() {
        let long_old = "old tool output ".repeat(30);
        let long_tail = "tail tool output ".repeat(30);
        let messages = vec![
            user_message("old user"),
            assistant_tool_call_message("old_call", "shell"),
            tool_message("old_call", &long_old),
            user_message("tail user"),
            assistant_tool_call_message("tail_drop_call", "shell"),
            tool_message("tail_drop_call", "tail drop output"),
            assistant_tool_call_message("tail_truncate_call", "shell"),
            tool_message("tail_truncate_call", &long_tail),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs =
            HashSet::from(["old_call".to_string(), "tail_truncate_call".to_string()]);
        let drop_tool_outputs = HashSet::from(["tail_drop_call".to_string()]);
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::from([
            ("old_call".to_string(), "shell".to_string()),
            ("tail_drop_call".to_string(), "shell".to_string()),
            ("tail_truncate_call".to_string(), "shell".to_string()),
        ]);
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert_eq!(stats.tool_truncated, 1);
        assert_eq!(stats.tool_dropped, 0);
        assert!(after.iter().any(|message| message
            .content
            .content_text_only()
            .starts_with("Tool result compressed:")));
        assert!(compression_report_index(&after) <= find_preserve_cutoff(&messages, 1));
        assert_preserved_tail_unchanged(&messages, &after, 1);
    }

    #[test]
    fn apply_tool_truncate_preview_redacts_bearer_token() {
        let secret = "secret-token-123";
        let messages = vec![
            user_message("old user"),
            assistant_tool_call_message("old_call", "shell"),
            tool_message(
                "old_call",
                &format!("request failed with Bearer {} {}", secret, "x".repeat(1000)),
            ),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::from(["old_call".to_string()]);
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::from([("old_call".to_string(), "shell".to_string())]);
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.tool_truncated, 1);
        let tool_text = after
            .iter()
            .find(|message| message.role == "tool")
            .unwrap()
            .content
            .content_text_only();
        assert!(tool_text.contains("Bearer [REDACTED]"));
        assert!(!tool_text.contains(secret));
    }

    #[test]
    fn apply_tool_truncate_preview_redacts_api_key() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz";
        let messages = vec![
            user_message("old user"),
            assistant_tool_call_message("old_call", "shell"),
            tool_message(
                "old_call",
                &format!(
                    "request failed with api_key={} {}",
                    secret,
                    "x".repeat(1000)
                ),
            ),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::from(["old_call".to_string()]);
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::from([("old_call".to_string(), "shell".to_string())]);
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.tool_truncated, 1);
        let tool_text = after
            .iter()
            .find(|message| message.role == "tool")
            .unwrap()
            .content
            .content_text_only();
        assert!(tool_text.contains("api_key=[REDACTED]"));
        assert!(!tool_text.contains(secret));
    }

    #[test]
    fn apply_drop_all_memories_detects_relative_memory_path() {
        let messages = vec![
            context_file_message("memory", ".refact/knowledge/foo.md", "memory"),
            context_file_message("source", "src/lib.rs", "source"),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.drop_all_memories = true;
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.memory_dropped, 1);
        assert!(!after
            .iter()
            .any(|message| message.content.content_text_only().contains("foo.md")));
        assert!(after
            .iter()
            .any(|message| message.content.content_text_only().contains("source")));
    }

    #[test]
    fn apply_drop_all_memories_detects_windows_task_path() {
        let messages = vec![
            context_file_message(
                "memory",
                r#"C:\repo\.refact\tasks\task-id\memories\note.md"#,
                "memory",
            ),
            context_file_message("source", "src/lib.rs", "source"),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.drop_all_memories = true;
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.memory_dropped, 1);
        assert!(!after
            .iter()
            .any(|message| message.content.content_text_only().contains("note.md")));
        assert!(after
            .iter()
            .any(|message| message.content.content_text_only().contains("source")));
    }

    #[test]
    fn apply_drop_all_memories_preserves_non_memory_paths() {
        let messages = vec![context_file_message(
            "source",
            "tests/.refact_fixture/tasks/example.rs",
            "fixture source",
        )];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.drop_all_memories = true;
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.memory_dropped, 0);
        assert!(after.iter().any(|message| message
            .content
            .content_text_only()
            .contains("fixture source")));
        assert_eq!(compression_report_count(&after), 0);
    }

    #[test]
    fn apply_inserts_compression_report_before_preserved_tail() {
        let messages = vec![
            user_message("old user"),
            context_file_message("old_context", "old.rs", "old context"),
            user_message("tail user"),
            assistant_message("tail assistant"),
        ];
        let drop_context_files = HashSet::from(["old.rs".to_string()]);
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let (after, stats, before_tokens) =
            compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert_eq!(stats.context_messages_dropped, 1);
        let report_idx = compression_report_index(&after);
        assert_eq!(
            after[report_idx + 1].content.content_text_only(),
            "tail user"
        );
        let metadata = &after[report_idx].extra["compression_report"];
        assert_eq!(metadata["kind"], json!("chat_compression_report"));
        assert_eq!(metadata["context_files_removed"], json!(1));
        assert_eq!(metadata["context_messages_dropped"], json!(1));
        assert_eq!(metadata["tool_results_truncated"], json!(0));
        assert!(after[report_idx]
            .content
            .content_text_only()
            .contains("- Context messages dropped: 1"));
        assert_eq!(metadata["tokens_before"], json!(before_tokens));
        assert!(metadata["tokens_after"].as_u64().unwrap() <= before_tokens as u64);
    }

    #[test]
    fn apply_report_after_first_user_when_leading_systems_and_first_user_preserved() {
        let messages = vec![
            system_message("root prompt"),
            system_message("workspace details"),
            context_file_message("old_context", "old.rs", "old context"),
            user_message("tail user"),
            assistant_message("tail assistant"),
        ];
        let drop_context_files = HashSet::from(["old.rs".to_string()]);
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.context_messages_dropped, 1);
        let report_idx = compression_report_index(&after);
        let first_user_idx = after
            .iter()
            .position(|message| message.role == "user")
            .unwrap();
        assert!(report_idx > first_user_idx);
        let roles: Vec<_> = after.iter().map(|message| message.role.as_str()).collect();
        assert_eq!(
            roles,
            vec![
                "system",
                "system",
                "user",
                "compression_report",
                "assistant"
            ]
        );
    }

    #[test]
    fn apply_noop_with_existing_report_returns_no_current_report_metadata() {
        let old_report = build_compression_report_message(1, 1, 0, 100, 80);
        let old_metadata = compression_report_metadata(&old_report);
        let messages = vec![
            user_message("old user"),
            old_report,
            assistant_message("answer"),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.preserve_last_turns = Some(0);

        let CompressChatApplyOutput {
            mut messages,
            stats,
            report,
            affected_boundary,
            ..
        } = compress_chat_apply_head_output(messages, &[], &request);
        let current_metadata =
            insert_current_compression_report(&mut messages, report, affected_boundary);

        assert!(!stats.has_meaningful_mutation());
        assert_eq!(current_metadata, None);
        assert_eq!(compression_report_count(&messages), 1);
        assert_eq!(compression_report_metadatas(&messages), vec![old_metadata]);
    }

    #[test]
    fn apply_preserved_tail_old_report_does_not_drive_current_metadata() {
        let old_report = build_compression_report_message(0, 0, 1, 1000, 900);
        let old_metadata = compression_report_metadata(&old_report);
        let messages = vec![
            user_message("old user"),
            context_file_message("old_context", "old.rs", "old context"),
            user_message("tail user"),
            old_report,
            assistant_message("tail assistant"),
        ];
        let drop_context_files = HashSet::from(["old.rs".to_string()]);
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );

        let CompressChatApplyOutput {
            mut messages,
            stats,
            report,
            affected_boundary,
            ..
        } = compress_chat_apply_head_output(messages, &[], &request);
        let current_metadata =
            insert_current_compression_report(&mut messages, report, affected_boundary)
                .expect("current invocation should create report metadata");

        assert_eq!(stats.context_messages_dropped, 1);
        assert_ne!(current_metadata, old_metadata);
        assert_eq!(compression_report_count(&messages), 2);
        assert!(compression_report_metadatas(&messages).contains(&old_metadata));
    }

    #[test]
    fn apply_repeated_equivalent_manual_compression_dedupes() {
        let existing_report = stable_existing_report_for_manual_context_drop();
        let existing_metadata = compression_report_metadata(&existing_report);
        let messages = vec![
            user_message("old user"),
            existing_report,
            context_file_message("old_context", "old.rs", "old context"),
            assistant_message("old assistant"),
        ];
        let drop_context_files = HashSet::from(["old.rs".to_string()]);
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.context_messages_dropped, 1);
        assert_eq!(compression_report_count(&after), 1);
        assert_eq!(
            compression_report_metadatas(&after),
            vec![existing_metadata]
        );
    }

    #[test]
    fn apply_non_equivalent_manual_reports_remain() {
        let existing_report = build_compression_report_message(0, 0, 1, 1000, 800);
        let existing_metadata = compression_report_metadata(&existing_report);
        let messages = vec![
            user_message("old user"),
            existing_report,
            context_file_message("old_context", "old.rs", "old context"),
            assistant_message("old assistant"),
        ];
        let drop_context_files = HashSet::from(["old.rs".to_string()]);
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.preserve_last_turns = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages, &[], &request);

        assert_eq!(stats.context_messages_dropped, 1);
        assert_eq!(compression_report_count(&after), 2);
        assert!(compression_report_metadatas(&after).contains(&existing_metadata));
    }

    #[test]
    fn apply_drop_project_information_preserves_tail_system_messages() {
        let messages = vec![
            system_message("root prompt"),
            system_message("old project workspace details"),
            user_message("tail user"),
            system_message("tail project workspace details"),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::new();
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::new();
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.drop_project_information = true;

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert_eq!(stats.project_info_dropped, 1);
        assert!(!after.iter().any(|message| message
            .content
            .content_text_only()
            .contains("old project workspace details")));
        let report_idx = compression_report_index(&after);
        assert_eq!(
            after[report_idx - 1].content.content_text_only(),
            "tail user"
        );
        assert!(after.iter().any(|message| message
            .content
            .content_text_only()
            .contains("tail project workspace details")));
    }

    #[test]
    fn apply_aggressive_combination_preserves_tail_byte_identically() {
        let messages = vec![
            system_message("root prompt"),
            user_message("old user"),
            assistant_message("old assistant"),
            context_file_message("old_memory", "/repo/.refact/knowledge/old.md", "old memory"),
            user_message("middle user"),
            assistant_message("middle assistant"),
            user_message("tail user"),
            context_file_message(
                "tail_memory",
                "/repo/.refact/knowledge/tail.md",
                "tail memory",
            ),
            assistant_tool_call_message("tail_call", "shell"),
            tool_message("tail_call", &"tail tool output ".repeat(30)),
        ];
        let drop_context_files = HashSet::new();
        let drop_memories = HashSet::new();
        let truncate_tool_outputs = HashSet::from(["tail_call".to_string()]);
        let drop_tool_outputs = HashSet::new();
        let drop_context_messages = HashSet::new();
        let tool_call_names = HashMap::from([("tail_call".to_string(), "shell".to_string())]);
        let mut request = apply_request(
            &drop_context_files,
            &drop_memories,
            &truncate_tool_outputs,
            &drop_tool_outputs,
            &drop_context_messages,
            &tool_call_names,
        );
        request.drop_all_memories = true;
        request.drop_project_information = true;
        request.strength = "aggressive";
        request.target_tokens = Some(0);

        let (after, stats, _) = compress_chat_apply_head_messages(messages.clone(), &[], &request);

        assert_eq!(stats.memory_dropped, 1);
        assert_eq!(stats.tool_truncated, 0);
        assert_eq!(
            stats.aggressive_summary_skipped_reason,
            Some(AGGRESSIVE_SUMMARY_SKIPPED_REASON)
        );
        assert_preserved_tail_unchanged(&messages, &after, 1);
    }
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
        let (chat_facade, chat_id, n_ctx) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.chat.facade.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.n_ctx,
            )
        };

        let messages = chat_facade.session_snapshot(&chat_id).await?.messages;

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

        let budget = compute_context_budget(&messages, n_ctx);
        let pressure_label = match budget.pressure {
            refact_chat_history::history_limit::ContextPressure::Low => "low",
            refact_chat_history::history_limit::ContextPressure::Medium => "medium",
            refact_chat_history::history_limit::ContextPressure::High => "high",
            refact_chat_history::history_limit::ContextPressure::Critical => "critical",
        };
        let pct_used = if n_ctx > 0 {
            total_tokens.saturating_mul(100) / n_ctx
        } else {
            0
        };

        let result = json!({
            "type": "ctx_probe",
            "messages_count": messages.len(),
            "total_tokens": total_tokens,
            "n_ctx": n_ctx,
            "pct_used": pct_used,
            "context_pressure": pressure_label,
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
                },
                "target_tokens": {
                    "type": "integer",
                    "description": "Target total token count after compression"
                },
                "strength": {
                    "type": "string",
                    "enum": ["conservative", "balanced", "aggressive"],
                    "description": "conservative=explicit ops only, balanced=+auto dedup, aggressive=+dedup and reports when LLM segment summarization is required"
                },
                "preserve_last_turns": {
                    "type": "integer",
                    "description": "Number of recent user/assistant turns to keep unmodified"
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Preview compression stats without applying changes"
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
        let dry_run = parse_bool(args, "dry_run");
        let strength = args
            .get("strength")
            .and_then(|v| v.as_str())
            .unwrap_or("conservative")
            .to_string();
        let preserve_last_turns = args
            .get("preserve_last_turns")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let target_tokens = args
            .get("target_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let (chat_facade, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.app.chat.facade.clone(), ccx_lock.chat_id.clone())
        };

        let session_snapshot = chat_facade.session_snapshot(&chat_id).await?;
        if matches!(session_snapshot.session_state, SessionState::Generating) {
            return Err("Cannot compress while generating".to_string());
        }

        let before_count = session_snapshot.messages.len();
        let active_start = session_snapshot
            .messages
            .iter()
            .rposition(|m| {
                m.role == "assistant"
                    && m.tool_calls
                        .as_ref()
                        .map(|tcs| tcs.iter().any(|tc| tc.id == *tool_call_id))
                        .unwrap_or(false)
            })
            .unwrap_or(session_snapshot.messages.len());

        if active_start >= session_snapshot.messages.len() {
            return Err("Active tool call not found in session".to_string());
        }

        let tool_call_names: HashMap<String, String> = session_snapshot
            .messages
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .flatten()
            .map(|tc| (tc.id.clone(), tc.function.name.clone()))
            .collect();
        let tail_messages = session_snapshot.messages[active_start..].to_vec();

        let drop_context_files: HashSet<String> = drop_context_files.into_iter().collect();
        let drop_memories: HashSet<String> = drop_memories.into_iter().collect();
        let drop_context_messages: HashSet<String> = drop_context_messages.into_iter().collect();
        let truncate_tool_outputs: HashSet<String> = truncate_tool_outputs.into_iter().collect();
        let drop_tool_outputs: HashSet<String> = drop_tool_outputs.into_iter().collect();

        let request = CompressChatApplyRequest {
            drop_context_files: &drop_context_files,
            drop_memories: &drop_memories,
            drop_all_memories,
            truncate_tool_outputs: &truncate_tool_outputs,
            drop_tool_outputs: &drop_tool_outputs,
            drop_context_messages: &drop_context_messages,
            dedup_context_files,
            drop_project_information,
            strength: &strength,
            preserve_last_turns,
            target_tokens,
            tool_call_names: &tool_call_names,
        };
        let CompressChatApplyOutput {
            messages: mut head_messages,
            stats,
            before_tokens,
            report,
            affected_boundary,
        } = compress_chat_apply_head_output(
            session_snapshot.messages[..active_start].to_vec(),
            &tail_messages,
            &request,
        );
        head_messages.extend(tail_messages);
        let compression_report_metadata =
            insert_current_compression_report(&mut head_messages, report, affected_boundary);

        let after_tokens = head_messages
            .iter()
            .map(approx_tokens_for_message)
            .sum::<usize>();
        let after_count = head_messages.len();
        let report_after_tokens = compression_report_metadata
            .as_ref()
            .and_then(|metadata| metadata.get("tokens_after"))
            .and_then(|value| value.as_u64())
            .map(|value| value as usize)
            .unwrap_or(after_tokens);
        let target_met = target_tokens.map_or(true, |t| report_after_tokens <= t);

        let first_role = head_messages.first().map(|m| m.role.as_str()).unwrap_or("");
        if !matches!(first_role, "system" | "user" | "event" | "plan") {
            return Err(format!(
                "ctx_apply would produce an invalid chat history: first message has role '{}', expected 'system', 'user', 'event', or 'plan'. Compression aborted.",
                if first_role.is_empty() { "(empty)" } else { first_role }
            ));
        }

        if !dry_run {
            chat_facade
                .update_session(
                    &chat_id,
                    ChatSessionUpdate {
                        messages: head_messages,
                        previous_response_id: None,
                    },
                )
                .await?;

            chat_facade.maybe_save_session(&chat_id).await?;
        }

        let result = json!({
            "type": "ctx_apply",
            "dry_run": dry_run,
            "before_message_count": before_count,
            "after_message_count": after_count,
            "before_tokens": before_tokens,
            "after_tokens": report_after_tokens,
            "target_tokens": target_tokens,
            "target_met": target_met,
            "strength": strength,
            "context_files_dropped": stats.context_files_dropped,
            "context_messages_dropped": stats.context_messages_dropped,
            "memories_dropped": stats.memory_dropped,
            "tool_outputs_truncated": stats.tool_truncated,
            "tool_outputs_dropped": stats.tool_dropped,
            "project_info_dropped": stats.project_info_dropped,
            "dedup_context_files": stats.dedup_count,
            "compression_report": compression_report_metadata.map(|metadata| json!({
                "role": "compression_report",
                "context_files_removed": metadata.get("context_files_removed").cloned().unwrap_or(Value::Null),
                "context_messages_dropped": metadata.get("context_messages_dropped").cloned().unwrap_or(Value::Null),
                "tool_results_truncated": metadata.get("tool_results_truncated").cloned().unwrap_or(Value::Null),
                "tokens_before": metadata.get("tokens_before").cloned().unwrap_or(Value::Null),
                "tokens_after": metadata.get("tokens_after").cloned().unwrap_or(Value::Null),
            })),
            "aggressive_summary_skipped_reason": stats.aggressive_summary_skipped_reason,
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
