use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;
use async_trait::async_trait;
use refact_core::ast_types::{AstDefinition, SymbolType};
use refact_chat_api::{
    attach_tool_enrichment, ToolEnrichment, ToolEnrichmentKind, ToolEnrichmentProvenance,
    ToolEnrichmentReference,
};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::scope_utils::{
    format_scope_notices, remap_context_file_for_execution_scope_for_model_context,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::knowledge_index::{KnowledgeCard, KnowledgeIndex};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::native_enrichment::{workspace_roots, NativeReferences};
use crate::worktrees::scope::ExecutionScope;
use regex::Regex;

const MAX_SYMBOLS: usize = 16;
const DEFS_LIMIT: usize = 20;
const HIERARCHY_LIMIT: usize = 8;
const RELATED_MEMORIES_LIMIT: usize = 8;

static IDENTIFIER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_:]{1,100}$").unwrap());

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

fn related_memory_entities(symbols_str: &str) -> Vec<String> {
    let mut entities = Vec::new();
    for raw in symbols_str.split(',') {
        let symbol = raw.trim();
        if symbol.is_empty() {
            continue;
        }
        let symbol = symbol.replace('.', "::");
        if let Some(last) = symbol.split("::").last() {
            if !last.is_empty() {
                entities.push(last.to_string());
            }
        }
        entities.push(symbol);
    }
    entities.sort();
    entities.dedup();
    entities.retain(|entity| IDENTIFIER_RE.is_match(entity));
    entities
}

async fn compute_related_memories(
    gcx: Arc<crate::global_context::GlobalContext>,
    mut files: Vec<String>,
    symbols_str: &str,
    abort_flag: &Arc<AtomicBool>,
) -> Vec<KnowledgeCard> {
    files.sort();
    files.dedup();
    let index = gcx.knowledge_index.clone();
    let Some(mut cards) = knowledge_index_lookup(&index, abort_flag, |index| {
        index.related_for_files(&files, RELATED_MEMORIES_LIMIT)
    })
    .await
    else {
        return Vec::new();
    };
    if cards.is_empty() {
        let Some(found) = knowledge_index_lookup(&index, abort_flag, |index| {
            index.related_for_related_files(&files, RELATED_MEMORIES_LIMIT)
        })
        .await
        else {
            return Vec::new();
        };
        cards = found;
    }
    if cards.is_empty() {
        let entities = related_memory_entities(symbols_str);
        if !entities.is_empty() {
            let Some(found) = knowledge_index_lookup(&index, abort_flag, |index| {
                index.related_for_entities(&entities, RELATED_MEMORIES_LIMIT)
            })
            .await
            else {
                return Vec::new();
            };
            cards = found;
            if cards.is_empty() {
                let Some(found) = knowledge_index_lookup(&index, abort_flag, |index| {
                    index.related_for_related_entities(&entities, RELATED_MEMORIES_LIMIT)
                })
                .await
                else {
                    return Vec::new();
                };
                cards = found;
            }
        }
    }
    cards
}

async fn knowledge_index_lookup<T>(
    index: &AMutex<KnowledgeIndex>,
    abort_flag: &AtomicBool,
    lookup: impl FnOnce(&KnowledgeIndex) -> T,
) -> Option<T> {
    if abort_flag.load(Ordering::SeqCst) {
        return None;
    }
    let lock = index.lock();
    tokio::pin!(lock);
    loop {
        tokio::select! {
            guard = &mut lock => {
                if abort_flag.load(Ordering::SeqCst) {
                    return None;
                }
                return Some(lookup(&guard));
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if abort_flag.load(Ordering::SeqCst) {
                    return None;
                }
            }
        }
    }
}

fn workspace_relative_path(path: &Path, roots: &[PathBuf]) -> Option<String> {
    let relative = roots.iter().find_map(|root| path.strip_prefix(root).ok())?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let relative = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str(),
            std::path::Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    (!relative.is_empty()).then_some(relative)
}

