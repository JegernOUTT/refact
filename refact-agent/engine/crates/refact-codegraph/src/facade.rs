use std::path::Path;
use std::sync::Arc;

use refact_core::ast_types::{AstCounters, AstDefinition, AstUsage, SymbolType};
use refact_codegraph_parsers::{SymbolKind, SymbolNode};

use crate::store::{Store, SymbolData};

fn symbol_type_of(kind: &SymbolKind) -> SymbolType {
    match kind {
        SymbolKind::Module => SymbolType::Module,
        SymbolKind::Struct => SymbolType::StructDeclaration,
        SymbolKind::TypeAlias => SymbolType::TypeAlias,
        SymbolKind::ClassField => SymbolType::ClassFieldDeclaration,
        SymbolKind::Import => SymbolType::ImportDeclaration,
        SymbolKind::Variable => SymbolType::VariableDefinition,
        SymbolKind::Function => SymbolType::FunctionDeclaration,
        SymbolKind::Comment => SymbolType::CommentDefinition,
        SymbolKind::Unknown => SymbolType::Unknown,
    }
}

fn file_stem(cpath: &str) -> String {
    Path::new(cpath)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cpath.to_string())
}

fn friendly_dcp(dcp: &str) -> String {
    if let Some((namespace, symbol_path)) = dcp.split_once("::") {
        if symbol_path.is_empty() {
            file_stem(namespace)
        } else {
            format!("{}::{}", file_stem(namespace), symbol_path)
        }
    } else {
        dcp.to_string()
    }
}

pub fn symbol_node_to_ast(
    symbol: &SymbolNode,
    node_path: &str,
    double_colon_path: &str,
    usages: Vec<AstUsage>,
) -> AstDefinition {
    let official_path = if double_colon_path.is_empty() {
        let mut path = vec![node_path.to_string()];
        path.extend(symbol.official_path.iter().cloned());
        path
    } else {
        double_colon_path
            .split("::")
            .map(|part| part.to_string())
            .collect()
    };
    AstDefinition {
        official_path,
        symbol_type: symbol_type_of(&symbol.kind),
        usages,
        resolved_type: String::new(),
        this_is_a_class: symbol.this_is_a_class.clone(),
        this_class_derived_from: symbol.this_class_derived_from.clone(),
        cpath: node_path.to_string(),
        decl_line1: symbol.decl_line1.max(1),
        decl_line2: symbol.decl_line2.max(1),
        body_line1: symbol.body_line1.max(1),
        body_line2: symbol.body_line2.max(1),
    }
}

fn usage_to_ast(line: usize, resolved_as: String, kind: String) -> AstUsage {
    AstUsage {
        targets_for_guesswork: Vec::new(),
        resolved_as,
        debug_hint: kind,
        uline: line.saturating_sub(1),
    }
}

fn rows_to_defs(store: &Store, rows: Vec<SymbolData>) -> Result<Vec<Arc<AstDefinition>>, String> {
    let mut defs = Vec::new();
    for row in rows {
        let Ok(symbol) = serde_json::from_str::<SymbolNode>(&row.data) else {
            continue;
        };
        let usages = store
            .usage_data_for_node(row.node_id)?
            .into_iter()
            .map(|(line, resolved_as, kind)| usage_to_ast(line, resolved_as, kind))
            .collect();
        defs.push(Arc::new(symbol_node_to_ast(
            &symbol,
            &row.path,
            &row.double_colon_path,
            usages,
        )));
    }
    Ok(defs)
}

pub fn doc_defs(store: &Store, cpath: &str) -> Result<Vec<Arc<AstDefinition>>, String> {
    rows_to_defs(store, store.symbol_data_for_path(cpath)?)
}

pub fn definitions(
    store: &Store,
    double_colon_path: &str,
) -> Result<Vec<Arc<AstDefinition>>, String> {
    let mut rows = store.symbol_data_by_dcp(double_colon_path)?;
    if rows.is_empty() {
        for stored_dcp in store.all_symbol_dcps()? {
            if friendly_dcp(&stored_dcp) == double_colon_path {
                rows.extend(store.symbol_data_by_dcp(&stored_dcp)?);
            }
        }
    }
    rows_to_defs(store, rows)
}

