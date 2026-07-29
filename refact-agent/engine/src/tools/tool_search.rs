use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::info;

use async_trait::async_trait;
use itertools::Itertools;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::{vec_context_file_to_context_tools, AtCommandsContext};
use crate::at_commands::at_search::execute_at_search;
use crate::files_in_workspace::{
    get_file_text_from_memory_or_disk_with_context, prepare_file_read_context, FileReadContext,
};
use crate::global_context::GlobalContext;
use crate::tools::scope_utils::{
    create_scope_filter_with_execution_scope, format_scope_notices, is_worktree_only_path,
    remap_context_files_for_execution_scope, resolve_scope_with_execution_scope_limited,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::knowledge_index::format_related_memories_section;
use crate::worktrees::scope::ExecutionScope;

pub struct ToolSearch {
    pub config_path: String,
}

const DEFAULT_CONTEXT_LINES: usize = 0;
const DEFAULT_MAX_FILES: usize = 50;
const DEFAULT_MAX_RECS_PER_FILE: usize = 10;
const DEFAULT_MAX_TOTAL_RECS: usize = 200;

const CAP_CONTEXT_LINES: usize = 100;
const CAP_MAX_FILES: usize = 1000;
const CAP_MAX_RECS_PER_FILE: usize = 1000;
const CAP_MAX_TOTAL_RECS: usize = 10000;

const FALLBACK_MAX_CANDIDATE_FILES: usize = 5000;
const MAX_SEARCH_FILE_BYTES: usize = 4 * 1024 * 1024;

const ABORTED_ERROR: &str =
    "⚠️ semantic_search worktree fallback aborted before completion (cancelled by caller).";

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

fn format_preview(lines: &[&str], start_idx: usize, end_idx_exclusive: usize) -> String {
    lines[start_idx..end_idx_exclusive]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>6} | {}", start_idx + i + 1, line))
        .join("\n")
}

fn query_terms(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    let mut terms = Vec::new();
    let trimmed = lower.trim();
    if trimmed.len() >= 2 {
        terms.push(trimmed.to_string());
    }
    terms.extend(
        lower
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|term| term.len() >= 2)
            .map(|term| term.to_string()),
    );
    terms.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    terms.dedup();
    terms
}

fn direct_fallback_window(
    file_name: &str,
    file_content: &str,
    terms: &[String],
    context_lines: usize,
) -> Option<(usize, usize, f32)> {
    let lines: Vec<&str> = file_content.lines().collect();
    let line_count = lines.len().max(1);
    for (line_idx, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        let matches = terms
            .iter()
            .filter(|term| lower.contains(term.as_str()))
            .count();
        if matches > 0 {
            let start = line_idx.saturating_sub(context_lines) + 1;
            let end = (line_idx + context_lines + 1).min(line_count).max(start);
            return Some((start, end, 55.0 + matches as f32));
        }
    }

    let lower_file_name = file_name.to_lowercase();
    if terms
        .iter()
        .any(|term| lower_file_name.contains(term.as_str()))
    {
        return Some((1, (context_lines * 2 + 1).min(line_count).max(1), 45.0));
    }

    None
}

fn context_file_key(context_file: &ContextFile) -> String {
    format!(
        "{}:{}:{}:{:?}",
        context_file.file_name, context_file.line1, context_file.line2, context_file.symbols
    )
}

fn append_unseen_context_files(
    context_files: &mut Vec<ContextFile>,
    additions: Vec<ContextFile>,
) -> usize {
    let mut seen: HashSet<String> = context_files.iter().map(context_file_key).collect();
    let mut added = 0;
    for context_file in additions {
        let key = context_file_key(&context_file);
        if seen.insert(key) {
            context_files.push(context_file);
            added += 1;
        }
    }
    added
}

#[derive(Debug)]
struct FallbackOutcome {
    context_files: Vec<ContextFile>,
    truncated: bool,
}

