use std::collections::{BTreeMap, HashMap, HashSet};

use std::path::PathBuf;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tracing::warn;

use refact_core::chunk_utils::{official_text_hashing_function, count_text_tokens_with_fallback};
use refact_core::chat_types::{
    ChatContent, ChatMessage, ContextFile, PostprocessSettings, SearchResult, format_search_results,
};
use crate::config::limits;
use crate::line_truncation::{truncate_tool_line, truncate_tool_text_lines};
use crate::pp_context_files::postprocess_context_files;
use crate::pp_context_provider::PPContextTrait;
use crate::pp_plain_text::postprocess_plain_text;

fn canonical_path_simple(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    p.canonicalize().unwrap_or(p)
}

const MIN_CONTEXT_SIZE: usize = 8192;

#[derive(Debug)]
pub struct ToolBudget {
    pub tokens_for_code: usize,
    pub tokens_for_text: usize,
}

impl ToolBudget {
    pub fn try_from_n_ctx(n_ctx: usize) -> Result<Self, String> {
        if n_ctx < MIN_CONTEXT_SIZE {
            return Err(format!(
                "Model context size {} is below minimum {} tokens",
                n_ctx, MIN_CONTEXT_SIZE
            ));
        }
        let pp_limits = limits();
        let total = (n_ctx / 2).max(4096).min(pp_limits.max_tool_budget_tokens);
        Ok(Self {
            tokens_for_code: total,
            tokens_for_text: total * pp_limits.tokens_for_text_percent / 100,
        })
    }
}

pub async fn postprocess_tool_results(
    ctx: Arc<dyn PPContextTrait>,
    tokenizer: Option<Arc<Tokenizer>>,
    tool_messages: Vec<ChatMessage>,
    context_files: Vec<ContextFile>,
    budget: ToolBudget,
    pp_settings: PostprocessSettings,
    existing_messages: &[ChatMessage],
    truncation_exempt_tool_call_ids: &HashSet<String>,
) -> Vec<ChatMessage> {
    let mut result = Vec::new();
    let visible_history = refact_core::active_context::active_context(existing_messages)
        .map(|view| view.messages)
        .unwrap_or_default();
    let existing_messages = visible_history.as_slice();

    let (mut diff_messages, other_messages): (Vec<_>, Vec<_>) =
        tool_messages.into_iter().partition(|m| {
            m.role == "diff"
                || m.extra.get("_ui_only").and_then(serde_json::Value::as_bool) == Some(true)
                || m.extra.contains_key("edit_result")
        });

    let mut edit_indices = Vec::new();
    let mut edit_messages = Vec::new();
    for (index, message) in diff_messages.iter().enumerate() {
        if message
            .extra
            .get("_ui_only")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            continue;
        }
        if let Some(text) = message
            .extra
            .get("edit_result")
            .filter(|value| value.get("version").and_then(serde_json::Value::as_u64) == Some(1))
            .and_then(|value| value.get("model_content"))
            .and_then(serde_json::Value::as_str)
        {
            let mut projection = message.clone();
            projection.role = "tool".to_string();
            projection.content = ChatContent::SimpleText(truncate_tool_text_lines(text));
            projection.output_filter = None;
            edit_indices.push(index);
            edit_messages.push(projection);
        }
    }
    let has_other_output = !other_messages.is_empty() || !context_files.is_empty();
    let edit_budget = if has_other_output {
        budget.tokens_for_code / 2
    } else {
        budget.tokens_for_code
    };
    let (edit_messages, edit_remaining) =
        postprocess_plain_text(edit_messages, tokenizer.clone(), edit_budget, &None).await;
    let edit_used = edit_budget.saturating_sub(edit_remaining);
    for (index, projection) in edit_indices.into_iter().zip(edit_messages) {
        if let Some(metadata) = diff_messages[index].extra.get_mut("edit_result") {
            metadata["model_content"] =
                serde_json::Value::String(projection.content.content_text_only());
        }
    }
    result.extend(diff_messages);

    let mut exempt_indexed: Vec<(usize, ChatMessage)> = Vec::new();
    let mut budgeted_indexed: Vec<(usize, ChatMessage)> = Vec::new();
    for (idx, msg) in other_messages.into_iter().enumerate() {
        if !msg.tool_call_id.is_empty()
            && truncation_exempt_tool_call_ids.contains(&msg.tool_call_id)
        {
            exempt_indexed.push((idx, msg));
        } else {
            budgeted_indexed.push((idx, msg));
        }
    }

    let total_budget = budget.tokens_for_code.saturating_sub(edit_used);
    let text_budget = if context_files.is_empty() {
        total_budget
    } else if budgeted_indexed.is_empty() {
        0
    } else {
        budget.tokens_for_text.min(total_budget)
    };

    let budgeted_indices: Vec<usize> = budgeted_indexed.iter().map(|(idx, _)| *idx).collect();
    let budgeted_messages: Vec<ChatMessage> =
        budgeted_indexed.into_iter().map(|(_, msg)| msg).collect();
    let (text_messages, text_remaining) =
        postprocess_plain_text(budgeted_messages, tokenizer.clone(), text_budget, &None).await;

    let mut ordered: Vec<(usize, ChatMessage)> =
        budgeted_indices.into_iter().zip(text_messages).collect();
    ordered.extend(exempt_indexed);
    ordered.sort_by_key(|(idx, _)| *idx);
    result.extend(ordered.into_iter().map(|(_, msg)| msg));

    deduplicate_web_search_tool_results(&mut result, existing_messages);

    let code_budget = total_budget.saturating_sub(text_budget) + text_remaining;

    let (file_message, notes, _code_used) = if !context_files.is_empty() {
        postprocess_context_file_results(
            ctx,
            tokenizer.clone(),
            context_files,
            code_budget,
            pp_settings,
            existing_messages,
        )
        .await
    } else {
        (None, vec![], 0)
    };

    if !notes.is_empty() {
        if let Some(last_tool_msg) = result.iter_mut().rev().find(|m| m.role == "tool") {
            if let ChatContent::SimpleText(ref mut text) = last_tool_msg.content {
                text.push_str("\n\n");
                text.push_str(&notes.join("\n"));
            }
        }
    }

    if let Some(msg) = file_message {
        result.push(msg);
    }

    result
}