pub fn definition_paths_fuzzy(
    store: &Store,
    pattern: &str,
    top_n: usize,
) -> Result<Vec<String>, String> {
    let needle = pattern.to_lowercase();
    let mut matches: Vec<String> = store
        .all_symbol_dcps()?
        .into_iter()
        .filter(|p| {
            p.to_lowercase().contains(&needle) || friendly_dcp(p).to_lowercase().contains(&needle)
        })
        .collect();
    matches.sort();
    matches.dedup();
    matches.truncate(top_n);
    Ok(matches)
}

pub fn type_hierarchy(store: &Store, subtree_of: &str) -> Result<String, String> {
    use std::collections::{BTreeMap, BTreeSet};

    let pairs = store.inherits_pairs()?;
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut all_children: BTreeSet<String> = BTreeSet::new();
    let mut all_nodes: BTreeSet<String> = BTreeSet::new();
    for (child, parent) in &pairs {
        children
            .entry(parent.clone())
            .or_default()
            .insert(child.clone());
        all_children.insert(child.clone());
        all_nodes.insert(child.clone());
        all_nodes.insert(parent.clone());
    }

    let roots: Vec<String> = if !subtree_of.is_empty() {
        vec![subtree_of.to_string()]
    } else {
        all_nodes
            .iter()
            .filter(|n| !all_children.contains(*n))
            .cloned()
            .collect()
    };

    fn render(
        node: &str,
        depth: usize,
        children: &BTreeMap<String, BTreeSet<String>>,
        out: &mut String,
        seen: &mut BTreeSet<String>,
    ) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(node);
        out.push('\n');
        if !seen.insert(node.to_string()) {
            return;
        }
        if let Some(kids) = children.get(node) {
            for kid in kids {
                render(kid, depth + 1, children, out, seen);
            }
        }
    }

    let mut out = String::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        render(&root, 0, &children, &mut out, &mut seen);
    }
    Ok(out)
}

