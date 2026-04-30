use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::info;

use async_trait::async_trait;
use itertools::Itertools;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::{vec_context_file_to_context_tools, AtCommandsContext};
use crate::at_commands::at_search::execute_at_search;
use crate::tools::scope_utils::{
    create_scope_filter_with_execution_scope, format_scope_notices,
    remap_context_files_for_execution_scope,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::knowledge_index::format_related_memories_section;

pub struct ToolSearch {
    pub config_path: String,
}

const DEFAULT_CONTEXT_LINES: usize = 0;
const DEFAULT_MAX_FILES: usize = 50;
const DEFAULT_MAX_RECS_PER_FILE: usize = 10;
const DEFAULT_MAX_TOTAL_RECS: usize = 200;

fn parse_usize_arg(args: &HashMap<String, Value>, key: &str) -> Result<Option<usize>, String> {
    match args.get(key) {
        Some(Value::Number(n)) => Ok(Some(n.as_u64().unwrap_or(0) as usize)),
        Some(Value::String(s)) => Ok(Some(s.parse::<usize>().unwrap_or(0))),
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

async fn execute_att_search(
    ccx: Arc<AMutex<AtCommandsContext>>,
    query: &String,
    scope: &String,
) -> Result<(Vec<ContextFile>, Vec<String>), String> {
    let (gcx, execution_scope) = {
        let ccx_locked = ccx.lock().await;
        (
            ccx_locked.global_context.clone(),
            ccx_locked.execution_scope.clone(),
        )
    };

    let scoped_filter =
        create_scope_filter_with_execution_scope(gcx.clone(), execution_scope.as_ref(), scope)
            .await?;

    info!("att-search: filter: {:?}", scoped_filter.filter);
    let context_files = execute_at_search(ccx.clone(), &query, scoped_filter.filter).await?;
    let (context_files, remap_notices) = remap_context_files_for_execution_scope(
        gcx.clone(),
        execution_scope.as_ref(),
        context_files,
    )
    .await?;
    let mut notices = scoped_filter.notices;
    notices.extend(remap_notices);
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

        let context_lines =
            parse_usize_arg(args, "context_lines")?.unwrap_or(DEFAULT_CONTEXT_LINES);
        let max_files = parse_usize_arg(args, "max_files")?.unwrap_or(DEFAULT_MAX_FILES);
        let max_recs_per_file =
            parse_usize_arg(args, "max_recs_per_file")?.unwrap_or(DEFAULT_MAX_RECS_PER_FILE);
        let max_total_recs =
            parse_usize_arg(args, "max_total_recs")?.unwrap_or(DEFAULT_MAX_TOTAL_RECS);

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
                execute_att_search(ccx.clone(), query, &scope).await?;
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
                let gcx = ccx.lock().await.global_context.clone();
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
            let gcx = ccx.lock().await.global_context.clone();
            let idx_arc = { gcx.read().await.knowledge_index.clone() };
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
