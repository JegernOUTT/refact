use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::ast::ast_structs::AstDB;
use crate::ast::ast_db::fetch_counters;
use crate::custom_error::trace_and_default;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::knowledge_index::format_related_memories_section;
use crate::tools::scope_utils::{format_scope_notices, remap_context_files_for_execution_scope};
use regex::Regex;

pub struct ToolAstDefinition {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolAstDefinition {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let mut corrections = false;
        let symbols_str = match args.get("symbols") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `symbols` is not a string: {:?}", v)),
            None => return Err("argument `symbols` is missing".to_string()),
        };

        let symbols: Vec<String> = symbols_str
            .split(',')
            .map(|s| s.trim().replace('.', "::"))
            .filter(|s| !s.is_empty())
            .collect();

        if symbols.is_empty() {
            return Err("No valid symbols provided".to_string());
        }

        let (gcx, execution_scope) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
            )
        };
        let ast_service_opt = gcx.read().await.ast_service.clone();
        if let Some(ast_service) = ast_service_opt {
            let ast_index = ast_service.lock().await.ast_index.clone();

            crate::ast::ast_indexer_thread::ast_indexer_block_until_finished(
                ast_service.clone(),
                20_000,
                true,
            )
            .await;

            let mut all_messages = Vec::new();
            let mut all_context_files = Vec::new();
            let mut all_scope_notices = Vec::new();

            for symbol in symbols {
                let defs =
                    crate::ast::ast_db::definitions(ast_index.clone(), &symbol).unwrap_or_default();

                if !defs.is_empty() {
                    const DEFS_LIMIT: usize = 20;
                    let raw_context_files: Vec<ContextFile> = defs
                        .iter()
                        .take(DEFS_LIMIT)
                        .map(|res| ContextFile {
                            file_name: res.cpath.clone(),
                            file_content: "".to_string(),
                            line1: res.full_line1(),
                            line2: res.full_line2(),
                            file_rev: None,
                            symbols: vec![res.path_drop0()],
                            gradient_type: 5,
                            usefulness: 100.0,
                            skip_pp: false,
                        })
                        .collect();
                    let (context_files, scope_notices) = remap_context_files_for_execution_scope(
                        gcx.clone(),
                        execution_scope.as_ref(),
                        raw_context_files,
                    )
                    .await?;
                    all_scope_notices.extend(scope_notices);

                    if context_files.is_empty() {
                        corrections = true;
                        all_messages.push(format!(
                            "For symbol `{}`:\n⚠️ Definitions were found in the source checkout, but none have files inside the active worktree scope.\n",
                            symbol
                        ));
                        continue;
                    }

                    let file_paths = context_files
                        .iter()
                        .map(|cf| cf.file_name.clone())
                        .collect::<Vec<_>>();
                    let short_file_paths =
                        crate::files_correction::shortify_paths(gcx.clone(), &file_paths).await;
                    let mut tool_message = format!("Definitions for `{}`:\n", symbol).to_string();
                    for (cf, short_path) in context_files.iter().zip(short_file_paths.iter()) {
                        let symbol_path = cf.symbols.get(0).cloned().unwrap_or_default();
                        tool_message.push_str(&format!(
                            "{} defined at {}:{}-{}\n",
                            symbol_path, short_path, cf.line1, cf.line2
                        ));
                    }

                    if defs.len() > DEFS_LIMIT {
                        tool_message.push_str(&format!(
                            "⚠️ {} more definitions not shown (limit: {}). 💡 Use more specific symbol name\n",
                            defs.len() - DEFS_LIMIT, DEFS_LIMIT
                        ));
                    }

                    all_messages.push(tool_message);
                    all_context_files
                        .extend(context_files.into_iter().map(ContextEnum::ContextFile));
                } else {
                    corrections = true;
                    let tool_message =
                        there_are_definitions_with_similar_names_though(ast_index.clone(), &symbol)
                            .await;
                    all_messages.push(format!("For symbol `{}`:\n{}", symbol, tool_message));
                }
            }

            let combined_message = format!(
                "{}{}",
                format_scope_notices(&all_scope_notices),
                all_messages.join("\n")
            );

            // Append related memories based on involved file paths.
            let related_section = {
                let idx_arc = { gcx.read().await.knowledge_index.clone() };
                let idx_guard = idx_arc.lock().await;
                let mut files: Vec<String> = all_context_files
                    .iter()
                    .filter_map(|c| match c {
                        ContextEnum::ContextFile(cf) => Some(cf.file_name.clone()),
                        _ => None,
                    })
                    .collect();
                files.sort();
                files.dedup();
                let mut cards = idx_guard.related_for_files(&files, 8);
                if cards.is_empty() {
                    cards = idx_guard.related_for_related_files(&files, 8);
                }

                // Also try entity-based lookup using the queried symbols (best-effort).
                if cards.is_empty() {
                    let mut ents: Vec<String> = Vec::new();
                    // Parse from the original args string (comma-separated)
                    for raw in symbols_str.split(',') {
                        let s = raw.trim();
                        if s.is_empty() {
                            continue;
                        }
                        let s = s.replace('.', "::");
                        // Prefer last segment as entity (often what's backticked in memories)
                        if let Some(last) = s.split("::").last() {
                            if !last.is_empty() {
                                ents.push(last.to_string());
                            }
                        }
                        ents.push(s);
                    }
                    ents.sort();
                    ents.dedup();

                    // Filter to reasonable identifier-ish tokens to avoid noise.
                    let id_re = Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_:]{1,100}$").unwrap();
                    ents.retain(|e| id_re.is_match(e));

                    if !ents.is_empty() {
                        cards = idx_guard.related_for_entities(&ents, 8);
                        if cards.is_empty() {
                            cards = idx_guard.related_for_related_entities(&ents, 8);
                        }
                    }
                }
                format_related_memories_section(&cards, None)
            };
            all_context_files.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!(
                    "{}{}",
                    combined_message, related_section
                )),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()), // Already compressed internally
                ..Default::default()
            }));

            Ok((corrections, all_context_files))
        } else {
            Err("attempt to use search_symbol_definition with no ast turned on".to_string())
        }
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "search_symbol_definition".to_string(),
            display_name: "Definition".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Find definition of a symbol in the project using AST".to_string(),
            input_schema: json_schema_from_params(&[("symbols", "string", "Comma-separated list of symbols to search for (functions, methods, classes, type aliases). No spaces allowed in symbol names.")], &["symbols"]),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["ast".to_string()]
    }
}

pub async fn there_are_definitions_with_similar_names_though(
    ast_index: Arc<AstDB>,
    symbol: &str,
) -> String {
    let fuzzy_matches: Vec<String> =
        crate::ast::ast_db::definition_paths_fuzzy(ast_index.clone(), symbol, 20, 5000)
            .await
            .unwrap_or_else(trace_and_default);

    let tool_message = if fuzzy_matches.is_empty() {
        let counters = fetch_counters(ast_index).unwrap_or_else(trace_and_default);
        format!(
            "⚠️ No definitions for '{}' found ({} total in AST). 💡 Check spelling or use search_pattern() to find\n",
            symbol, counters.counter_defs
        )
    } else {
        let mut msg = format!(
            "⚠️ No exact match for '{}'. 💡 Similar definitions found:\n",
            symbol
        );
        for line in fuzzy_matches {
            msg.push_str(&format!("{}\n", line));
        }
        msg
    };

    tool_message
}
