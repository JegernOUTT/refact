use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use itertools::Itertools;
use regex::Regex;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use tracing::info;

use crate::at_commands::at_commands::{vec_context_file_to_context_tools, AtCommandsContext};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::files_correction::shortify_paths;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::tools::scope_utils::{
    format_scope_notices, resolve_scope_with_execution_scope, validate_scope_files,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::knowledge_index::format_related_memories_section;

pub struct ToolRegexSearch {
    pub config_path: String,
}

const DEFAULT_CONTEXT_LINES: usize = 5;
const DEFAULT_MAX_FILES: usize = 50;
const DEFAULT_MAX_MATCHES_PER_FILE: usize = 25;
const DEFAULT_MAX_TOTAL_MATCHES: usize = 200;

#[derive(Clone, Debug)]
struct RegexMatch {
    file_name: String,
    match_line: usize,    // 1-based
    context_start: usize, // 1-based
    context_end_inclusive: usize,
    preview: String,
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
    gcx: Arc<ARwLock<GlobalContext>>,
    file_path: String,
    regex: &Regex,
    context_lines: usize,
) -> Vec<RegexMatch> {
    let file_content =
        match get_file_text_from_memory_or_disk(gcx.clone(), &PathBuf::from(&file_path)).await {
            Ok(content) => content.to_string(),
            Err(_) => return Vec::new(),
        };

    let lines: Vec<&str> = file_content.lines().collect();
    let mut file_results = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
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
        }
    }

    file_results
}

/// Maximum concurrent file reads to avoid overwhelming I/O
const MAX_CONCURRENT_FILE_READS: usize = 32;

async fn search_files_with_regex(
    gcx: Arc<ARwLock<GlobalContext>>,
    pattern: &str,
    files_to_search: &[String],
    context_lines: usize,
) -> Result<Vec<RegexMatch>, String> {
    let regex = Regex::new(pattern).map_err(|e| format!("Invalid regex pattern: {}", e))?;
    let regex_arc = Arc::new(regex);

    // Use bounded concurrency to avoid overwhelming I/O with thousands of files
    let results: Vec<Vec<RegexMatch>> =
        stream::iter(files_to_search.iter().cloned())
            .map(|file_path| {
                let gcx_clone = gcx.clone();
                let regex_clone = regex_arc.clone();
                let context_lines = context_lines;
                async move {
                    search_single_file(gcx_clone, file_path, &regex_clone, context_lines).await
                }
            })
            .buffer_unordered(MAX_CONCURRENT_FILE_READS)
            .collect()
            .await;

    let mut flat_results: Vec<RegexMatch> = results.into_iter().flatten().collect();
    flat_results.sort_by(|a, b| {
        a.file_name
            .cmp(&b.file_name)
            .then(a.match_line.cmp(&b.match_line))
    });
    Ok(flat_results)
}

fn path_depth(path: &str) -> usize {
    path.chars().filter(|&c| c == '/' || c == '\\').count()
}