async fn direct_worktree_fallback_search(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    read_context: &FileReadContext,
    scope: &str,
    query: &str,
    context_lines: usize,
    limit: usize,
    abort_flag: &AtomicBool,
) -> Result<FallbackOutcome, String> {
    let empty = FallbackOutcome {
        context_files: Vec::new(),
        truncated: false,
    };
    let Some(execution_scope) = execution_scope else {
        return Ok(empty);
    };
    if !execution_scope.is_enforced() || limit == 0 {
        return Ok(empty);
    }

    if abort_flag.load(Ordering::Relaxed) {
        return Err(ABORTED_ERROR.to_string());
    }

    let terms = query_terms(query);
    if terms.is_empty() {
        return Ok(empty);
    }

    let scoped_files = resolve_scope_with_execution_scope_limited(
        gcx.clone(),
        Some(execution_scope),
        scope,
        FALLBACK_MAX_CANDIDATE_FILES,
        Some(abort_flag),
    )
    .await?;
    let mut files = scoped_files.files;
    files.sort();

    let mut context_files = Vec::new();
    let mut candidates_inspected: usize = 0;
    let mut truncated = !scoped_files.notices.is_empty();

    for (idx, file) in files.iter().enumerate() {
        if (idx & 0x3F) == 0 && abort_flag.load(Ordering::Relaxed) {
            return Err(ABORTED_ERROR.to_string());
        }
        if context_files.len() >= limit {
            break;
        }
        let file_path = PathBuf::from(file);
        if !is_worktree_only_path(execution_scope, &file_path) {
            continue;
        }
        if candidates_inspected >= FALLBACK_MAX_CANDIDATE_FILES {
            truncated = true;
            break;
        }
        candidates_inspected += 1;
        let file_content = match get_file_text_from_memory_or_disk_with_context(
            gcx.clone(),
            &file_path,
            read_context,
            Some(MAX_SEARCH_FILE_BYTES),
        )
        .await
        {
            Ok(content) => content,
            Err(_) => continue,
        };
        if let Some((line1, line2, usefulness)) =
            direct_fallback_window(file, &file_content, &terms, context_lines)
        {
            context_files.push(ContextFile {
                file_name: file.clone(),
                file_content: String::new(),
                line1,
                line2,
                file_rev: None,
                symbols: vec![],
                gradient_type: 4,
                usefulness,
                skip_pp: false,
            });
        }
    }

    Ok(FallbackOutcome {
        context_files,
        truncated,
    })
}

async fn execute_att_search(
    ccx: Arc<AMutex<AtCommandsContext>>,
    query: &String,
    scope: &String,
    context_lines: usize,
    fallback_limit: usize,
) -> Result<(Vec<ContextFile>, Vec<String>), String> {
    let (gcx, execution_scope, abort_flag) = {
        let cgcx = ccx.lock().await;
        (
            cgcx.app.gcx.clone(),
            cgcx.execution_scope.clone(),
            cgcx.abort_flag.clone(),
        )
    };

    let scoped_filter =
        create_scope_filter_with_execution_scope(gcx.clone(), execution_scope.as_ref(), scope)
            .await?;

    info!("att-search: filter: {:?}", scoped_filter.filter);
    let context_files = execute_at_search(ccx.clone(), &query, scoped_filter.filter).await?;
    let (mut context_files, remap_notices) = remap_context_files_for_execution_scope(
        gcx.clone(),
        execution_scope.as_ref(),
        context_files,
    )
    .await?;
    let mut notices = scoped_filter.notices;
    notices.extend(remap_notices);

    let read_context = prepare_file_read_context(gcx.clone()).await;
    let fallback_outcome = direct_worktree_fallback_search(
        gcx.clone(),
        execution_scope.as_ref(),
        &read_context,
        scope,
        query,
        context_lines,
        fallback_limit,
        &abort_flag,
    )
    .await?;
    let FallbackOutcome {
        context_files: fallback_context_files,
        truncated,
    } = fallback_outcome;
    let added = append_unseen_context_files(&mut context_files, fallback_context_files);
    if added > 0 {
        notices.push(format!(
            "⚠️ Direct worktree filesystem fallback added {} worktree-only result(s) not present in the source index.",
            added
        ));
    }
    if truncated {
        notices.push(format!(
            "⚠️ Direct worktree filesystem fallback inspected only the first {} worktree-only candidate file(s); results may be incomplete. Narrow the scope for a complete fallback.",
            FALLBACK_MAX_CANDIDATE_FILES
        ));
    }
    Ok((context_files, notices))
}

