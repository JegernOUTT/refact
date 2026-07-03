use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::knowledge_index::format_related_memories_section;
use regex::Regex;

pub struct ToolAstDefinition {
    pub config_path: String,
}

pub async fn compute_related_memories_section(
    gcx: Arc<crate::global_context::GlobalContext>,
    mut files: Vec<String>,
    symbols_str: &str,
) -> String {
    let idx_arc = gcx.knowledge_index.clone();
    let idx_guard = idx_arc.lock().await;
    files.sort();
    files.dedup();
    let mut cards = idx_guard.related_for_files(&files, 8);
    if cards.is_empty() {
        cards = idx_guard.related_for_related_files(&files, 8);
    }

    if cards.is_empty() {
        let mut ents: Vec<String> = Vec::new();
        for raw in symbols_str.split(',') {
            let s = raw.trim();
            if s.is_empty() {
                continue;
            }
            let s = s.replace('.', "::");
            if let Some(last) = s.split("::").last() {
                if !last.is_empty() {
                    ents.push(last.to_string());
                }
            }
            ents.push(s);
        }
        ents.sort();
        ents.dedup();

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
}

async fn symbol_def_via_codegraph(
    gcx: Arc<crate::global_context::GlobalContext>,
    service: Arc<crate::codegraph::CodeGraphService>,
    symbols: &[String],
    symbols_str: &str,
    tool_call_id: &String,
) -> Result<(bool, Vec<ContextEnum>), String> {
    const DEFS_LIMIT: usize = 20;
    let mut corrections = false;
    let mut all_messages = Vec::new();
    let mut all_context_files = Vec::new();

    for symbol in symbols {
        let defs = service.definitions(symbol).await?;
        if defs.is_empty() {
            corrections = true;
            let fuzzy = service.definition_paths_fuzzy(symbol, 20).await?;
            if fuzzy.is_empty() {
                let counters = service.fetch_counters().await?;
                all_messages.push(format!(
                    "For symbol `{}`:\n⚠️ No definitions found ({} total in codegraph). 💡 Check spelling or use regex_search() to find\n",
                    symbol, counters.counter_defs
                ));
            } else {
                let mut msg = format!(
                    "For symbol `{}`:\n⚠️ No exact match. 💡 Similar definitions found:\n",
                    symbol
                );
                for line in fuzzy {
                    msg.push_str(&format!("{}\n", line));
                }
                all_messages.push(msg);
            }
            continue;
        }

        let context_files: Vec<ContextFile> = defs
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

        let file_paths = context_files
            .iter()
            .map(|cf| cf.file_name.clone())
            .collect::<Vec<_>>();
        let short_file_paths =
            crate::files_correction::shortify_paths(gcx.clone(), &file_paths).await;
        let mut tool_message = format!("Definitions for `{}`:\n", symbol);
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
                defs.len() - DEFS_LIMIT,
                DEFS_LIMIT
            ));
        }
        all_messages.push(tool_message);
        all_context_files.extend(context_files.into_iter().map(ContextEnum::ContextFile));
    }

    let files: Vec<String> = all_context_files
        .iter()
        .filter_map(|c| match c {
            ContextEnum::ContextFile(cf) => Some(cf.file_name.clone()),
            _ => None,
        })
        .collect();
    let related_section = compute_related_memories_section(gcx.clone(), files, symbols_str).await;

    all_context_files.push(ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(format!("{}{}", all_messages.join("\n"), related_section)),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    }));

    Ok((corrections, all_context_files))
}

#[async_trait]
impl Tool for ToolAstDefinition {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
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

        let gcx = {
            let cgcx = ccx.lock().await;
            cgcx.app.gcx.clone()
        };

        let codegraph_opt = gcx.codegraph.lock().await.clone();
        match codegraph_opt {
            Some(service) => {
                symbol_def_via_codegraph(gcx.clone(), service, &symbols, &symbols_str, tool_call_id)
                    .await
            }
            None => Err("codegraph is not available".to_string()),
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
            description: "Find definition of a symbol in the project using the codegraph".to_string(),
            input_schema: json_schema_from_params(&[("symbols", "string", "Comma-separated list of symbols to search for (functions, methods, classes, type aliases). No spaces allowed in symbol names.")], &["symbols"]),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["codegraph".to_string()]
    }
}