fn normalize_title_key(title: &str) -> String {
    title
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_snippet_key(snippet: &str) -> String {
    normalize_title_key(snippet)
}

fn canonicalize_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn deduplicate_search_results_for_postprocessing(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut deduped: Vec<SearchResult> = Vec::new();
    let mut by_url: HashMap<String, usize> = HashMap::new();
    let mut by_title_snippet: HashMap<(String, String), usize> = HashMap::new();

    for mut result in results {
        result.title = result.title.trim().to_string();
        result.url = canonicalize_url(&result.url);
        result.snippet = result.snippet.trim().to_string();

        if result.title.is_empty() || result.url.is_empty() {
            continue;
        }

        let url_key = result.url.clone();
        if let Some(existing_idx) = by_url.get(&url_key).copied() {
            let existing = &mut deduped[existing_idx];
            if existing.snippet.is_empty() && !result.snippet.is_empty() {
                existing.snippet = result.snippet.clone();
            }
            continue;
        }

        let key = (
            normalize_title_key(&result.title),
            normalize_snippet_key(&result.snippet),
        );
        if let Some(existing_idx) = by_title_snippet.get(&key).copied() {
            let existing = &mut deduped[existing_idx];
            if existing.snippet.is_empty() && !result.snippet.is_empty() {
                existing.snippet = result.snippet.clone();
            }
            continue;
        }

        let idx = deduped.len();
        by_url.insert(url_key, idx);
        by_title_snippet.insert(key, idx);
        deduped.push(result);
    }

    deduped
}

fn extract_search_results_from_extra(msg: &ChatMessage) -> Option<Vec<SearchResult>> {
    let raw = msg.extra.get("search_results")?.as_array()?;
    let mut parsed = Vec::new();

    for item in raw {
        let obj = item.as_object()?;
        let title = obj.get("title")?.as_str()?.to_string();
        let url = obj.get("url")?.as_str()?.to_string();
        let snippet = obj
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let source = obj
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        parsed.push(SearchResult {
            title,
            url,
            snippet,
            source,
        });
    }

    Some(parsed)
}

fn is_web_search_tool_message(msg: &ChatMessage, existing_messages: &[ChatMessage]) -> bool {
    if msg.role != "tool" || msg.tool_call_id.is_empty() {
        return false;
    }

    existing_messages.iter().any(|existing| {
        existing
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .any(|call| call.id == msg.tool_call_id && call.function.name == "web_search")
            })
            .unwrap_or(false)
    })
}

fn deduplicate_web_search_tool_results(
    result: &mut [ChatMessage],
    existing_messages: &[ChatMessage],
) {
    for msg in result.iter_mut() {
        if !is_web_search_tool_message(msg, existing_messages) {
            continue;
        }

        let Some(search_results) = extract_search_results_from_extra(msg) else {
            continue;
        };

        let deduped = deduplicate_search_results_for_postprocessing(search_results);
        if let Some(extra_results) = msg.extra.get_mut("search_results") {
            *extra_results = serde_json::json!(deduped.clone());
        }

        if let ChatContent::SimpleText(text) = &mut msg.content {
            let query = text
                .strip_prefix("Web search results for \"")
                .and_then(|rest| rest.split_once("\":\n\n"))
                .map(|(q, _)| q.to_string())
                .unwrap_or_else(|| "search".to_string());
            *text = format_search_results(&query, &deduped);
        }
    }
}

async fn deduplicate_and_merge_context_files(
    ctx: Arc<dyn PPContextTrait>,
    context_files: Vec<ContextFile>,
    existing_messages: &[ChatMessage],
) -> (Vec<ContextFile>, Vec<String>) {
    let mut file_groups: BTreeMap<String, Vec<ContextFile>> = BTreeMap::new();

    for cf in context_files {
        let key = if cf.file_name.contains("://") {
            cf.file_name.clone()
        } else {
            canonical_path_simple(&cf.file_name)
                .to_string_lossy()
                .to_string()
        };
        file_groups.entry(key).or_default().push(cf);
    }

    let mut result = Vec::new();
    let mut notes = Vec::new();

    for (_canonical, mut files) in file_groups {
        if files.len() == 1 {
            let cf = files.remove(0);
            if let Some((msg_idx, tool_name)) = find_coverage_in_history(&cf, existing_messages) {
                if !record_deduplicated_file(ctx.as_ref(), &cf, existing_messages, msg_idx, None)
                    .await
                {
                    result.push(cf);
                    continue;
                }
                let range = if cf.line1 > 0 && cf.line2 > 0 {
                    format!("{}:{}-{}", cf.file_name, cf.line1, cf.line2)
                } else {
                    cf.file_name.clone()
                };
                notes.push(format!(
                    "📎 `{}` already in context (message #{}, via `{}`). Skipping to save tokens.",
                    range,
                    msg_idx + 1,
                    tool_name
                ));
            } else {
                result.push(cf);
            }
            continue;
        }

        files.sort_by_key(|f| f.line1);
        let merged = merge_overlapping_ranges(files);

        for cf in merged {
            if let Some((msg_idx, tool_name)) = find_coverage_in_history(&cf, existing_messages) {
                if !record_deduplicated_file(ctx.as_ref(), &cf, existing_messages, msg_idx, None)
                    .await
                {
                    result.push(cf);
                    continue;
                }
                let range = if cf.line1 > 0 && cf.line2 > 0 {
                    format!("{}:{}-{}", cf.file_name, cf.line1, cf.line2)
                } else {
                    cf.file_name.clone()
                };
                notes.push(format!(
                    "📎 `{}` already in context (message #{}, via `{}`). Skipping to save tokens.",
                    range,
                    msg_idx + 1,
                    tool_name
                ));
            } else {
                result.push(cf);
            }
        }
    }

    (result, notes)
}

