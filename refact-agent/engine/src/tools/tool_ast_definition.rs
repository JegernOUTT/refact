use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use async_trait::async_trait;
use refact_core::ast_types::{AstDefinition, SymbolType};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::scope_utils::{format_scope_notices, remap_context_file_for_execution_scope};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::knowledge_index::format_related_memories_section;
use crate::worktrees::scope::ExecutionScope;
use regex::Regex;

const MAX_SYMBOLS: usize = 16;
const DEFS_LIMIT: usize = 20;
const HIERARCHY_LIMIT: usize = 8;

pub struct ToolAstDefinition {
    pub config_path: String,
}

fn is_type_definition(def: &AstDefinition) -> bool {
    def.symbol_type == SymbolType::StructDeclaration || !def.this_is_a_class.is_empty()
}

async fn type_hierarchy_sections(
    service: &Arc<crate::codegraph::CodeGraphService>,
    defs: &[Arc<AstDefinition>],
    abort_flag: &Arc<AtomicBool>,
) -> Result<Vec<String>, String> {
    let mut seen = BTreeSet::new();
    let mut sections = Vec::new();
    for def in defs {
        if abort_flag.load(Ordering::SeqCst) {
            break;
        }
        if sections.len() >= HIERARCHY_LIMIT {
            break;
        }
        if !is_type_definition(def) {
            continue;
        }
        let name = def.name();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        let hierarchy = service.type_hierarchy(&name).await?;
        if hierarchy.trim().is_empty() {
            continue;
        }
        sections.push(format!(
            "Inheritance for `{}`:\n{}",
            name,
            hierarchy.trim_end()
        ));
    }
    Ok(sections)
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
    execution_scope: Option<&ExecutionScope>,
    abort_flag: Arc<AtomicBool>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let mut corrections = false;
    let mut all_messages = Vec::new();
    let mut all_context_files = Vec::new();
    let mut all_notices: Vec<String> = Vec::new();

    for symbol in symbols {
        if abort_flag.load(Ordering::SeqCst) {
            all_messages.push("⚠️ Aborted before all symbols were processed.".to_string());
            break;
        }

        let defs = service.definitions(symbol).await?;
        if defs.is_empty() {
            corrections = true;
            let fuzzy = service.definition_paths_fuzzy(symbol, 20).await?;
            if fuzzy.is_empty() {
                let counters = service.fetch_counters().await?;
                all_messages.push(format!(
                    "For symbol `{}`:\n⚠️ No definitions found ({} total in codegraph). 💡 Check spelling or use search_pattern() to find text\n",
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

        let mut context_files = Vec::new();
        for res in defs.iter() {
            let context_file = ContextFile {
                file_name: res.cpath.clone(),
                file_content: "".to_string(),
                line1: res.full_line1(),
                line2: res.full_line2(),
                file_rev: None,
                symbols: vec![res.path_drop0()],
                gradient_type: 5,
                usefulness: 100.0,
                skip_pp: false,
            };
            if let Some((context_file, notices)) =
                remap_context_file_for_execution_scope(gcx.clone(), execution_scope, context_file)
                    .await?
            {
                context_files.push(context_file);
                all_notices.extend(notices);
                if context_files.len() >= DEFS_LIMIT {
                    break;
                }
            }
        }

        if context_files.is_empty() {
            corrections = true;
            all_messages.push(format!(
                "For symbol `{}`:\n⚠️ Definitions found only outside the active worktree and were suppressed. 💡 Use search_pattern() within the worktree\n",
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
        let mut tool_message = format!("Definitions for `{}`:\n", symbol);
        for (cf, short_path) in context_files.iter().zip(short_file_paths.iter()) {
            let symbol_path = cf.symbols.get(0).cloned().unwrap_or_default();
            tool_message.push_str(&format!(
                "{} defined at {}:{}-{}\n",
                symbol_path, short_path, cf.line1, cf.line2
            ));
        }
        if abort_flag.load(Ordering::SeqCst) {
            all_messages.push(tool_message);
            all_context_files.extend(context_files.into_iter().map(ContextEnum::ContextFile));
            all_messages.push("⚠️ Aborted before type hierarchy was computed.".to_string());
            break;
        }
        let hierarchy_sections = type_hierarchy_sections(&service, &defs, &abort_flag).await?;
        if !hierarchy_sections.is_empty() {
            tool_message.push_str("Inheritance:\n");
            tool_message.push_str(&hierarchy_sections.join("\n\n"));
            tool_message.push('\n');
        }
        if defs.len() > context_files.len() {
            tool_message.push_str(&format!(
                "⚠️ {} more definitions not shown (limit: {}). 💡 Use more specific symbol name\n",
                defs.len() - context_files.len(),
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

    let notices_section = format_scope_notices(&all_notices);
    all_context_files.push(ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(format!(
            "{}{}{}",
            all_messages.join("\n"),
            notices_section,
            related_section
        )),
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

        let raw_symbol_count = symbols_str
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .count();
        if raw_symbol_count > MAX_SYMBOLS {
            return Err(format!(
                "⚠️ Too many symbols requested ({}). 💡 Pass at most {} comma-separated symbols per call.",
                raw_symbol_count, MAX_SYMBOLS
            ));
        }

        let symbols: Vec<String> = symbols_str
            .split(',')
            .map(|s| s.trim().replace('.', "::"))
            .filter(|s| !s.is_empty())
            .collect();

        if symbols.is_empty() {
            return Err("No valid symbols provided".to_string());
        }

        let (gcx, execution_scope, abort_flag) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
                cgcx.abort_flag.clone(),
            )
        };

        let codegraph_opt = gcx.codegraph.lock().await.clone();
        match codegraph_opt {
            Some(service) => {
                symbol_def_via_codegraph(
                    gcx.clone(),
                    service,
                    &symbols,
                    &symbols_str,
                    tool_call_id,
                    execution_scope.as_ref(),
                    abort_flag,
                )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::{FilePrivacySettings, PrivacySettings};
    use refact_core::worktree_meta::WorktreeMeta;
    use std::fs;
    use std::path::PathBuf;

    fn no_abort() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[tokio::test]
    async fn symbol_def_hierarchy_sections_include_type_chain() {
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/base.py", "class A:\n    pass\n", "python")
            .await
            .unwrap();
        service
            .index_file("src/mid.py", "class B(A):\n    pass\n", "python")
            .await
            .unwrap();
        service
            .index_file("src/leaf.py", "class C(B):\n    pass\n", "python")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let defs = service.definitions("B").await.unwrap();
        let sections = type_hierarchy_sections(&service, &defs, &no_abort())
            .await
            .unwrap();
        let rendered = sections.join("\n");

        assert!(rendered.contains("Inheritance for `B`"));
        assert!(rendered.contains("A"));
        assert!(rendered.contains("B"));
        assert!(rendered.contains("C"));
    }

    struct ScopeFixture {
        _temp: tempfile::TempDir,
        worktree: WorktreeMeta,
        root: PathBuf,
        source: PathBuf,
    }

    fn make_scope_fixture() -> ScopeFixture {
        let temp = tempfile::Builder::new()
            .prefix("refact-symbol-def-scope-")
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
        fs::write(root.join("src").join("lib.rs"), "pub fn shared() {}\n").unwrap();
        fs::write(source.join("src").join("lib.rs"), "pub fn shared() {}\n").unwrap();
        fs::write(
            source.join("src").join("source_only.rs"),
            "pub fn only_source() {}\n",
        )
        .unwrap();
        let root = dunce::simplified(&fs::canonicalize(root).unwrap()).to_path_buf();
        let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
        let worktree = WorktreeMeta {
            id: "wt-symbol-def".to_string(),
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
        ScopeFixture {
            _temp: temp,
            worktree,
            root,
            source,
        }
    }

    async fn scope_gcx(blocked: Vec<String>) -> Arc<crate::global_context::GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let privacy_settings = gcx.privacy_settings.clone();
            *privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
                privacy_rules: FilePrivacySettings {
                    only_send_to_servers_I_control: vec![],
                    blocked,
                },
                loaded_ts: u64::MAX / 2,
            });
        }
        gcx
    }

    fn context_file_names(results: &[ContextEnum]) -> Vec<String> {
        results
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ContextFile(file) => Some(file.file_name.replace('\\', "/")),
                _ => None,
            })
            .collect()
    }

    fn tool_text(results: &[ContextEnum]) -> String {
        results
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ChatMessage(message) => match &message.content {
                    ChatContent::SimpleText(text) => Some(text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
            .replace('\\', "/")
    }

    async fn index_absolute(
        service: &Arc<refact_codegraph::CodeGraphService>,
        path: &PathBuf,
        text: &str,
    ) {
        service
            .index_file(&path.to_string_lossy(), text, "rust")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn symbol_def_source_context_file_remaps_to_worktree() {
        let fixture = make_scope_fixture();
        let gcx = scope_gcx(vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        let source_lib = fixture.source.join("src").join("lib.rs");
        index_absolute(&service, &source_lib, "pub fn shared() {}\n").await;
        service.connect_usages().await.unwrap();

        let worktree_lib = fixture
            .root
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .replace('\\', "/");

        let (_corrections, results) = symbol_def_via_codegraph(
            gcx.clone(),
            service,
            &["shared".to_string()],
            "shared",
            &"call".to_string(),
            Some(&scope),
            no_abort(),
        )
        .await
        .unwrap();

        let names = context_file_names(&results);
        let text = tool_text(&results);
        assert!(
            names.iter().any(|n| n == &worktree_lib),
            "expected {worktree_lib} in {names:?}"
        );
        let source_lib_str = fixture
            .source
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            text.contains("Definitions for `shared`"),
            "definitions header missing: {text}"
        );
        assert!(
            !text.contains(&format!("defined at {source_lib_str}")),
            "definition line must not use source path: {text}"
        );
    }

    #[tokio::test]
    async fn symbol_def_source_only_context_file_is_dropped() {
        let fixture = make_scope_fixture();
        let gcx = scope_gcx(vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        let source_only = fixture.source.join("src").join("source_only.rs");
        index_absolute(&service, &source_only, "pub fn only_source() {}\n").await;
        service.connect_usages().await.unwrap();

        let (corrections, results) = symbol_def_via_codegraph(
            gcx.clone(),
            service,
            &["only_source".to_string()],
            "only_source",
            &"call".to_string(),
            Some(&scope),
            no_abort(),
        )
        .await
        .unwrap();

        let names = context_file_names(&results);
        let text = tool_text(&results);
        assert!(
            names.is_empty(),
            "source-only defs should be dropped: {names:?}"
        );
        assert!(corrections, "dropped defs should count as a correction");
        assert!(
            !text.contains("source_only.rs"),
            "must not leak source path: {text}"
        );
        assert!(
            text.contains("suppressed"),
            "should mention suppression: {text}"
        );
    }

    #[tokio::test]
    async fn symbol_def_blocked_context_file_is_suppressed() {
        let fixture = make_scope_fixture();
        let gcx = scope_gcx(vec!["*.rs".to_string()]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        let worktree_lib = fixture.root.join("src").join("lib.rs");
        index_absolute(&service, &worktree_lib, "pub fn shared() {}\n").await;
        service.connect_usages().await.unwrap();

        let result = symbol_def_via_codegraph(
            gcx.clone(),
            service,
            &["shared".to_string()],
            "shared",
            &"call".to_string(),
            Some(&scope),
            no_abort(),
        )
        .await;

        assert!(result.is_err(), "privacy-blocked defs should error out");
        let err = result.unwrap_err();
        assert!(
            err.contains("Blocked"),
            "error should mention Blocked: {err}"
        );
    }

    #[tokio::test]
    async fn symbol_def_symbol_count_cap_rejects_excess() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = Arc::new(AMutex::new(
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
        ));
        let mut tool = ToolAstDefinition {
            config_path: String::new(),
        };
        let many = (0..MAX_SYMBOLS + 5)
            .map(|i| format!("sym{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let args = HashMap::from_iter([("symbols".to_string(), Value::String(many))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();
        assert!(err.contains("Too many symbols"), "{err}");
    }

    #[tokio::test]
    async fn symbol_def_abort_stops_processing() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/lib.rs", "pub fn shared() {}\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let abort_flag = Arc::new(AtomicBool::new(true));
        let (_corrections, results) = symbol_def_via_codegraph(
            gcx.clone(),
            service,
            &["shared".to_string()],
            "shared",
            &"call".to_string(),
            None,
            abort_flag,
        )
        .await
        .unwrap();

        let names = context_file_names(&results);
        let text = tool_text(&results);
        assert!(
            names.is_empty(),
            "aborted call should not attach defs: {names:?}"
        );
        assert!(text.contains("Aborted"), "should note the abort: {text}");
    }
}