#[async_trait]
impl Tool for ToolSearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "search_semantic".to_string(),
            display_name: "Search".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Find semantically similar pieces of code or text using vector database (semantic search)".to_string(),
            input_schema: json_schema_from_params(&[("queries", "string", "Comma-separated list of queries. Each query can be a single line, paragraph or code sample to search for semantically similar content."), ("scope", "string", "'workspace' to search all files in workspace, 'dir/subdir/' to search in files within a directory, 'dir/file.ext' to search in a single file."), ("context_lines", "integer", "If >0, include a small line-numbered preview around each hit in the tool text output (default: 0)."), ("max_files", "integer", "Max distinct files to attach as context (default: 50)."), ("max_recs_per_file", "integer", "Max vecdb records per file to attach as context (default: 10)."), ("max_total_recs", "integer", "Max total vecdb records to attach as context (default: 200).")], &["queries", "scope"]),
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
        let query_str = match args.get("queries") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `queries` is not a string: {:?}", v)),
            None => {
                return Err("Missing argument `queries` in the search_semantic() call.".to_string())
            }
        };
        let scope = match args.get("scope") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `scope` is not a string: {:?}", v)),
            None => {
                return Err("Missing argument `scope` in the search_semantic() call.".to_string())
            }
        };

        let context_lines = parse_usize_arg(args, "context_lines")?
            .unwrap_or(DEFAULT_CONTEXT_LINES)
            .min(CAP_CONTEXT_LINES);
        let max_files = parse_usize_arg(args, "max_files")?
            .unwrap_or(DEFAULT_MAX_FILES)
            .clamp(1, CAP_MAX_FILES);
        let max_recs_per_file = parse_usize_arg(args, "max_recs_per_file")?
            .unwrap_or(DEFAULT_MAX_RECS_PER_FILE)
            .clamp(1, CAP_MAX_RECS_PER_FILE);
        let max_total_recs = parse_usize_arg(args, "max_total_recs")?
            .unwrap_or(DEFAULT_MAX_TOTAL_RECS)
            .clamp(1, CAP_MAX_TOTAL_RECS);

        let queries: Vec<String> = query_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if queries.is_empty() {
            return Err("No valid queries provided".to_string());
        }

        let mut all_context_files = Vec::new();
        let mut all_content = String::new();

        for (i, query) in queries.iter().enumerate() {
            if i > 0 {
                all_content.push_str("\n\n");
            }

            all_content.push_str(&format!("Results for query: \"{}\"\n", query));

            let (vector_of_context_file, scope_notices) =
                execute_att_search(ccx.clone(), query, &scope, context_lines, max_total_recs)
                    .await?;
            all_content.push_str(&format_scope_notices(&scope_notices));
            info!(
                "att-search: vector_of_context_file={:?}",
                vector_of_context_file
            );

            if vector_of_context_file.is_empty() {
                all_content.push_str("⚠️ No results for this query. 💡 Try different keywords or broaden scope to 'workspace'\n");
                continue;
            }

            all_content.push_str("Records found:\n\n");
            let mut file_results_to_reqs: HashMap<String, Vec<&ContextFile>> = HashMap::new();
            vector_of_context_file.iter().for_each(|rec| {
                file_results_to_reqs
                    .entry(rec.file_name.clone())
                    .or_insert(vec![])
                    .push(rec)
            });

            // Optional: include small previews in the tool text output.
            // This is intentionally best-effort and bounded.
            if context_lines > 0 {
                let gcx = ccx.lock().await.app.gcx.clone();
                let mut files_sorted: Vec<String> = file_results_to_reqs.keys().cloned().collect();
                files_sorted.sort();
                for file in files_sorted.iter().take(max_files) {
                    if let Some(recs) = file_results_to_reqs.get(file) {
                        let mut recs_sorted = recs.clone();
                        recs_sorted.sort_by(|a, b| a.line1.cmp(&b.line1));
                        let text =
                            match crate::files_in_workspace::get_file_text_from_memory_or_disk(
                                gcx.clone(),
                                &std::path::PathBuf::from(file),
                            )
                            .await
                            {
                                Ok(t) => t,
                                Err(_) => continue,
                            };
                        let lines: Vec<&str> = text.lines().collect();
                        if lines.is_empty() {
                            continue;
                        }
                        all_content.push_str(&format!("\n{}:\n", file));
                        for rec in recs_sorted.into_iter().take(max_recs_per_file) {
                            let start_line = rec.line1.max(1);
                            let end_line = rec.line2.max(start_line);
                            let center = ((start_line + end_line) / 2).max(1);
                            let start_idx = center.saturating_sub(1 + context_lines);
                            let end_idx_excl = (center + context_lines).min(lines.len());
                            let preview = format_preview(&lines, start_idx, end_idx_excl);
                            all_content.push_str(&format!(
                                "  lines {}-{} score {:.1}%\n{}\n\n",
                                rec.line1,
                                rec.line2,
                                rec.usefulness,
                                preview.lines().map(|l| format!("    {}", l)).join("\n")
                            ));
                        }
                    }
                }
            }

            let mut used_files: HashSet<String> = HashSet::new();
            let mut total_emitted: usize = 0;
            for rec in vector_of_context_file
                .iter()
                .sorted_by(|rec1, rec2| rec2.usefulness.total_cmp(&rec1.usefulness))
            {
                if used_files.len() >= max_files || total_emitted >= max_total_recs {
                    break;
                }
                if !used_files.contains(&rec.file_name) {
                    all_content.push_str(&format!("{}:\n", rec.file_name.clone()));
                    let file_recs = file_results_to_reqs.get(&rec.file_name).unwrap();
                    let mut per_file_emitted: usize = 0;
                    for file_req in file_recs
                        .iter()
                        .sorted_by(|rec1, rec2| rec2.usefulness.total_cmp(&rec1.usefulness))
                    {
                        if total_emitted >= max_total_recs || per_file_emitted >= max_recs_per_file
                        {
                            break;
                        }
                        all_content.push_str(&format!(
                            "    lines {}-{} score {:.1}%\n",
                            file_req.line1, file_req.line2, file_req.usefulness
                        ));
                        all_context_files.push((*file_req).clone());
                        total_emitted += 1;
                        per_file_emitted += 1;
                    }
                    used_files.insert(rec.file_name.clone());
                }
            }

            if vector_of_context_file.len() > total_emitted {
                all_content.push_str(&format!(
                    "⚠️ Attached {} records (of {}). Narrow scope/query or raise max_total_recs/max_files if needed.\n",
                    total_emitted,
                    vector_of_context_file.len()
                ));
            }
        }

        if all_context_files.is_empty() {
            return Err("⚠️ All searches produced no results. 💡 Try different keywords, broaden scope to 'workspace', or use search_pattern() for regex search".to_string());
        }

        // Append related memories (short form) based on involved file paths.
        // This does not require VecDB and is <50ms (in-memory index).
        let related_section = {
            let gcx = ccx.lock().await.app.gcx.clone();
            let idx_arc = { gcx.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let mut files: Vec<String> = all_context_files
                .iter()
                .map(|cf| cf.file_name.clone())
                .unique()
                .collect();
            files.sort();
            let mut cards = idx_guard.related_for_files(&files, 8);
            if cards.is_empty() {
                cards = idx_guard.related_for_related_files(&files, 8);
            }
            format_related_memories_section(&cards, None)
        };

        let mut results = vec_context_file_to_context_tools(all_context_files);
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(format!("{}{}", all_content, related_section)),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));
        Ok((false, results))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::{FilePrivacySettings, PrivacySettings};
    use crate::worktrees::types::WorktreeMeta;
    use std::fs;
    use std::sync::atomic::AtomicBool;

    async fn make_gcx() -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts: u64::MAX / 2,
        });
        gcx
    }

    struct FallbackFixture {
        _temp: tempfile::TempDir,
        worktree: WorktreeMeta,
        root: PathBuf,
    }

    fn make_fallback_fixture() -> FallbackFixture {
        let temp = tempfile::Builder::new()
            .prefix("refact-semantic-fallback-")
            .tempdir()
            .unwrap();
        let root = temp
            .path()
            .join(".cache")
            .join("refact")
            .join("worktrees")
            .join("wt")
            .join("engine");
        let source = temp.path().join("source");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(source.join("src")).unwrap();
        let root = dunce::simplified(&fs::canonicalize(&root).unwrap()).to_path_buf();
        let source = dunce::simplified(&fs::canonicalize(&source).unwrap()).to_path_buf();
        let worktree = WorktreeMeta {
            id: "wt-semantic-fallback".to_string(),
            kind: "chat".to_string(),
            root: root.clone(),
            source_workspace_root: source.clone(),
            repo_root: source.clone(),
            branch: Some("feature".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        };
        FallbackFixture {
            _temp: temp,
            worktree,
            root,
        }
    }

    #[test]
    fn parse_usize_arg_rejects_invalid_values() {
        let invalid_string =
            HashMap::from_iter([("max_files".to_string(), Value::String("many".to_string()))]);
        let negative = HashMap::from_iter([("max_files".to_string(), Value::from(-1))]);
        let fractional = HashMap::from_iter([("context_lines".to_string(), Value::from(1.5))]);

        assert!(parse_usize_arg(&invalid_string, "max_files").is_err());
        assert!(parse_usize_arg(&negative, "max_files").is_err());
        assert!(parse_usize_arg(&fractional, "context_lines").is_err());
        assert_eq!(parse_usize_arg(&HashMap::new(), "max_files").unwrap(), None);
        assert_eq!(
            parse_usize_arg(
                &HashMap::from_iter([("max_files".to_string(), Value::from(7u64))]),
                "max_files"
            )
            .unwrap(),
            Some(7)
        );
    }

    #[tokio::test]
    async fn fallback_skips_oversized_worktree_only_file() {
        let gcx = make_gcx().await;
        let fixture = make_fallback_fixture();
        let big = fixture.root.join("src").join("huge.rs");
        let mut content = String::from("needle marker\n");
        content.push_str(&"a".repeat(MAX_SEARCH_FILE_BYTES + 1));
        fs::write(&big, content).unwrap();

        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let read_context = prepare_file_read_context(gcx.clone()).await;
        let abort = AtomicBool::new(false);

        let outcome = direct_worktree_fallback_search(
            gcx,
            Some(&scope),
            &read_context,
            "workspace",
            "needle",
            0,
            50,
            &abort,
        )
        .await
        .unwrap();

        assert!(
            outcome.context_files.is_empty(),
            "oversized worktree-only file must be skipped by the capped read"
        );
        assert!(!outcome.truncated);
    }

    #[tokio::test]
    async fn fallback_stops_at_candidate_budget_and_reports_truncation() {
        let gcx = make_gcx().await;
        let fixture = make_fallback_fixture();
        for idx in 0..(FALLBACK_MAX_CANDIDATE_FILES + 5) {
            let path = fixture.root.join("src").join(format!("f_{idx:05}.rs"));
            fs::write(&path, "harmless content\n").unwrap();
        }

        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let read_context = prepare_file_read_context(gcx.clone()).await;
        let abort = AtomicBool::new(false);

        let outcome = direct_worktree_fallback_search(
            gcx,
            Some(&scope),
            &read_context,
            "workspace",
            "needle",
            0,
            50,
            &abort,
        )
        .await
        .unwrap();

        assert!(
            outcome.truncated,
            "candidate budget must stop the fallback and flag truncation"
        );
        assert!(outcome.context_files.is_empty());
    }

    #[tokio::test]
    async fn fallback_respects_preset_abort_flag() {
        let gcx = make_gcx().await;
        let fixture = make_fallback_fixture();
        let file = fixture.root.join("src").join("worktree_only.rs");
        fs::write(&file, "needle marker\n").unwrap();

        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let read_context = prepare_file_read_context(gcx.clone()).await;
        let abort = AtomicBool::new(true);

        let result = direct_worktree_fallback_search(
            gcx,
            Some(&scope),
            &read_context,
            "workspace",
            "needle",
            0,
            50,
            &abort,
        )
        .await;

        assert!(result.is_err(), "preset abort must return an error");
        assert!(
            result.unwrap_err().contains("aborted"),
            "error should mention the abort"
        );
    }
}