async fn record_deduplicated_file(
    ctx: &dyn PPContextTrait,
    file: &ContextFile,
    messages: &[ChatMessage],
    message_index: usize,
    current_text: Option<&str>,
) -> bool {
    let loaded;
    let text = match current_text {
        Some(text) => text,
        None => match ctx
            .read_file_for_dedup(&PathBuf::from(&file.file_name))
            .await
        {
            Ok(text) => {
                loaded = text;
                &loaded
            }
            Err(_) => return false,
        },
    };
    if file.file_rev.as_ref() != Some(&official_text_hashing_function(text)) {
        return false;
    }
    let message = &messages[message_index];
    if message.extra.get("_ui_only").and_then(|v| v.as_bool()) == Some(true)
        || message.extra.get("ui_only").and_then(|v| v.as_bool()) == Some(true)
    {
        return false;
    }
    let files = match &message.content {
        ChatContent::ContextFiles(files) => files.clone(),
        ChatContent::SimpleText(text) => match serde_json::from_str::<Vec<ContextFile>>(text) {
            Ok(files) => files,
            Err(_) => return false,
        },
        _ => return false,
    };
    for existing in files {
        if canonical_path_simple(&existing.file_name) != canonical_path_simple(&file.file_name)
            || existing.file_rev != file.file_rev
            || has_truncation_markers(&existing.file_content)
        {
            continue;
        }
        if file.file_name.contains("://") {
            return existing.file_content == text;
        }
        let lines: Vec<_> = text.lines().collect();
        let start = normalize_line_start(file.line1, lines.len());
        let end = normalize_line_end(file.line2, lines.len(), start);
        let shown: Vec<_> = existing
            .file_content
            .lines()
            .filter_map(|line| {
                let (number, content) = line.split_once(" | ")?;
                Some((number.trim().parse::<usize>().ok()?, content, line))
            })
            .filter(|(number, _, _)| *number > start && *number <= end)
            .collect();
        if shown.len() != end - start
            || !shown
                .iter()
                .enumerate()
                .all(|(index, (number, content, _))| {
                    *number == start + index + 1 && *content == lines[start + index]
                })
            || (lines.is_empty() && !existing.file_content.is_empty())
        {
            continue;
        }
        let mut visible = file.clone();
        visible.line1 = start + 1;
        visible.line2 = end;
        visible.file_content = shown
            .iter()
            .map(|(_, _, line)| *line)
            .collect::<Vec<_>>()
            .join("\n");
        ctx.record_deduplicated_read(&visible, text);
        return true;
    }
    false
}

fn merge_overlapping_ranges(mut files: Vec<ContextFile>) -> Vec<ContextFile> {
    if files.is_empty() {
        return files;
    }

    let mut result = Vec::new();
    let mut current = files.remove(0);

    for next in files {
        let curr_start = if current.line1 == 0 { 1 } else { current.line1 };
        let curr_end = if current.line2 == 0 {
            usize::MAX
        } else {
            current.line2
        };
        let next_start = if next.line1 == 0 { 1 } else { next.line1 };
        let next_end = if next.line2 == 0 {
            usize::MAX
        } else {
            next.line2
        };

        if curr_end == usize::MAX || next_start <= curr_end.saturating_add(1) {
            current.line1 = curr_start.min(next_start);
            current.line2 = if curr_end == usize::MAX || next_end == usize::MAX {
                0
            } else {
                curr_end.max(next_end)
            };
            current.usefulness = current.usefulness.max(next.usefulness);
            for sym in next.symbols {
                if !current.symbols.contains(&sym) {
                    current.symbols.push(sym);
                }
            }
        } else {
            result.push(current);
            current = next;
        }
    }
    result.push(current);
    result
}

fn has_truncation_markers(content: &str) -> bool {
    content.contains("...") || content.contains("⋮") || content.contains("omitted")
}