async fn smart_compress_results(
    search_results: &[RegexMatch],
    file_results: &HashMap<String, Vec<&RegexMatch>>,
    gcx: Arc<ARwLock<GlobalContext>>,
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
        Some(Value::Number(n)) => Ok(Some(n.as_u64().unwrap_or(0) as usize)),
        Some(Value::String(s)) => Ok(Some(s.parse::<usize>().unwrap_or(0))),
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
            description: "Search for files and folders whose names or paths match the given regular expression pattern, and also search for text matches inside files using the same pattern. Reports both path matches and text matches in separate sections.".to_string(),
            input_schema: json_schema_from_params(&[("pattern", "string", "The pattern is used to search for matching file/folder names/paths, and also for matching text inside files. Use (?i) at the start for case-insensitive search."), ("scope", "string", "'workspace' to search all files in workspace, 'dir/subdir/' to search in files within a directory, 'dir/file.ext' to search in a single file."), ("context_lines", "integer", "Lines of context before/after each match (default: 5)."), ("max_files", "integer", "Max files to attach as context (default: 50)."), ("max_matches_per_file", "integer", "Max matches per file to include (default: 25)."), ("max_total_matches", "integer", "Max total matches to attach as context (default: 200).")], &["pattern", "scope"]),
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

        let context_lines =
            parse_usize_arg(args, "context_lines")?.unwrap_or(DEFAULT_CONTEXT_LINES);
        let max_files = parse_usize_arg(args, "max_files")?.unwrap_or(DEFAULT_MAX_FILES);
        let max_matches_per_file =
            parse_usize_arg(args, "max_matches_per_file")?.unwrap_or(DEFAULT_MAX_MATCHES_PER_FILE);
        let max_total_matches =
            parse_usize_arg(args, "max_total_matches")?.unwrap_or(DEFAULT_MAX_TOTAL_MATCHES);

        let (gcx, execution_scope) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
            )
        };

        let scoped_files =
            resolve_scope_with_execution_scope(gcx.clone(), execution_scope.as_ref(), &scope)
                .await?;
        let files_in_scope = validate_scope_files(scoped_files.files, &scope)?;

        let mut all_content = format_scope_notices(&scoped_files.notices);
        let mut all_search_results = Vec::new();

        // 1. Path matches
        let regex = match Regex::new(&pattern) {
            Ok(r) => r,
            Err(e) => return Err(format!("⚠️ Invalid regex '{}': {}. 💡 Use (?i) for case-insensitive, escape special chars with \\", pattern, e)),
        };
        let mut path_matches: Vec<String> = files_in_scope
            .iter()
            .filter(|path| regex.is_match(path))
            .cloned()
            .collect();
        path_matches.sort();

        const MAX_PATH_MATCHES_TO_LIST: usize = 25;
        const MAX_PATH_MATCHES_TO_ATTACH: usize = 10;
        const PATH_MATCH_PREVIEW_LINES: usize = 30;

        all_content.push_str("Path matches (file/folder names):\n");
        if path_matches.is_empty() {
            all_content.push_str("  No files or folders matched by name.\n");
        } else {
            for path in path_matches.iter().take(MAX_PATH_MATCHES_TO_LIST) {
                all_content.push_str(&format!("  {}\n", path));
            }
            if path_matches.len() > MAX_PATH_MATCHES_TO_LIST {
                all_content.push_str(&format!(
                    "  ... and {} more path matches\n",
                    path_matches.len() - MAX_PATH_MATCHES_TO_LIST
                ));
            }
        }

        for path in path_matches.iter().take(MAX_PATH_MATCHES_TO_ATTACH) {
            let cf = ContextFile {
                file_name: path.clone(),
                file_content: "".to_string(),
                line1: 1,
                line2: PATH_MATCH_PREVIEW_LINES,
                file_rev: None,
                symbols: vec![],
                gradient_type: 4,
                usefulness: 80.0,
                skip_pp: true,
            };
            all_search_results.push(cf);
        }

        let search_results =
            search_files_with_regex(gcx.clone(), &pattern, &files_in_scope, context_lines).await?;
        all_content.push_str("\nText matches inside files:\n");
        if search_results.is_empty() {
            all_content.push_str("  No text matches found in any file.\n");
        } else {
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
                    total_emitted += 1;
                    files_emitted.insert(file.clone());
                }
            }

            if search_results.len() > total_emitted {
                all_content.push_str(&format!(
                    "\n⚠️ Attached {} match windows (of {}). Narrow scope/pattern or raise max_total_matches/max_files if needed.\n",
                    total_emitted,
                    search_results.len()
                ));
            }
        }

        if all_search_results.is_empty() {
            return Err("⚠️ No matches found for pattern or path. 💡 Try broader scope ('workspace'), simpler pattern, or use (?i) for case-insensitive".to_string());
        }

        // Append related memories (short form) based on the matched file paths.
        let related_section = {
            let idx_arc = { gcx.read().await.knowledge_index.clone() };
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

        let mut results = vec_context_file_to_context_tools(all_search_results);
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(format!("{}{}", all_content, related_section)),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            output_filter: Some(OutputFilter::no_limits()), // Already compressed internally
            ..Default::default()
        }));

        Ok((false, results))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
