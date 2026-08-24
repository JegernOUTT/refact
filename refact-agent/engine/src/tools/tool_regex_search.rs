use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use itertools::Itertools;
use regex::Regex;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tracing::info;

use crate::at_commands::at_commands::{vec_context_file_to_context_tools, AtCommandsContext};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::files_correction::shortify_paths;
use crate::files_in_workspace::{
    check_file_privacy_for_read_preparation_with_context,
    get_file_text_from_memory_or_disk_for_model_context_with_context, prepare_file_read_context,
    FileReadContext,
};
use crate::global_context::GlobalContext;
use crate::tools::scope_utils::{
    format_scope_notices, resolve_scope_with_execution_scope_limited_for_model_context,
    validate_scope_files,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::knowledge_index::format_related_memories_section;
use crate::tools::native_enrichment::{workspace_roots, NativeReferences};

pub struct ToolRegexSearch {
    pub config_path: String,
}

const DEFAULT_CONTEXT_LINES: usize = 5;
const DEFAULT_MAX_FILES: usize = 50;
const DEFAULT_MAX_MATCHES_PER_FILE: usize = 25;
const DEFAULT_MAX_TOTAL_MATCHES: usize = 200;

const CAP_CONTEXT_LINES: usize = 100;
const CAP_MAX_FILES: usize = 1000;
const CAP_MAX_MATCHES_PER_FILE: usize = 1000;
const CAP_MAX_TOTAL_MATCHES: usize = 10000;
const MAX_SEARCH_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SEARCH_CANDIDATE_ENTRIES: usize = 20_000;

const ABORTED_ERROR: &str = "⚠️ search_pattern aborted before completion (cancelled by caller).";

#[derive(Clone, Debug)]
struct RegexMatch {
    file_name: String,
    match_line: usize,    // 1-based
    context_start: usize, // 1-based
    context_end_inclusive: usize,
    preview: String,
}

#[derive(Debug)]
struct RegexSearchOutcome {
    matches: Vec<RegexMatch>,
    files_scanned: usize,
    files_total: usize,
    stopped_early: bool,
}

fn format_preview(
    lines: &[&str],
    start_idx: usize,
    end_idx_exclusive: usize,
    match_line: usize,
) -> String {
    let mut out = String::new();
    for idx in start_idx..end_idx_exclusive {
        let lineno = idx + 1;
        let marker = if lineno == match_line { ">" } else { " " };
        if let Some(line) = lines.get(idx) {
            out.push_str(&format!("{}{:>6} | {}\n", marker, lineno, line));
        }
    }
    out.trim_end().to_string()
}

async fn search_single_file(
    gcx: Arc<GlobalContext>,
    file_path: String,
    regex: &Regex,
    context_lines: usize,
    max_matches_per_file: usize,
    abort_flag: &AtomicBool,
    read_context: &FileReadContext,
) -> Vec<RegexMatch> {
    if abort_flag.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let file_path_buf = PathBuf::from(&file_path);
    let file_content = match get_file_text_from_memory_or_disk_for_model_context_with_context(
        gcx.clone(),
        &file_path_buf,
        read_context,
        Some(MAX_SEARCH_FILE_BYTES as usize),
    )
    .await
    {
        Ok(content) => content.to_string(),
        Err(_) => return Vec::new(),
    };

    let lines: Vec<&str> = file_content.lines().collect();
    let mut file_results = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        if (line_idx & 0x3FF) == 0 && abort_flag.load(Ordering::Relaxed) {
            break;
        }
        if regex.is_match(line) {
            let match_line = line_idx + 1;
            let context_start_idx = line_idx.saturating_sub(context_lines);
            let context_end_excl = (line_idx + context_lines + 1).min(lines.len());
            let preview = format_preview(&lines, context_start_idx, context_end_excl, match_line);
            file_results.push(RegexMatch {
                file_name: file_path.clone(),
                match_line,
                context_start: context_start_idx + 1,
                context_end_inclusive: context_end_excl,
                preview,
            });
            if file_results.len() >= max_matches_per_file {
                break;
            }
        }
    }

    file_results
}

/// Maximum concurrent file reads to avoid overwhelming I/O
const MAX_CONCURRENT_FILE_READS: usize = 32;