fn find_coverage_in_history(cf: &ContextFile, messages: &[ChatMessage]) -> Option<(usize, String)> {
    let is_virtual = cf.file_name.contains("://");
    let cf_canonical = if is_virtual {
        PathBuf::from(&cf.file_name)
    } else {
        canonical_path_simple(&cf.file_name)
    };
    let cf_start = if cf.line1 == 0 { 1 } else { cf.line1 };
    let cf_end = if cf.line2 == 0 { usize::MAX } else { cf.line2 };

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != "context_file" {
            continue;
        }

        let files_to_check: Vec<ContextFile> = match &msg.content {
            ChatContent::ContextFiles(files) => files.clone(),
            ChatContent::SimpleText(text) => {
                if let Ok(parsed) = serde_json::from_str::<Vec<ContextFile>>(text) {
                    parsed
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        for existing in files_to_check {
            let existing_canonical = if existing.file_name.contains("://") {
                PathBuf::from(&existing.file_name)
            } else {
                canonical_path_simple(&existing.file_name)
            };
            if existing_canonical != cf_canonical {
                continue;
            }
            let same_rev = matches!(
                (&cf.file_rev, &existing.file_rev),
                (Some(a), Some(b)) if a == b
            );
            if !same_rev {
                continue;
            }
            if has_truncation_markers(&existing.file_content) {
                continue;
            }
            let ex_start = if existing.line1 == 0 {
                1
            } else {
                existing.line1
            };
            let ex_end = if existing.line2 == 0 {
                usize::MAX
            } else {
                existing.line2
            };
            if ex_start <= cf_start && ex_end >= cf_end {
                return Some((idx, msg.tool_call_id.clone()));
            }
        }
    }
    None
}

async fn postprocess_context_file_results(
    ctx: Arc<dyn PPContextTrait>,
    tokenizer: Option<Arc<Tokenizer>>,
    context_files: Vec<ContextFile>,
    tokens_limit: usize,
    mut pp_settings: PostprocessSettings,
    existing_messages: &[ChatMessage],
) -> (Option<ChatMessage>, Vec<String>, usize) {
    let (deduped_files, dedup_notes) =
        deduplicate_and_merge_context_files(ctx.clone(), context_files, existing_messages).await;

    let (skip_pp_files, mut pp_files): (Vec<_>, Vec<_>) =
        deduped_files.into_iter().partition(|cf| cf.skip_pp);

    pp_settings.close_small_gaps = true;
    if pp_settings.max_files_n == 0 {
        pp_settings.max_files_n = 25;
    }

    let total_files = pp_files.len() + skip_pp_files.len();
    let pp_ratio = if total_files > 0 {
        pp_files.len() * 100 / total_files
    } else {
        50
    };
    let tokens_for_pp = tokens_limit * pp_ratio / 100;
    let tokens_for_skip = tokens_limit.saturating_sub(tokens_for_pp);

    let (pp_result, pp_notes) = postprocess_context_files(
        ctx.clone(),
        &mut pp_files,
        tokenizer.clone(),
        tokens_for_pp,
        false,
        &pp_settings,
    )
    .await;

    let (skip_result, skip_notes) = fill_skip_pp_files_with_budget(
        ctx.clone(),
        tokenizer.clone(),
        skip_pp_files,
        tokens_for_skip,
        existing_messages,
    )
    .await;

    let notes: Vec<String> = dedup_notes
        .into_iter()
        .chain(pp_notes)
        .chain(skip_notes)
        .collect();

    let all_files: Vec<_> = pp_result
        .into_iter()
        .chain(skip_result)
        .filter(|cf| !cf.file_name.is_empty())
        .collect();

    if all_files.is_empty() {
        return (None, notes, 0);
    }

    let tokens_used: usize = all_files
        .iter()
        .map(|cf| count_text_tokens_with_fallback(tokenizer.clone(), &cf.file_content))
        .sum();

    (
        Some(ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(all_files),
            ..Default::default()
        }),
        notes,
        tokens_used,
    )
}

const MIN_PER_FILE_BUDGET: usize = 50;

async fn fill_skip_pp_files_with_budget(
    ctx: Arc<dyn PPContextTrait>,
    tokenizer: Option<Arc<Tokenizer>>,
    files: Vec<ContextFile>,
    tokens_limit: usize,
    existing_messages: &[ChatMessage],
) -> (Vec<ContextFile>, Vec<String>) {
    if files.is_empty() {
        return (vec![], vec![]);
    }

    if tokens_limit < MIN_PER_FILE_BUDGET {
        return (
            vec![],
            vec![format!(
                "⚠️ {} files skipped: token budget ({}) below minimum ({})",
                files.len(),
                tokens_limit,
                MIN_PER_FILE_BUDGET
            )],
        );
    }

    let max_files_by_budget = tokens_limit / MIN_PER_FILE_BUDGET;
    let files_to_skip = files.len().saturating_sub(max_files_by_budget);
    let files: Vec<_> = files.into_iter().take(max_files_by_budget).collect();

    if files.is_empty() {
        return (
            vec![],
            vec![format!(
                "⚠️ {} files skipped due to token budget constraints",
                files_to_skip
            )],
        );
    }

    let max_per_file_budget = limits().max_per_file_budget_tokens;
    let per_file_budget = (tokens_limit / files.len()).min(max_per_file_budget);
    let mut result = Vec::new();
    let mut notes = Vec::new();

    if files_to_skip > 0 {
        notes.push(format!(
            "⚠️ {} files skipped due to token budget constraints",
            files_to_skip
        ));
    }

    for mut cf in files {
        // If content is already provided (e.g., skill:// virtual URIs), use it directly
        if !cf.file_content.trim().is_empty() {
            cf.file_rev = Some(official_text_hashing_function(&cf.file_content));

            if let Some(dup_info) = find_duplicate_in_history(&cf, existing_messages) {
                let current_text = if cf.file_name.contains("://") {
                    Some(cf.file_content.clone())
                } else {
                    ctx.read_file_for_dedup(&PathBuf::from(&cf.file_name))
                        .await
                        .ok()
                };
                if let Some(text) = current_text {
                    if record_deduplicated_file(
                        ctx.as_ref(),
                        &cf,
                        existing_messages,
                        dup_info.0,
                        Some(&text),
                    )
                    .await
                    {
                        let range = if cf.line1 > 0 && cf.line2 > 0 {
                            format!("{}:{}-{}", cf.file_name, cf.line1, cf.line2)
                        } else {
                            cf.file_name.clone()
                        };
                        notes.push(format!(
                            "📎 Skipped `{}`: already retrieved in message #{} via `{}`.",
                            range,
                            dup_info.0 + 1,
                            dup_info.1
                        ));
                        continue;
                    }
                }
            }

            cf.file_content = truncate_tool_text_lines(&cf.file_content);

            let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &cf.file_content);
            if tokens > per_file_budget {
                let total_lines = cf.file_content.lines().count();
                let mut truncated = String::new();
                let mut kept_lines = 0usize;
                for line in cf.file_content.lines() {
                    let candidate = if truncated.is_empty() {
                        line.to_string()
                    } else {
                        format!("{}\n{}", truncated, line)
                    };
                    if count_text_tokens_with_fallback(tokenizer.clone(), &candidate)
                        > per_file_budget
                    {
                        if !truncated.is_empty() {
                            truncated.push_str("\n\n");
                            truncated.push_str(&per_file_truncation_marker(
                                kept_lines,
                                total_lines,
                                tokens,
                                per_file_budget,
                            ));
                        }
                        break;
                    }
                    truncated = candidate;
                    kept_lines += 1;
                }
                cf.file_content = truncated;
            }
            result.push(cf);
            continue;
        }

        match ctx.read_file(&PathBuf::from(&cf.file_name)).await {
            Ok(text) => {
                cf.file_rev = Some(official_text_hashing_function(&text));

                if let Some(dup_info) = find_duplicate_in_history(&cf, existing_messages) {
                    if record_deduplicated_file(
                        ctx.as_ref(),
                        &cf,
                        existing_messages,
                        dup_info.0,
                        Some(&text),
                    )
                    .await
                    {
                        let range = if cf.line1 > 0 && cf.line2 > 0 {
                            format!("{}:{}-{}", cf.file_name, cf.line1, cf.line2)
                        } else {
                            cf.file_name.clone()
                        };
                        notes.push(format!(
                            "📎 Skipped `{}`: already retrieved in message #{} via `{}`.",
                            range,
                            dup_info.0 + 1,
                            dup_info.1
                        ));
                        continue;
                    }
                }

                let lines: Vec<&str> = text.lines().collect();
                let total_lines = lines.len();

                if total_lines == 0 {
                    cf.file_content = String::new();
                    result.push(cf);
                    continue;
                }

                let start = normalize_line_start(cf.line1, total_lines);
                let end = normalize_line_end(cf.line2, total_lines, start);

                let content = format_lines_with_numbers(&lines, start, end);
                let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &content);

                if tokens <= per_file_budget {
                    cf.file_content = content;
                    cf.line1 = start + 1;
                    cf.line2 = end;
                } else {
                    cf.file_content = truncate_file_head_tail(
                        &lines,
                        start,
                        end,
                        tokenizer.clone(),
                        per_file_budget,
                    );
                    cf.line1 = start + 1;
                    cf.line2 = end;
                }
                result.push(cf);
            }
            Err(e) => {
                warn!("Failed to load file {}: {}", cf.file_name, e);
                notes.push(format!("⚠️ Failed to load `{}`: {}", cf.file_name, e));
            }
        }
    }

    (result, notes)
}