pub fn fetch_counters(store: &Store) -> Result<AstCounters, String> {
    let c = store.counts()?;
    Ok(AstCounters {
        counter_defs: store.symbol_count()? as i32,
        counter_usages: store.usage_edge_count()? as i32,
        counter_docs: c.files as i32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_rust() -> Store {
        let store = Store::open_in_memory().unwrap();
        let src = "\
struct Widget;

impl Widget {
    fn render(&self) {
        helper();
    }
}

fn helper() {}
";
        store
            .index_file_graph("src/widget.rs", src, "rust")
            .unwrap();
        store
    }

    #[test]
    fn doc_defs_returns_ast_definitions_with_path_qualified_paths() {
        let store = store_with_rust();
        let defs = doc_defs(&store, "src/widget.rs").unwrap();
        let paths: Vec<String> = defs.iter().map(|d| d.path()).collect();
        assert!(paths.contains(&"src/widget.rs::Widget".to_string()));
        assert!(paths.contains(&"src/widget.rs::Widget::render".to_string()));
        assert!(paths.contains(&"src/widget.rs::helper".to_string()));

        let widget = defs.iter().find(|d| d.name() == "Widget").unwrap();
        assert_eq!(widget.symbol_type, SymbolType::StructDeclaration);
        assert_eq!(widget.this_is_a_class, "Widget");
        assert_eq!(widget.cpath, "src/widget.rs");

        let render = defs.iter().find(|d| d.name() == "render").unwrap();
        assert_eq!(render.symbol_type, SymbolType::FunctionDeclaration);
        assert!(render.full_line1() >= 1 && render.full_line2() >= render.full_line1());
    }

    #[test]
    fn doc_defs_returns_codegraph_usages() {
        let store = store_with_rust();
        store.connect_usages().unwrap();
        let defs = doc_defs(&store, "src/widget.rs").unwrap();
        let render = defs.iter().find(|d| d.name() == "render").unwrap();

        assert!(render.usages.iter().any(|usage| {
            usage.resolved_as == "src/widget.rs::helper"
                && usage.debug_hint == "calls"
                && usage.uline == 4
        }));
    }

    #[test]
    fn definitions_resolves_by_double_colon_path() {
        let store = store_with_rust();
        let defs = definitions(&store, "Widget::render").unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name(), "render");
    }

    #[test]
    fn definition_paths_fuzzy_matches_substring() {
        let store = store_with_rust();
        let hits = definition_paths_fuzzy(&store, "render", 10).unwrap();
        assert!(hits.iter().any(|p| p.contains("render")));
    }

    #[test]
    fn same_basename_facade_keeps_path_qualified_identities() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a/m.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph(
                "src/b/m.rs",
                "fn run() { helper(); }\nfn helper() {}\n",
                "rust",
            )
            .unwrap();
        store.connect_usages().unwrap();

        let helper_defs = definitions(&store, "m::helper").unwrap();
        let helper_paths: Vec<String> = helper_defs.iter().map(|d| d.path()).collect();
        assert_eq!(helper_defs.len(), 2, "helper defs: {helper_paths:?}");
        assert!(helper_paths.contains(&"src/a/m.rs::helper".to_string()));
        assert!(helper_paths.contains(&"src/b/m.rs::helper".to_string()));

        let fuzzy = definition_paths_fuzzy(&store, "helper", 10).unwrap();
        assert!(fuzzy.contains(&"src/a/m.rs::helper".to_string()));
        assert!(fuzzy.contains(&"src/b/m.rs::helper".to_string()));

        let run_defs = definitions(&store, "src/b/m.rs::run").unwrap();
        assert_eq!(run_defs.len(), 1);
        let run = &run_defs[0];
        assert_eq!(run.path(), "src/b/m.rs::run");
        assert!(run.usages.iter().any(|usage| {
            usage.resolved_as == "src/b/m.rs::helper" && usage.debug_hint == "calls"
        }));
        assert!(!run.usages.iter().any(|usage| {
            usage.resolved_as == "src/a/m.rs::helper" && usage.debug_hint == "calls"
        }));
    }

    #[test]
    fn fetch_counters_reports_docs_and_defs() {
        let store = store_with_rust();
        let counters = fetch_counters(&store).unwrap();
        assert_eq!(counters.counter_docs, 1);
        assert!(counters.counter_defs >= 3);
    }

    #[test]
    fn fetch_counters_counts_symbols_not_helper_nodes() {
        let store = Store::open_in_memory().unwrap();
        let file_id = store
            .insert_node("file", "src/api.rs", "api.rs", "rust", 1, 3)
            .unwrap();
        let route_id = store
            .insert_node("route", "src/api.rs", "GET /users", "rust", 0, 0)
            .unwrap();
        store
            .add_edge(route_id, file_id, "defined_in", 1.0)
            .unwrap();

        let counters = fetch_counters(&store).unwrap();

        assert_eq!(counters.counter_docs, 1);
        assert_eq!(counters.counter_defs, 0);
        assert_eq!(counters.counter_usages, 0);
    }

    #[test]
    fn type_hierarchy_renders_inheritance_chain() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a.py", "class A:\n    pass\n", "python")
            .unwrap();
        store
            .index_file_graph("src/b.py", "class B(A):\n    pass\n", "python")
            .unwrap();
        store
            .index_file_graph("src/c.py", "class C(B):\n    pass\n", "python")
            .unwrap();
        store.connect_usages().unwrap();

        let tree = type_hierarchy(&store, "").unwrap();
        assert!(tree.contains('A'), "tree should mention root A: {tree}");
        assert!(tree.contains('B'));
        assert!(tree.contains('C'));
        let a_pos = tree.find("A").unwrap();
        let b_pos = tree.find("B").unwrap();
        assert!(a_pos < b_pos, "A (root) should appear before B");
    }
}