async fn search_files_with_regex(
    gcx: Arc<GlobalContext>,
    pattern: &str,
    files_to_search: &[String],
    context_lines: usize,
    max_files: usize,
    max_matches_per_file: usize,
    max_total_matches: usize,
    abort_flag: Arc<AtomicBool>,
    read_context: Arc<FileReadContext>,
) -> Result<RegexSearchOutcome, String> {
    let regex = Regex::new(pattern).map_err(|e| format!("Invalid regex pattern: {}", e))?;
    let regex_arc = Arc::new(regex);

    if abort_flag.load(Ordering::Relaxed) {
        return Err(ABORTED_ERROR.to_string());
    }

    let mut ordered_files = files_to_search.to_vec();
    ordered_files.sort();
    let files_total = ordered_files.len();
    let mut matches = Vec::new();
    let mut files_scanned = 0;
    let mut files_matched = 0;
    'batches: for batch in ordered_files.chunks(MAX_CONCURRENT_FILE_READS) {
        if abort_flag.load(Ordering::Relaxed) {
            return Err(ABORTED_ERROR.to_string());
        }

        let batch_results: Vec<Vec<RegexMatch>> = stream::iter(batch.iter().cloned())
            .map(|file_path| {
                let gcx_clone = gcx.clone();
                let regex_clone = regex_arc.clone();
                let abort_clone = abort_flag.clone();
                let read_context = read_context.clone();
                async move {
                    search_single_file(
                        gcx_clone,
                        file_path,
                        &regex_clone,
                        context_lines,
                        max_matches_per_file,
                        &abort_clone,
                        &read_context,
                    )
                    .await
                }
            })
            .buffered(MAX_CONCURRENT_FILE_READS)
            .collect()
            .await;

        if abort_flag.load(Ordering::Relaxed) {
            return Err(ABORTED_ERROR.to_string());
        }
        files_scanned += batch_results.len();

        for mut file_matches in batch_results {
            if file_matches.is_empty() {
                continue;
            }
            files_matched += 1;
            let remaining = max_total_matches.saturating_sub(matches.len());
            file_matches.truncate(remaining);
            matches.extend(file_matches);
            if files_matched >= max_files || matches.len() >= max_total_matches {
                break 'batches;
            }
        }
    }

    matches.sort_by(|a, b| {
        a.file_name
            .cmp(&b.file_name)
            .then(a.match_line.cmp(&b.match_line))
    });
    Ok(RegexSearchOutcome {
        matches,
        files_scanned,
        files_total,
        stopped_early: files_scanned < files_total,
    })
}

fn path_depth(path: &str) -> usize {
    path.chars().filter(|&c| c == '/' || c == '\\').count()
}

async fn smart_compress_results(
    search_results: &[RegexMatch],
    file_results: &HashMap<String, Vec<&RegexMatch>>,
    gcx: Arc<GlobalContext>,
    pattern: &str,
    max_matches_per_file: usize,
    max_output_bytes: usize,
) -> String {
    let total_matches = search_results.len();
    let total_files = file_results.len();

    let mut content = format!("Regex search results for pattern '{}':\n\n", pattern);
    content.push_str(&format!(
        "Found {} matches across {} files\n\n",
        total_matches, total_files
    ));

    let mut file_paths: Vec<String> = file_results.keys().cloned().collect();

    file_paths.sort_by(|a, b| {
        let a_depth = path_depth(a);
        let b_depth = path_depth(b);
        if a_depth == b_depth {
            a.cmp(b)
        } else {
            a_depth.cmp(&b_depth)
        }
    });

    let mut used_files = HashSet::new();
    let mut estimated_size = content.len();
    let short_paths = shortify_paths(gcx.clone(), &file_paths).await;

    for file_path in file_paths.iter() {
        if used_files.contains(file_path) {
            continue;
        }
        let idx = file_paths.iter().position(|p| p == file_path);
        let short_path = idx.and_then(|i| short_paths.get(i)).unwrap_or(file_path);
        let file_matches = file_results.get(file_path).unwrap();
        let file_header = format!("{}: ({} matches)\n", short_path, file_matches.len());
        estimated_size += file_header.len();
        content.push_str(&file_header);
        let matches_to_show = std::cmp::min(file_matches.len(), max_matches_per_file);
        for file_match in file_matches
            .iter()
            .take(matches_to_show)
            .sorted_by_key(|m| m.match_line)
        {
            let match_line = format!("    line {}\n", file_match.match_line);
            estimated_size += match_line.len();
            content.push_str(&match_line);

            // Indent preview (already line-numbered).
            let preview = file_match
                .preview
                .lines()
                .map(|l| format!("        {}", l))
                .join("\n");
            estimated_size += preview.len() + 2;
            content.push_str(&preview);
            content.push_str("\n\n");

            if estimated_size > max_output_bytes * 3 / 4 {
                break;
            }
        }
        if file_matches.len() > max_matches_per_file {
            let summary = format!(
                "    ... and {} more matches in this file\n",
                file_matches.len() - max_matches_per_file
            );
            estimated_size += summary.len();
            content.push_str(&summary);
        }
        content.push('\n');
        estimated_size += 1;
        used_files.insert(file_path.clone());
        if estimated_size > max_output_bytes * 3 / 4 {
            break;
        }
    }
    if file_paths.len() > used_files.len() {
        let remaining_files = file_paths.len() - used_files.len();
        content.push_str(&format!(
            "⚠️ {} more files not shown (4KB limit). 💡 Use narrower scope or more specific pattern\n",
            remaining_files
        ));
    }
    if estimated_size > max_output_bytes {
        info!(
            "Compressing `search_pattern` output: estimated {} bytes (exceeds 4KB limit)",
            estimated_size
        );
        content.push_str(
            "\n⚠️ Output compressed due to size. 💡 Use cat('file:line') to see specific matches\n",
        );
    }
    content
}