fn find_duplicate_in_history(
    cf: &ContextFile,
    messages: &[ChatMessage],
) -> Option<(usize, String)> {
    let is_virtual = cf.file_name.contains("://");
    let cf_canonical = if is_virtual {
        PathBuf::from(&cf.file_name)
    } else {
        canonical_path_simple(&cf.file_name)
    };
    let cf_start = if cf.line1 == 0 { 1 } else { cf.line1 };
    let cf_end = if cf.line2 == 0 { usize::MAX } else { cf.line2 };

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != "context_file" {
            continue;
        }
        if let ChatContent::ContextFiles(files) = &msg.content {
            for existing in files {
                let existing_canonical = if existing.file_name.contains("://") {
                    PathBuf::from(&existing.file_name)
                } else {
                    canonical_path_simple(&existing.file_name)
                };
                if existing_canonical != cf_canonical {
                    continue;
                }
                let same_rev = matches!(
                    (&cf.file_rev, &existing.file_rev),
                    (Some(a), Some(b)) if a == b
                );
                if !same_rev {
                    continue;
                }
                if has_truncation_markers(&existing.file_content) {
                    continue;
                }
                let ex_start = if existing.line1 == 0 {
                    1
                } else {
                    existing.line1
                };
                let ex_end = if existing.line2 == 0 {
                    usize::MAX
                } else {
                    existing.line2
                };
                if ex_start <= cf_start && ex_end >= cf_end {
                    let tool_name = find_tool_name_for_context(messages, idx);
                    return Some((idx, tool_name));
                }
            }
        }
    }
    None
}

fn find_tool_name_for_context(messages: &[ChatMessage], context_idx: usize) -> String {
    for i in (0..context_idx).rev() {
        if messages[i].role == "tool" {
            let tool_call_id = &messages[i].tool_call_id;
            for j in (0..i).rev() {
                if let Some(calls) = messages[j].tool_calls.as_ref() {
                    for call in calls {
                        if &call.id == tool_call_id {
                            return call.function.name.clone();
                        }
                    }
                }
            }
            return "tool".to_string();
        }
    }
    "unknown".to_string()
}

fn normalize_line_start(line1: usize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    if line1 == 0 {
        0
    } else {
        (line1.saturating_sub(1)).min(total.saturating_sub(1))
    }
}

fn normalize_line_end(line2: usize, total: usize, start: usize) -> usize {
    if line2 == 0 {
        total
    } else {
        line2.min(total).max(start)
    }
}

