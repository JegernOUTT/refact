use std::collections::{BTreeMap, HashMap};

use std::path::PathBuf;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;
use tracing::warn;

use crate::ast::chunk_utils::official_text_hashing_function;
use crate::call_validation::{ChatContent, ChatMessage, ContextFile, PostprocessSettings};
use crate::files_correction::canonical_path;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::at_commands::at_web_search::{format_search_results, SearchResult};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::postprocessing::pp_plain_text::postprocess_plain_text;
use crate::tokens::count_text_tokens_with_fallback;

const MIN_CONTEXT_SIZE: usize = 8192;
const MAX_TOOL_BUDGET: usize = 32768;

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
        let total = (n_ctx / 2).max(4096).min(MAX_TOOL_BUDGET);
        Ok(Self {
            tokens_for_code: total,
            tokens_for_text: total * 30 / 100,
        })
    }
}

pub async fn postprocess_tool_results(
    gcx: Arc<ARwLock<GlobalContext>>,
    tokenizer: Option<Arc<Tokenizer>>,
    tool_messages: Vec<ChatMessage>,
    context_files: Vec<ContextFile>,
    budget: ToolBudget,
    pp_settings: PostprocessSettings,
    existing_messages: &[ChatMessage],
) -> Vec<ChatMessage> {
    let mut result = Vec::new();

    let (diff_messages, other_messages): (Vec<_>, Vec<_>) =
        tool_messages.into_iter().partition(|m| m.role == "diff");

    result.extend(diff_messages);

    let total_budget = budget.tokens_for_code;
    let text_budget = if context_files.is_empty() {
        total_budget
    } else if other_messages.is_empty() {
        0
    } else {
        budget.tokens_for_text
    };

    let (text_messages, text_remaining) =
        postprocess_plain_text(other_messages, tokenizer.clone(), text_budget, &None).await;
    result.extend(text_messages);

    deduplicate_web_search_tool_results(&mut result, existing_messages);

    let code_budget = total_budget.saturating_sub(text_budget) + text_remaining;

    let (file_message, notes, _code_used) = if !context_files.is_empty() {
        postprocess_context_file_results(
            gcx,
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

fn deduplicate_and_merge_context_files(
    context_files: Vec<ContextFile>,
    existing_messages: &[ChatMessage],
) -> (Vec<ContextFile>, Vec<String>) {
    let mut file_groups: BTreeMap<String, Vec<ContextFile>> = BTreeMap::new();

    for cf in context_files {
        let key = if cf.file_name.contains("://") {
            cf.file_name.clone()
        } else {
            canonical_path(&cf.file_name).to_string_lossy().to_string()
        };
        file_groups.entry(key).or_default().push(cf);
    }

    let mut result = Vec::new();
    let mut notes = Vec::new();

    for (_canonical, mut files) in file_groups {
        if files.len() == 1 {
            let cf = files.remove(0);
            if let Some((msg_idx, tool_name)) = find_coverage_in_history(&cf, existing_messages) {
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
        canonical_path(&cf.file_name)
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
                canonical_path(&existing.file_name)
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
    gcx: Arc<ARwLock<GlobalContext>>,
    tokenizer: Option<Arc<Tokenizer>>,
    context_files: Vec<ContextFile>,
    tokens_limit: usize,
    mut pp_settings: PostprocessSettings,
    existing_messages: &[ChatMessage],
) -> (Option<ChatMessage>, Vec<String>, usize) {
    let (deduped_files, dedup_notes) =
        deduplicate_and_merge_context_files(context_files, existing_messages);

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
        gcx.clone(),
        &mut pp_files,
        tokenizer.clone(),
        tokens_for_pp,
        false,
        &pp_settings,
    )
    .await;

    let (skip_result, skip_notes) = fill_skip_pp_files_with_budget(
        gcx.clone(),
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
const MAX_PER_FILE_BUDGET: usize = 32768;

async fn fill_skip_pp_files_with_budget(
    gcx: Arc<ARwLock<GlobalContext>>,
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

    let per_file_budget = (tokens_limit / files.len()).min(MAX_PER_FILE_BUDGET);
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

            let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &cf.file_content);
            if tokens > per_file_budget {
                // Simple line-based truncation for prefilled content (markdown/instructions)
                let mut truncated = String::new();
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
                            truncated.push_str("\n\n... (content truncated to fit token budget)");
                        }
                        break;
                    }
                    truncated = candidate;
                }
                cf.file_content = truncated;
            }
            result.push(cf);
            continue;
        }

        match get_file_text_from_memory_or_disk(gcx.clone(), &PathBuf::from(&cf.file_name)).await {
            Ok(text) => {
                cf.file_rev = Some(official_text_hashing_function(&text));

                if let Some(dup_info) = find_duplicate_in_history(&cf, existing_messages) {
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
        canonical_path(&cf.file_name)
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
                    canonical_path(&existing.file_name)
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
        .map(|(i, line)| format!("{:4} | {}", start + i + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_text_prefix_to_token_budget(
    text: &str,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
    marker: &str,
) -> String {
    if text.is_empty() || tokens_limit == 0 {
        return String::new();
    }

    if count_text_tokens_with_fallback(tokenizer.clone(), text) <= tokens_limit {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    let mut best_prefix = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let candidate = if mid < chars.len() {
            format!("{}{}", prefix, marker)
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
    if best_prefix < chars.len() {
        out.push_str(marker);
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
            format!("\n... ({} lines omitted) ...\n", tail_start - head_end)
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
                "\n... (content truncated to fit token budget)",
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
    use crate::call_validation::{ChatToolCall, ChatToolFunction};

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
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn test_tool_budget_from_n_ctx() {
        let budget = ToolBudget::try_from_n_ctx(8192).unwrap();
        assert_eq!(budget.tokens_for_code, 4096);
        assert_eq!(budget.tokens_for_text, 1228);

        let budget_small = ToolBudget::try_from_n_ctx(1000);
        assert!(budget_small.is_err());
        assert!(budget_small.unwrap_err().contains("below minimum"));

        // Large context models are capped at MAX_TOOL_BUDGET (32768) to prevent bloated context
        let budget_large = ToolBudget::try_from_n_ctx(128000).unwrap();
        assert_eq!(budget_large.tokens_for_code, 32768);
        assert_eq!(budget_large.tokens_for_text, 9830);
    }

    #[test]
    fn test_normalize_line_start() {
        assert_eq!(normalize_line_start(0, 100), 0);
        assert_eq!(normalize_line_start(1, 100), 0);
        assert_eq!(normalize_line_start(10, 100), 9);
        assert_eq!(normalize_line_start(200, 100), 99); // clamp to last valid index
        assert_eq!(normalize_line_start(5, 0), 0); // empty file edge case
    }

    #[test]
    fn test_normalize_line_end() {
        assert_eq!(normalize_line_end(0, 100, 0), 100);
        assert_eq!(normalize_line_end(50, 100, 0), 50);
        assert_eq!(normalize_line_end(200, 100, 0), 100);
        assert_eq!(normalize_line_end(10, 100, 20), 20);
    }

    #[test]
    fn test_format_lines_with_numbers() {
        let lines = vec!["line1", "line2", "line3", "line4", "line5"];
        let result = format_lines_with_numbers(&lines, 0, 3);
        assert!(result.contains("   1 | line1"));
        assert!(result.contains("   2 | line2"));
        assert!(result.contains("   3 | line3"));
        assert!(!result.contains("line4"));

        let result2 = format_lines_with_numbers(&lines, 2, 5);
        assert!(result2.contains("   3 | line3"));
        assert!(result2.contains("   4 | line4"));
        assert!(result2.contains("   5 | line5"));
    }

    #[test]
    fn test_find_duplicate_in_history_no_match() {
        let cf = make_context_file("new_file.rs", 1, 10);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "other.rs", 1, 10,
        )])];
        assert!(find_duplicate_in_history(&cf, &messages).is_none());
    }

    #[test]
    fn test_find_duplicate_in_history_exact_match() {
        let cf = make_context_file("test.rs", 1, 10);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 10,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, 0);
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
    fn test_find_duplicate_in_history_fully_covered() {
        let cf = make_context_file("test.rs", 5, 10);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 20,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_some());
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
    fn test_find_duplicate_in_history_full_file_covered_by_full() {
        let cf = make_context_file("test.rs", 0, 0);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 0, 0,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_some());
    }

    #[test]
    fn test_find_tool_name_for_context() {
        let messages = vec![
            make_assistant_with_tool_calls(vec!["cat"]),
            make_tool_message("result", "call_0"),
            make_context_file_message(vec![make_context_file("test.rs", 1, 10)]),
        ];
        let name = find_tool_name_for_context(&messages, 2);
        assert_eq!(name, "cat");
    }

    #[test]
    fn test_find_tool_name_for_context_no_tool() {
        let messages = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 10,
        )])];
        let name = find_tool_name_for_context(&messages, 0);
        assert_eq!(name, "unknown");
    }

    #[test]
    fn test_truncate_file_head_tail() {
        let lines: Vec<&str> = (0..100).map(|_| "content").collect();
        let result = truncate_file_head_tail(&lines, 0, 100, None, 50);
        assert!(result.contains("   1 |"));
        assert!(result.contains("omitted"));
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
    fn test_find_duplicate_path_normalization() {
        let cf = make_context_file("src/main.rs", 1, 10);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "src/main.rs",
            1,
            10,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_some());
    }

    #[test]
    fn test_find_duplicate_different_files_same_basename() {
        let cf = make_context_file("src/a/main.rs", 1, 10);
        let messages = vec![make_context_file_message(vec![make_context_file(
            "src/b/main.rs",
            1,
            10,
        )])];
        let result = find_duplicate_in_history(&cf, &messages);
        assert!(result.is_none());
    }

    #[test]
    fn test_budget_ratio_all_skip_pp() {
        let skip_files = vec![
            ContextFile {
                skip_pp: true,
                ..make_context_file("a.rs", 1, 10)
            },
            ContextFile {
                skip_pp: true,
                ..make_context_file("b.rs", 1, 10)
            },
        ];
        let pp_files: Vec<ContextFile> = vec![];
        let total = skip_files.len() + pp_files.len();
        let pp_ratio = if total > 0 {
            pp_files.len() * 100 / total
        } else {
            50
        };
        assert_eq!(pp_ratio, 0);
    }

    #[test]
    fn test_budget_ratio_all_pp() {
        let skip_files: Vec<ContextFile> = vec![];
        let pp_files = vec![
            make_context_file("a.rs", 1, 10),
            make_context_file("b.rs", 1, 10),
        ];
        let total = skip_files.len() + pp_files.len();
        let pp_ratio = if total > 0 {
            pp_files.len() * 100 / total
        } else {
            50
        };
        assert_eq!(pp_ratio, 100);
    }

    #[test]
    fn test_budget_ratio_mixed() {
        let skip_files = vec![ContextFile {
            skip_pp: true,
            ..make_context_file("a.rs", 1, 10)
        }];
        let pp_files = vec![
            make_context_file("b.rs", 1, 10),
            make_context_file("c.rs", 1, 10),
            make_context_file("d.rs", 1, 10),
        ];
        let total = skip_files.len() + pp_files.len();
        let pp_ratio = if total > 0 {
            pp_files.len() * 100 / total
        } else {
            50
        };
        assert_eq!(pp_ratio, 75);
    }

    #[test]
    fn test_find_tool_name_multiple_tools() {
        let messages = vec![
            make_assistant_with_tool_calls(vec!["tree", "cat", "search"]),
            make_tool_message("tree result", "call_0"),
            make_tool_message("cat result", "call_1"),
            make_context_file_message(vec![make_context_file("test.rs", 1, 10)]),
        ];
        let name = find_tool_name_for_context(&messages, 3);
        assert_eq!(name, "cat");
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

    #[test]
    fn test_merge_overlapping_ranges() {
        let files = vec![
            make_context_file("test.rs", 1, 50),
            make_context_file("test.rs", 40, 100),
        ];
        let merged = merge_overlapping_ranges(files);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].line1, 1);
        assert_eq!(merged[0].line2, 100);
    }

    #[test]
    fn test_merge_adjacent_ranges() {
        let files = vec![
            make_context_file("test.rs", 1, 50),
            make_context_file("test.rs", 51, 100),
        ];
        let merged = merge_overlapping_ranges(files);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].line1, 1);
        assert_eq!(merged[0].line2, 100);
    }

    #[test]
    fn test_merge_non_overlapping_ranges() {
        let files = vec![
            make_context_file("test.rs", 1, 50),
            make_context_file("test.rs", 100, 150),
        ];
        let merged = merge_overlapping_ranges(files);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_deduplicate_same_file_different_tools() {
        let files = vec![
            make_context_file("test.rs", 1, 50),
            make_context_file("test.rs", 40, 100),
            make_context_file("other.rs", 1, 20),
        ];
        let (result, _notes) = deduplicate_and_merge_context_files(files, &[]);
        assert_eq!(result.len(), 2);
        let test_file = result.iter().find(|f| f.file_name == "test.rs").unwrap();
        assert_eq!(test_file.line1, 1);
        assert_eq!(test_file.line2, 100);
    }

    #[test]
    fn test_deduplicate_against_history() {
        let files = vec![make_context_file("test.rs", 1, 50)];
        let history = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 100,
        )])];
        let (result, notes) = deduplicate_and_merge_context_files(files, &history);
        assert_eq!(result.len(), 0);
        assert!(!notes.is_empty());
    }

    #[test]
    fn test_deduplicate_partial_coverage() {
        let files = vec![make_context_file("test.rs", 80, 150)];
        let history = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 100,
        )])];
        let (result, _notes) = deduplicate_and_merge_context_files(files, &history);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_find_coverage_in_history() {
        let cf = make_context_file("test.rs", 10, 50);
        let history = vec![make_context_file_message(vec![make_context_file(
            "test.rs", 1, 100,
        )])];
        assert!(find_coverage_in_history(&cf, &history).is_some());

        let cf2 = make_context_file("test.rs", 10, 150);
        assert!(find_coverage_in_history(&cf2, &history).is_none());
    }
}