async fn related_memory_references(
    gcx: Arc<crate::global_context::GlobalContext>,
    cards: Vec<KnowledgeCard>,
    roots: &[PathBuf],
    execution_scope: Option<&ExecutionScope>,
    abort_flag: &Arc<AtomicBool>,
) -> Vec<ToolEnrichmentReference> {
    let mut references = Vec::new();
    for card in cards.into_iter().take(RELATED_MEMORIES_LIMIT) {
        if abort_flag.load(Ordering::SeqCst) {
            break;
        }
        let context_file = ContextFile {
            file_name: card.file_path.to_string_lossy().to_string(),
            file_content: String::new(),
            line1: 0,
            line2: 0,
            file_rev: None,
            symbols: vec![],
            gradient_type: 0,
            usefulness: 0.0,
            skip_pp: true,
        };
        let Ok(Some((context_file, _))) = remap_context_file_for_execution_scope_for_model_context(
            gcx.clone(),
            execution_scope,
            context_file,
        )
        .await
        else {
            continue;
        };
        let Some(target) = workspace_relative_path(Path::new(&context_file.file_name), roots)
        else {
            continue;
        };
        let mut reference = ToolEnrichmentReference::new(ToolEnrichmentKind::Path, target);
        reference.provenance = ToolEnrichmentProvenance::Native;
        reference.label = Some(card.title);
        reference.status = Some("related_memory".to_string());
        reference.source = Some("search_symbol_definition".to_string());
        references.push(reference);
    }
    references
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
    let roots = workspace_roots(&gcx, execution_scope);
    let mut references = NativeReferences::new();

    for symbol in symbols {
        if abort_flag.load(Ordering::SeqCst) {
            all_messages.push("⚠️ Aborted before all symbols were processed.".to_string());
            break;
        }

        let defs = service.definitions(symbol).await?;
        if defs.is_empty() {
            references.add_symbol(symbol, "not_found", "search_symbol_definition");
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
                remap_context_file_for_execution_scope_for_model_context(
                    gcx.clone(),
                    execution_scope,
                    context_file,
                )
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
            references.add_symbol(symbol, "suppressed", "search_symbol_definition");
            corrections = true;
            all_messages.push(format!(
                "For symbol `{}`:\n⚠️ Definitions found only outside the active worktree and were suppressed. 💡 Use search_pattern() within the worktree\n",
                symbol
            ));
            continue;
        }

        references.add_symbol(symbol, "found", "search_symbol_definition");
        for context_file in &context_files {
            references.add_context_file(
                context_file,
                &roots,
                "definition",
                "search_symbol_definition",
            );
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
            references.mark_truncated();
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
    let related_memories = if abort_flag.load(Ordering::SeqCst) {
        Vec::new()
    } else {
        compute_related_memories(gcx.clone(), files, symbols_str, &abort_flag).await
    };
    let related_references = if abort_flag.load(Ordering::SeqCst) {
        Vec::new()
    } else {
        related_memory_references(
            gcx.clone(),
            related_memories,
            &roots,
            execution_scope,
            &abort_flag,
        )
        .await
    };

    let notices_section = format_scope_notices(&all_notices);
    let mut tool_message = ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(format!("{}{}", all_messages.join("\n"), notices_section)),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    };
    if !related_references.is_empty() {
        attach_tool_enrichment(
            &mut tool_message,
            ToolEnrichment {
                references: related_references,
                ..Default::default()
            },
        );
    }
    references.attach(&mut tool_message);
    all_context_files.push(ContextEnum::ChatMessage(tool_message));

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
    use std::time::Duration;

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

    fn related_memory(path: PathBuf, filename: &str, entity: &str) -> KnowledgeCard {
        KnowledgeCard {
            id: path.to_string_lossy().to_string(),
            title: "Related memory".to_string(),
            summary: Some("Related memory summary".to_string()),
            description: None,
            tags: vec![],
            filenames: vec![filename.to_string()],
            entities: vec![entity.to_string()],
            related_files: vec![],
            related_entities: vec![],
            kind: Some("memory".to_string()),
            created: None,
            created_at: None,
            updated: None,
            file_path: path,
        }
    }

    async fn add_related_memory(
        gcx: Arc<crate::global_context::GlobalContext>,
        path: PathBuf,
        filename: &str,
        entity: &str,
    ) {
        gcx.knowledge_index
            .lock()
            .await
            .add_card(related_memory(path, filename, entity));
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
        let enrichment = results
            .iter()
            .find_map(|result| match result {
                ContextEnum::ChatMessage(message) if message.role == "tool" => {
                    refact_chat_api::tool_enrichment_from_extra(&message.extra)
                }
                _ => None,
            })
            .expect("definition enrichment");
        assert_eq!(enrichment.references[0].target, "shared");
        assert_eq!(enrichment.references[0].status.as_deref(), Some("found"));
        assert!(enrichment
            .references
            .iter()
            .any(|reference| reference.target == "src/lib.rs"));
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

    #[tokio::test]
    async fn symbol_def_related_memory_stays_in_sidecar_enrichment() {
        let fixture = make_scope_fixture();
        let gcx = scope_gcx(vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        let worktree_lib = fixture.root.join("src").join("lib.rs");
        index_absolute(&service, &worktree_lib, "pub fn shared() {}\n").await;
        service.connect_usages().await.unwrap();
        let (_corrections, baseline_results) = symbol_def_via_codegraph(
            gcx.clone(),
            service.clone(),
            &["shared".to_string()],
            "shared",
            &"call".to_string(),
            Some(&scope),
            no_abort(),
        )
        .await
        .unwrap();
        let baseline_text = tool_text(&baseline_results);
        let memory_path = fixture
            .root
            .join(".refact")
            .join("knowledge")
            .join("shared.md");
        fs::create_dir_all(memory_path.parent().unwrap()).unwrap();
        fs::write(&memory_path, "related\n").unwrap();
        add_related_memory(
            gcx.clone(),
            memory_path.clone(),
            &worktree_lib.to_string_lossy(),
            "shared",
        )
        .await;

        let (_corrections, results) = symbol_def_via_codegraph(
            gcx,
            service,
            &["shared".to_string()],
            "shared",
            &"call".to_string(),
            Some(&scope),
            no_abort(),
        )
        .await
        .unwrap();

        let text = tool_text(&results);
        assert_eq!(text, baseline_text);
        let enrichment = results
            .iter()
            .find_map(|result| match result {
                ContextEnum::ChatMessage(message) if message.role == "tool" => {
                    refact_chat_api::tool_enrichment_from_extra(&message.extra)
                }
                _ => None,
            })
            .expect("definition enrichment");
        assert!(enrichment.references.iter().any(|reference| {
            reference.target == ".refact/knowledge/shared.md"
                && reference.status.as_deref() == Some("related_memory")
        }));
    }

    #[tokio::test]
    async fn related_memory_lookup_releases_global_index_before_local_lookups() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        add_related_memory(
            gcx.clone(),
            PathBuf::from("/knowledge/shared.md"),
            "src/lib.rs",
            "shared",
        )
        .await;
        let lock = gcx.knowledge_index.lock().await;
        let abort_flag = no_abort();
        let lookup = compute_related_memories(
            gcx.clone(),
            vec!["src/lib.rs".to_string()],
            "shared",
            &abort_flag,
        );
        tokio::pin!(lookup);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut lookup)
                .await
                .is_err(),
            "related-memory lookup should wait only for the shared index lock"
        );
        drop(lock);
        assert_eq!(lookup.await.len(), 1);
    }

    #[tokio::test]
    async fn aborted_symbol_def_skips_related_memory_lookup() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/lib.rs", "pub fn shared() {}\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        let abort_flag = Arc::new(AtomicBool::new(true));
        let index_lock = gcx.knowledge_index.clone();
        let lock = index_lock.try_lock().expect("test owns index mutex");

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            symbol_def_via_codegraph(
                gcx,
                service,
                &["shared".to_string()],
                "shared",
                &"call".to_string(),
                None,
                abort_flag,
            ),
        )
        .await
        .expect("aborted symbol lookup must not attempt a related-memory index lock")
        .unwrap();
        drop(lock);

        assert!(tool_text(&result.1).contains("Aborted"));
    }

    #[test]
    fn related_memory_identifier_regex_is_static_and_filters_invalid_entities() {
        let first = std::ptr::addr_of!(*IDENTIFIER_RE);
        let second = std::ptr::addr_of!(*IDENTIFIER_RE);
        assert_eq!(first, second);
        assert_eq!(
            related_memory_entities("crate.shared, valid::Thing, invalid-name"),
            vec![
                "Thing".to_string(),
                "crate::shared".to_string(),
                "shared".to_string(),
                "valid::Thing".to_string(),
            ]
        );
    }
}