fn format_lines_with_numbers(lines: &[&str], start: usize, end: usize) -> String {
    lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:4} | {}", start + i + 1, truncate_tool_line(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn per_file_truncation_marker(
    shown_lines: usize,
    total_lines: usize,
    total_tokens: usize,
    budget_tokens: usize,
) -> String {
    format!(
        "... (truncated: showing {} of {} lines, {} of {} tokens to fit the token budget; raise pp_max_per_file_budget_tokens in settings)",
        format_thousands(shown_lines),
        format_thousands(total_lines),
        format_thousands(budget_tokens),
        format_thousands(total_tokens)
    )
}

fn prefix_truncation_marker(shown_chars: usize, total_chars: usize) -> String {
    format!(
        "\n... (content truncated: showing {} of {} chars to fit the token budget; raise pp_max_per_file_budget_tokens in settings)",
        format_thousands(shown_chars),
        format_thousands(total_chars)
    )
}

fn truncate_text_prefix_to_token_budget(
    text: &str,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
) -> String {
    if text.is_empty() || tokens_limit == 0 {
        return String::new();
    }

    if count_text_tokens_with_fallback(tokenizer.clone(), text) <= tokens_limit {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();
    let mut low = 0usize;
    let mut high = total_chars;
    let mut best_prefix = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let candidate = if mid < total_chars {
            format!("{}{}", prefix, prefix_truncation_marker(mid, total_chars))
        } else {
            prefix
        };

        let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &candidate);
        if tokens <= tokens_limit {
            best_prefix = mid;
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }

    let mut out: String = chars[..best_prefix].iter().collect();
    if best_prefix < total_chars {
        out.push_str(&prefix_truncation_marker(best_prefix, total_chars));
    }
    out
}