fn parse_usize_arg(args: &HashMap<String, Value>, key: &str) -> Result<Option<usize>, String> {
    match args.get(key) {
        Some(Value::Number(n)) => {
            let value = n
                .as_u64()
                .ok_or_else(|| format!("argument `{}` must be a non-negative integer", key))?;
            let value =
                usize::try_from(value).map_err(|_| format!("argument `{}` is too large", key))?;
            Ok(Some(value))
        }
        Some(Value::String(s)) => s
            .parse::<usize>()
            .map(Some)
            .map_err(|_| format!("argument `{}` must be a non-negative integer", key)),
        Some(v) => Err(format!("argument `{}` is not an integer: {:?}", key, v)),
        None => Ok(None),
    }
}

#[async_trait]
impl Tool for ToolRegexSearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "search_pattern".to_string(),
            display_name: "Regex Search".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Search for text matches inside files using a regular expression pattern.".to_string(),
            input_schema: json_schema_from_params(&[("pattern", "string", "The regular expression used to search file contents. Use (?i) at the start for case-insensitive search."), ("scope", "string", "'workspace' to search all files in workspace, 'dir/subdir/' to search in files within a directory, 'dir/file.ext' to search in a single file."), ("context_lines", "integer", "Lines of context before/after each match (default: 5)."), ("max_files", "integer", "Max files to attach as context (default: 50)."), ("max_matches_per_file", "integer", "Max matches collected per file (default: 25, hard cap 1000). Scanning of a file stops once this many matches are found."), ("max_total_matches", "integer", "Max total matches to attach as context (default: 200, hard cap 10000).")], &["pattern", "scope"]),
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
        let pattern = match args.get("pattern") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `pattern` is not a string: {:?}", v)),
            None => {
                return Err("Missing argument `pattern` in the `search_pattern()` call.".to_string())
            }
        };

        let scope = match args.get("scope") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `scope` is not a string: {:?}", v)),
            None => {
                return Err("Missing argument `scope` in the search_pattern() call.".to_string())
            }
        };

        let context_lines = parse_usize_arg(args, "context_lines")?
            .unwrap_or(DEFAULT_CONTEXT_LINES)
            .min(CAP_CONTEXT_LINES);
        let max_files = parse_usize_arg(args, "max_files")?
            .unwrap_or(DEFAULT_MAX_FILES)
            .clamp(1, CAP_MAX_FILES);
        let max_matches_per_file = parse_usize_arg(args, "max_matches_per_file")?
            .unwrap_or(DEFAULT_MAX_MATCHES_PER_FILE)
            .clamp(1, CAP_MAX_MATCHES_PER_FILE);
        let max_total_matches = parse_usize_arg(args, "max_total_matches")?
            .unwrap_or(DEFAULT_MAX_TOTAL_MATCHES)
            .clamp(1, CAP_MAX_TOTAL_MATCHES);

        let (gcx, execution_scope, abort_flag) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
                cgcx.abort_flag.clone(),
            )
        };
        let roots = workspace_roots(&gcx, execution_scope.as_ref());

        let scoped_files = resolve_scope_with_execution_scope_limited_for_model_context(
            gcx.clone(),
            execution_scope.as_ref(),
            &scope,
            MAX_SEARCH_CANDIDATE_ENTRIES,
            Some(&abort_flag),
        )
        .await?;
        let read_context = Arc::new(prepare_file_read_context(gcx.clone()).await);
        let files_in_scope = validate_scope_files(scoped_files.files, &scope)?
            .into_iter()
            .filter(|path| {
                check_file_privacy_for_read_preparation_with_context(
                    &read_context,
                    &PathBuf::from(path),
                )
                .is_ok()
            })
            .collect::<Vec<_>>();

        let mut all_content = format_scope_notices(&scoped_files.notices);
        let mut all_search_results = Vec::new();

        let regex = match Regex::new(&pattern) {
            Ok(r) => r,
            Err(e) => return Err(format!("⚠️ Invalid regex '{}': {}. 💡 Use (?i) for case-insensitive, escape special chars with \\", pattern, e)),
        };
        drop(regex);

        let search_outcome = search_files_with_regex(
            gcx.clone(),
            &pattern,
            &files_in_scope,
            context_lines,
            max_files,
            max_matches_per_file,
            max_total_matches,
            abort_flag.clone(),
            read_context,
        )
        .await?;
        let RegexSearchOutcome {
            matches: search_results,
            files_scanned,
            files_total,
            stopped_early,
        } = search_outcome;
        let mut references = NativeReferences::new();
        references.add_query(
            &pattern,
            search_results.len(),
            if search_results.is_empty() {
                "no_matches"
            } else {
                "matched"
            },
            "search_pattern",
        );
        if search_results.is_empty() {
            all_content.push_str(&format!(
                "\nNo matches found for pattern '{}' in {}. 💡 Try broader scope ('workspace'), a simpler pattern, or use (?i) for case-insensitive search.\n",
                pattern, scope
            ));
        } else {
            all_content.push_str("\nText matches inside files:\n");
            let mut file_results: HashMap<String, Vec<&RegexMatch>> = HashMap::new();
            search_results.iter().for_each(|rec| {
                file_results
                    .entry(rec.file_name.clone())
                    .or_insert(vec![])
                    .push(rec)
            });
            let pattern_content = smart_compress_results(
                &search_results,
                &file_results,
                gcx.clone(),
                &pattern,
                max_matches_per_file,
                4 * 1024,
            )
            .await;
            all_content.push_str(&pattern_content);

            // Attach context: per-match windows (will be merged/deduped in postprocessing).
            // Hard-capped to avoid tool runs that accidentally explode context.
            let mut files_emitted = HashSet::<String>::new();
            let mut total_emitted: usize = 0;
            for (file, mut matches) in file_results
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .sorted_by(|a, b| a.0.cmp(&b.0))
            {
                if files_emitted.len() >= max_files || total_emitted >= max_total_matches {
                    break;
                }
                matches.sort_by_key(|m| m.match_line);
                let per_file = matches.len().min(max_matches_per_file);
                for m in matches.into_iter().take(per_file) {
                    if total_emitted >= max_total_matches {
                        break;
                    }
                    all_search_results.push(ContextFile {
                        file_name: file.clone(),
                        file_content: String::new(),
                        line1: m.context_start,
                        line2: m.context_end_inclusive,
                        file_rev: None,
                        symbols: vec![],
                        gradient_type: 5,
                        usefulness: 100.0,
                        skip_pp: true,
                    });
                    references.add_path(
                        PathBuf::from(&file).as_path(),
                        &roots,
                        m.context_start,
                        m.context_end_inclusive,
                        "match",
                        Some(1.0),
                        "search_pattern",
                    );
                    total_emitted += 1;
                    files_emitted.insert(file.clone());
                }
            }

            if search_results.len() > total_emitted {
                references.mark_truncated();
                all_content.push_str(&format!(
                    "\n⚠️ Attached {} match windows (of {}). Narrow scope/pattern or raise max_total_matches/max_files if needed.\n",
                    total_emitted,
                    search_results.len()
                ));
            }
        }

        if stopped_early {
            references.mark_truncated();
            all_content.push_str(&format!(
                "\n⚠️ Search stopped after scanning {} of {} files because the requested match limits were reached. Narrow the scope or raise max_files/max_total_matches for more results.\n",
                files_scanned, files_total
            ));
        }

        // Append related memories (short form) based on the matched file paths.
        let related_section = {
            let idx_arc = { gcx.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let matched_files: Vec<String> = all_search_results
                .iter()
                .map(|cf| cf.file_name.clone())
                .unique()
                .collect();
            let mut cards = idx_guard.related_for_files(&matched_files, 8);
            if cards.is_empty() {
                cards = idx_guard.related_for_related_files(&matched_files, 8);
            }
            format_related_memories_section(&cards, None)
        };

        let scanned_paths = files_in_scope.iter().map(PathBuf::from).collect::<Vec<_>>();
        let mut results = vec_context_file_to_context_tools(all_search_results);
        let mut tool_message = ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(format!("{}{}", all_content, related_section)),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            output_filter: Some(OutputFilter::no_limits()), // Already compressed internally
            ..Default::default()
        };
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let records = crate::privacy::records::declared_file_records(&gcx, scanned_paths)?;
        crate::privacy::records::merge_records(&mut tool_message, records);
        references.attach(&mut tool_message);
        results.push(ContextEnum::ChatMessage(tool_message));

        Ok((false, results))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;

    async fn make_gcx() -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts: u64::MAX / 2,
        });
        gcx
    }

    async fn make_ccx(gcx: Arc<GlobalContext>) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                crate::app_state::AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    #[tokio::test]
    async fn tool_execute_returns_successful_message_for_zero_matches() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-no-match-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("search.rs");
        fs::write(&file, "fn present() {}\n").unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![file];
        let ccx = make_ccx(gcx).await;
        let mut tool = ToolRegexSearch {
            config_path: String::new(),
        };
        let args = HashMap::from_iter([
            ("pattern".to_string(), Value::String("absent".to_string())),
            ("scope".to_string(), Value::String("workspace".to_string())),
        ]);

        let (corrections, results) = tool
            .tool_execute(ccx, &"search-call".to_string(), &args)
            .await
            .unwrap();

        assert!(!corrections);
        assert_eq!(results.len(), 1);
        let ContextEnum::ChatMessage(message) = &results[0] else {
            panic!("zero-match result must be a tool message");
        };
        let ChatContent::SimpleText(text) = &message.content else {
            panic!("zero-match result must contain text");
        };
        assert!(text.contains("No matches found for pattern 'absent' in workspace."));
        assert!(text.contains("Try broader scope"));
        assert!(message.output_filter.is_some());
        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(
            enrichment.references[0].kind,
            refact_chat_api::ToolEnrichmentKind::Query
        );
        assert_eq!(enrichment.references[0].target, "absent");
        assert_eq!(
            enrichment.references[0].status.as_deref(),
            Some("no_matches")
        );
        assert_eq!(message.content.content_text_only(), *text);
    }

    #[tokio::test]
    async fn tool_execute_keeps_invalid_regex_and_path_as_errors() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-invalid-input-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("search.rs");
        fs::write(&file, "fn present() {}\n").unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![file];
        let mut tool = ToolRegexSearch {
            config_path: String::new(),
        };
        let invalid_regex_args = HashMap::from_iter([
            ("pattern".to_string(), Value::String("[".to_string())),
            ("scope".to_string(), Value::String("workspace".to_string())),
        ]);

        let regex_error = tool
            .tool_execute(
                make_ccx(gcx.clone()).await,
                &"search-call".to_string(),
                &invalid_regex_args,
            )
            .await
            .unwrap_err();
        assert!(regex_error.contains("Invalid regex"), "{regex_error}");

        let missing_path = temp.path().join("missing.rs").to_string_lossy().to_string();
        let invalid_path_args = HashMap::from_iter([
            ("pattern".to_string(), Value::String("anything".to_string())),
            ("scope".to_string(), Value::String(missing_path)),
        ]);
        assert!(
            tool.tool_execute(
                make_ccx(gcx).await,
                &"search-call".to_string(),
                &invalid_path_args,
            )
            .await
            .is_err(),
            "invalid path must remain a tool error"
        );
    }

    #[tokio::test]
    async fn search_single_file_stops_at_max_matches_per_file() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-early-stop-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("many_matches.rs");
        let content = (0..100)
            .map(|i| format!("needle line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file, content).unwrap();

        let regex = Regex::new("needle").unwrap();
        let abort = AtomicBool::new(false);
        let read_context = prepare_file_read_context(gcx.clone()).await;
        let matches = search_single_file(
            gcx,
            file.to_string_lossy().to_string(),
            &regex,
            0,
            7,
            &abort,
            &read_context,
        )
        .await;

        assert_eq!(
            matches.len(),
            7,
            "search_single_file must stop scanning at max_matches_per_file"
        );
    }

    #[tokio::test]
    async fn search_single_file_respects_preset_abort_flag() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-abort-file-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("has_matches.rs");
        fs::write(&file, "needle one\nneedle two\n").unwrap();

        let regex = Regex::new("needle").unwrap();
        let abort = AtomicBool::new(true);
        let read_context = prepare_file_read_context(gcx.clone()).await;
        let matches = search_single_file(
            gcx,
            file.to_string_lossy().to_string(),
            &regex,
            0,
            25,
            &abort,
            &read_context,
        )
        .await;

        assert!(
            matches.is_empty(),
            "aborted search must not collect matches"
        );
    }

    #[tokio::test]
    async fn search_files_with_regex_aborts_promptly() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-abort-batch-")
            .tempdir()
            .unwrap();
        let mut files = Vec::new();
        for i in 0..8 {
            let file = temp.path().join(format!("file_{i}.rs"));
            fs::write(&file, format!("needle in file {i}\n")).unwrap();
            files.push(file.to_string_lossy().to_string());
        }

        let abort = Arc::new(AtomicBool::new(true));
        let read_context = Arc::new(prepare_file_read_context(gcx.clone()).await);
        let result =
            search_files_with_regex(gcx, "needle", &files, 0, 50, 25, 200, abort, read_context)
                .await;

        assert!(result.is_err(), "aborted batch search must return an error");
        assert!(
            result.unwrap_err().contains("aborted"),
            "error should mention the abort"
        );
    }

    #[tokio::test]
    async fn search_files_with_regex_finds_matches_when_not_aborted() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-happy-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("normal.rs");
        fs::write(&file, "alpha\nneedle here\nbeta\n").unwrap();
        let files = vec![file.to_string_lossy().to_string()];

        let abort = Arc::new(AtomicBool::new(false));
        let read_context = Arc::new(prepare_file_read_context(gcx.clone()).await);
        let outcome =
            search_files_with_regex(gcx, "needle", &files, 1, 50, 25, 200, abort, read_context)
                .await
                .unwrap();

        assert_eq!(outcome.matches.len(), 1);
        assert_eq!(outcome.matches[0].match_line, 2);
        assert!(!outcome.stopped_early);
    }

    #[tokio::test]
    async fn search_files_with_regex_stops_at_global_limits() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-global-limit-")
            .tempdir()
            .unwrap();
        let mut files = Vec::new();
        for index in 0..64 {
            let name = format!("file_{index:03}.rs");
            let file = temp.path().join(name);
            fs::write(&file, "needle one\nneedle two\n").unwrap();
            files.push(file.to_string_lossy().to_string());
        }

        let read_context = Arc::new(prepare_file_read_context(gcx.clone()).await);
        let outcome = search_files_with_regex(
            gcx,
            "needle",
            &files,
            0,
            1,
            25,
            200,
            Arc::new(AtomicBool::new(false)),
            read_context,
        )
        .await
        .unwrap();

        assert_eq!(outcome.files_scanned, MAX_CONCURRENT_FILE_READS);
        assert_eq!(outcome.matches.len(), 2);
        assert!(outcome
            .matches
            .iter()
            .all(|m| m.file_name.ends_with("file_000.rs")));
        assert!(outcome.stopped_early);
    }

    #[tokio::test]
    async fn search_single_file_skips_oversized_files() {
        let gcx = make_gcx().await;
        let temp = tempfile::Builder::new()
            .prefix("refact-regex-oversized-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("oversized.rs");
        fs::write(&file, vec![b'a'; MAX_SEARCH_FILE_BYTES as usize + 1]).unwrap();
        let read_context = prepare_file_read_context(gcx.clone()).await;

        let matches = search_single_file(
            gcx,
            file.to_string_lossy().to_string(),
            &Regex::new("a").unwrap(),
            0,
            25,
            &AtomicBool::new(false),
            &read_context,
        )
        .await;

        assert!(matches.is_empty());
    }

    #[test]
    fn parse_usize_arg_rejects_invalid_values() {
        let invalid_string =
            HashMap::from_iter([("max_files".to_string(), Value::String("many".to_string()))]);
        let negative = HashMap::from_iter([("max_files".to_string(), Value::from(-1))]);

        assert!(parse_usize_arg(&invalid_string, "max_files").is_err());
        assert!(parse_usize_arg(&negative, "max_files").is_err());
    }
}