fn truncate_file_head_tail(
    lines: &[&str],
    start: usize,
    end: usize,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
) -> String {
    let total_lines = end - start;
    let head_lines = (total_lines * 80 / 100).max(1);
    let tail_lines = (total_lines * 20 / 100).max(1);

    let mut head_end = start + head_lines.min(total_lines);
    let mut tail_start = end.saturating_sub(tail_lines);

    if tail_start <= head_end {
        tail_start = head_end;
    }

    loop {
        let head_content = format_lines_with_numbers(lines, start, head_end);
        let tail_content = if tail_start < end {
            format_lines_with_numbers(lines, tail_start, end)
        } else {
            String::new()
        };

        let truncation_marker = if tail_start > head_end {
            let shown = total_lines - (tail_start - head_end);
            format!(
                "\n... ({} lines omitted; showing {} of {} lines to fit the token budget; raise pp_max_per_file_budget_tokens in settings) ...\n",
                format_thousands(tail_start - head_end),
                format_thousands(shown),
                format_thousands(total_lines)
            )
        } else {
            String::new()
        };

        let full_content = format!("{}{}{}", head_content, truncation_marker, tail_content);
        let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &full_content);

        if tokens <= tokens_limit {
            return full_content;
        }

        if head_end <= start + 1 {
            return truncate_text_prefix_to_token_budget(
                &full_content,
                tokenizer.clone(),
                tokens_limit,
            );
        }

        head_end = start + (head_end - start) * 80 / 100;
        if tail_start < end {
            tail_start = end - (end - tail_start) * 80 / 100;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{install_postprocessing_limits, PostprocessingLimits};
    use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

    fn make_context_file(name: &str, line1: usize, line2: usize) -> ContextFile {
        make_context_file_with_rev(name, line1, line2, Some("test_rev".to_string()))
    }

    fn make_context_file_with_rev(
        name: &str,
        line1: usize,
        line2: usize,
        file_rev: Option<String>,
    ) -> ContextFile {
        ContextFile {
            file_name: name.to_string(),
            file_content: String::new(),
            line1,
            line2,
            file_rev,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 0.0,
            skip_pp: false,
        }
    }

    fn make_tool_message(content: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    fn make_web_search_tool_message(
        content: &str,
        tool_call_id: &str,
        results: Vec<SearchResult>,
    ) -> ChatMessage {
        let mut msg = make_tool_message(content, tool_call_id);
        msg.extra
            .insert("search_results".to_string(), serde_json::json!(results));
        msg
    }

    fn make_context_file_message(files: Vec<ContextFile>) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(files),
            ..Default::default()
        }
    }

    fn make_assistant_with_tool_calls(tool_names: Vec<&str>) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: Some(
                tool_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| ChatToolCall {
                        id: format!("call_{}", i),
                        index: Some(i),
                        function: ChatToolFunction {
                            name: name.to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                        started_at_ms: None,
                        completed_at_ms: None,
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn test_tool_budget_from_n_ctx() {
        let _guard = crate::config::test_limits_lock();
        install_postprocessing_limits(PostprocessingLimits::default());

        let budget = ToolBudget::try_from_n_ctx(8192).unwrap();
        assert_eq!(budget.tokens_for_code, 4096);
        assert_eq!(budget.tokens_for_text, 1228);

        let budget_small = ToolBudget::try_from_n_ctx(1000);
        assert!(budget_small.is_err());
        assert!(budget_small.unwrap_err().contains("below minimum"));

        let budget_large = ToolBudget::try_from_n_ctx(128000).unwrap();
        assert_eq!(budget_large.tokens_for_code, 64000);
        assert_eq!(budget_large.tokens_for_text, 19200);
    }

    #[test]
    fn test_tool_budget_respects_installed_ceiling() {
        let _guard = crate::config::test_limits_lock();
        install_postprocessing_limits(PostprocessingLimits::default());
        assert_eq!(
            ToolBudget::try_from_n_ctx(1_000_000)
                .unwrap()
                .tokens_for_code,
            131_072
        );

        install_postprocessing_limits(PostprocessingLimits {
            max_tool_budget_tokens: 262_144,
            ..PostprocessingLimits::default()
        });
        let raised = ToolBudget::try_from_n_ctx(1_000_000).unwrap();

        install_postprocessing_limits(PostprocessingLimits::default());
        assert_eq!(raised.tokens_for_code, 262_144);
        assert_eq!(raised.tokens_for_text, 262_144 * 30 / 100);
    }

    #[test]
    fn test_truncation_markers_are_quantified() {
        let marker = per_file_truncation_marker(412, 3180, 9000, 1200);
        assert!(marker.contains("showing 412 of 3,180 lines"));
        assert!(marker.contains("1,200 of 9,000 tokens"));
        assert!(marker.contains("pp_max_per_file_budget_tokens"));

        let prefix_marker = prefix_truncation_marker(100, 48213);
        assert!(prefix_marker.contains("showing 100 of 48,213 chars"));
        assert!(prefix_marker.contains("pp_max_per_file_budget_tokens"));
    }

    #[tokio::test]
    async fn skip_pp_prefilled_one_liner_uses_the_tool_line_cap() {
        let mut file = make_context_file("minified.json", 1, 1);
        file.skip_pp = true;
        file.file_content = format!("HEAD{}TAIL", "x".repeat(50_000));

        let (result, notes) =
            fill_skip_pp_files_with_budget(Arc::new(NoopPPContext), None, vec![file], 100_000, &[])
                .await;

        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_content.chars().count(), 2_000);
        assert!(result[0].file_content.starts_with("HEAD"));
        assert!(result[0].file_content.ends_with("TAIL"));
        assert!(result[0].file_content.contains("chars omitted"));
        assert!(result[0].file_content.contains("tool_max_line_chars"));
        assert_eq!((result[0].line1, result[0].line2), (1, 1));
    }

    #[test]
    fn test_find_duplicate_in_history_partial_overlap_not_covered() {
        let cf = make_context_file("test.rs", 5, 15);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 10,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_duplicate_in_history_full_file_not_covered_by_partial() {
        let cf = make_context_file("test.rs", 0, 0);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 50, 100,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_none());
    }

    #[test]
    fn test_truncate_file_head_tail_single_line_respects_budget() {
        let long_line = "x".repeat(200_000);
        let lines = vec![long_line.as_str()];
        let token_budget = 120;
        let result = truncate_file_head_tail(&lines, 0, 1, None, token_budget);
        let used = count_text_tokens_with_fallback(None, &result);

        assert!(used <= token_budget);
        assert!(result.contains("content truncated"));
    }

    #[test]
    fn test_find_tool_name_correct_tool_call_id() {
        let messages = vec![
            make_assistant_with_tool_calls(vec!["tree", "cat"]),
            make_tool_message("tree result", "call_0"),
            make_context_file_message(vec![make_context_file("test.rs", 1, 10)]),
        ];
        let name = find_tool_name_for_context(&messages, 2);
        assert_eq!(name, "tree");
    }

    #[test]
    fn test_deduplicate_web_search_tool_results_rewrites_extra_and_text() {
        let assistant = make_assistant_with_tool_calls(vec!["web_search"]);
        let mut tool_messages = vec![make_web_search_tool_message(
            "Web search results for \"rust\":\n\n1. [Rust Book](https://doc.rust-lang.org/book/)\n   Official book\n\n2. [Rust Book](https://doc.rust-lang.org/book/)\n   Duplicate\n",
            "call_0",
            vec![
                SearchResult {
                    title: "Rust Book".to_string(),
                    url: "https://doc.rust-lang.org/book/".to_string(),
                    snippet: "Official book".to_string(),
                    source: Some("searxng".to_string()),
                },
                SearchResult {
                    title: "Rust Book".to_string(),
                    url: "https://doc.rust-lang.org/book".to_string(),
                    snippet: "".to_string(),
                    source: Some("duckduckgo".to_string()),
                },
            ],
        )];

        deduplicate_web_search_tool_results(&mut tool_messages, &[assistant]);

        let msg = &tool_messages[0];
        let extra_results = msg
            .extra
            .get("search_results")
            .and_then(|v| v.as_array())
            .expect("search_results array");
        assert_eq!(extra_results.len(), 1);

        let ChatContent::SimpleText(text) = &msg.content else {
            panic!("expected simple text");
        };
        assert!(text.contains("1. [Rust Book](https://doc.rust-lang.org/book)"));
        assert!(!text.contains("2. [Rust Book]"));
    }

    #[tokio::test]
    async fn test_deduplicate_same_file_different_tools() {
        let files = vec![
            make_context_file("test.rs", 1, 50),
            make_context_file("test.rs", 40, 100),
            make_context_file("other.rs", 1, 20),
        ];
        let (result, _notes) =
            deduplicate_and_merge_context_files(Arc::new(NoopPPContext), files, &[]).await;
        assert_eq!(result.len(), 2);
        let test_file = result.iter().find(|f| f.file_name == "test.rs").unwrap();
        assert_eq!(test_file.line1, 1);
        assert_eq!(test_file.line2, 100);
    }

    #[tokio::test]
    async fn test_deduplicate_against_history() {
        let text = "content\n".repeat(100);
        let mut file = make_context_file_with_rev(
            "test.rs",
            1,
            50,
            Some(official_text_hashing_function(&text)),
        );
        let files = vec![file.clone()];
        file.line2 = 100;
        file.file_content = format_lines_with_numbers(&text.lines().collect::<Vec<_>>(), 0, 100);
        let history = vec![make_context_file_message(vec![file])];
        let (result, notes) = deduplicate_and_merge_context_files(
            Arc::new(StaticPPContext {
                content: text,
            }),
            files,
            &history,
        )
        .await;
        assert_eq!(result.len(), 0);
        assert!(!notes.is_empty());
    }

    #[tokio::test]
    async fn test_deduplicate_partial_coverage() {
        let files = vec![make_context_file("test.rs", 80, 150)];
        let history = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 100,
        )])];
        let (result, _notes) =
            deduplicate_and_merge_context_files(Arc::new(NoopPPContext), files, &history).await;
        assert_eq!(result.len(), 1);
    }

    struct NoopPPContext;

    #[async_trait::async_trait]
    impl PPContextTrait for NoopPPContext {
        async fn read_file(&self, _path: &PathBuf) -> Result<String, String> {
            Err("no files in tests".to_string())
        }
        async fn correct_to_nearest_filename(&self, _path: &str, _limit: usize) -> Vec<String> {
            vec![]
        }
        async fn shortify_paths(&self, paths: &[String]) -> Vec<String> {
            paths.to_vec()
        }
        async fn doc_defs_for_path(
            &self,
            _path: &str,
        ) -> Vec<Arc<refact_core::ast_types::AstDefinition>> {
            vec![]
        }
        fn canonical_path(&self, path: &str) -> PathBuf {
            PathBuf::from(path)
        }
    }

    struct StaticPPContext {
        content: String,
    }

    #[async_trait::async_trait]
    impl PPContextTrait for StaticPPContext {
        async fn read_file(&self, _path: &PathBuf) -> Result<String, String> {
            Ok(self.content.clone())
        }
        async fn correct_to_nearest_filename(&self, path: &str, _limit: usize) -> Vec<String> {
            vec![path.to_string()]
        }
        async fn shortify_paths(&self, paths: &[String]) -> Vec<String> {
            paths.to_vec()
        }
        async fn doc_defs_for_path(
            &self,
            _path: &str,
        ) -> Vec<Arc<refact_core::ast_types::AstDefinition>> {
            vec![]
        }
        fn canonical_path(&self, path: &str) -> PathBuf {
            PathBuf::from(path)
        }
    }

    fn small_budget() -> ToolBudget {
        ToolBudget {
            tokens_for_code: 200,
            tokens_for_text: 60,
        }
    }

    #[tokio::test]
    async fn edit_excerpts_share_budget_without_mutating_ui_diffs() {
        let messages: Vec<_> = (0..4)
            .map(|index| {
                let mut message = make_tool_message(
                    &format!(r#"[{{"raw_diff":{index}}}]"#),
                    &format!("call_{index}"),
                );
                message.role = "diff".into();
                message.extra.insert(
                    "edit_result".into(),
                    serde_json::json!({
                        "version": 1,
                        "model_content": format!("excerpt-{index}: {}", "payload ".repeat(1000))
                    }),
                );
                message
            })
            .collect();
        let originals: Vec<_> = messages
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();
        let result = postprocess_tool_results(
            Arc::new(NoopPPContext),
            None,
            messages,
            vec![],
            small_budget(),
            PostprocessSettings::default(),
            &[],
            &HashSet::new(),
        )
        .await;
        assert_eq!(result.len(), 4);
        let mut total = 0;
        for (index, message) in result.iter().enumerate() {
            assert_eq!(message.content.content_text_only(), originals[index]);
            let text = message.extra["edit_result"]["model_content"]
                .as_str()
                .unwrap();
            assert!(
                text.contains("truncated") || text.contains("Truncated"),
                "{text}"
            );
            total += count_text_tokens_with_fallback(None, text);
        }
        assert!(total <= 200, "aggregate model tokens: {total}");
    }

    #[tokio::test]
    async fn exempt_tool_results_bypass_shared_budget_truncation() {
        let big = "content line\n".repeat(2000);
        let exempt_content = format!("exempt-report: {}", big);
        let messages = vec![
            make_tool_message(&exempt_content, "call_exempt"),
            make_tool_message(&format!("normal-output: {}", big), "call_normal"),
        ];
        let exempt_ids: HashSet<String> = HashSet::from(["call_exempt".to_string()]);

        let result = postprocess_tool_results(
            Arc::new(NoopPPContext),
            None,
            messages,
            vec![],
            small_budget(),
            PostprocessSettings::default(),
            &[],
            &exempt_ids,
        )
        .await;

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].tool_call_id, "call_exempt");
        assert_eq!(result[0].content.content_text_only(), exempt_content);
        assert_eq!(result[1].tool_call_id, "call_normal");
        assert!(result[1]
            .content
            .content_text_only()
            .contains("Truncated: too many tokens"));
    }

    #[tokio::test]
    async fn empty_exempt_set_keeps_shared_budget_for_all() {
        let big = "content line\n".repeat(2000);
        let messages = vec![
            make_tool_message(&format!("first: {}", big), "call_0"),
            make_tool_message(&format!("second: {}", big), "call_1"),
        ];

        let result = postprocess_tool_results(
            Arc::new(NoopPPContext),
            None,
            messages,
            vec![],
            small_budget(),
            PostprocessSettings::default(),
            &[],
            &HashSet::new(),
        )
        .await;

        assert_eq!(result.len(), 2);
        for msg in &result {
            assert!(msg
                .content
                .content_text_only()
                .contains("Truncated: too many tokens"));
        }
    }
}
