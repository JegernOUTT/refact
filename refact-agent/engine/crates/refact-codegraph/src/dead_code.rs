use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::store::{Store, SymbolRecord};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadSymbol {
    pub node_id: i64,
    pub name: String,
    pub path: String,
    pub line: usize,
    pub reason: String,
    pub confidence: f64,
    pub incoming_edges: usize,
}

#[derive(Debug, Clone)]
struct Reachability {
    symbols: BTreeMap<i64, SymbolRecord>,
    reachable: BTreeSet<i64>,
    incoming_calls: BTreeMap<i64, usize>,
    incoming_edges: BTreeMap<i64, usize>,
}

pub fn dead_code(store: &Store) -> Result<Vec<DeadSymbol>, String> {
    let symbols = store.symbol_records()?;
    let dcp_pairs = store.all_symbols()?;
    let edges = store.graph_edges()?;
    Ok(dead_code_from_parts(symbols, dcp_pairs, &edges))
}

pub fn dead_code_from_parts(
    symbols: Vec<SymbolRecord>,
    dcp_pairs: Vec<(String, i64)>,
    edges: &[(i64, i64, String)],
) -> Vec<DeadSymbol> {
    let reachability = analyze_reachability_from_parts(symbols, dcp_pairs, edges);
    let mut dead = Vec::new();

    for (node_id, symbol) in &reachability.symbols {
        if reachability.reachable.contains(node_id) || is_excluded_symbol(symbol) {
            continue;
        }
        if reachability
            .incoming_calls
            .get(node_id)
            .copied()
            .unwrap_or(0)
            != 0
        {
            continue;
        }
        let incoming_edges = reachability
            .incoming_edges
            .get(node_id)
            .copied()
            .unwrap_or(0);
        dead.push(DeadSymbol {
            node_id: *node_id,
            name: symbol.name.clone(),
            path: symbol.path.clone(),
            line: symbol.line1,
            reason: "no callers, unreachable from entry points".to_string(),
            confidence: confidence_for(incoming_edges),
            incoming_edges,
        });
    }

    dead.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    dead.truncate(500);
    dead
}

pub fn reachable_count(store: &Store) -> Result<(usize, usize), String> {
    let symbols = store.symbol_records()?;
    let dcp_pairs = store.all_symbols()?;
    let edges = store.graph_edges()?;
    let reachability = analyze_reachability_from_parts(symbols, dcp_pairs, &edges);
    Ok((reachability.reachable.len(), reachability.symbols.len()))
}

fn analyze_reachability_from_parts(
    symbols: Vec<SymbolRecord>,
    dcp_pairs: Vec<(String, i64)>,
    edges: &[(i64, i64, String)],
) -> Reachability {
    let symbols: BTreeMap<i64, SymbolRecord> = symbols
        .into_iter()
        .map(|symbol| (symbol.node_id, symbol))
        .collect();
    let known_ids: HashSet<i64> = symbols.keys().copied().collect();

    let mut out = BTreeMap::<i64, Vec<i64>>::new();
    let mut incoming_calls = BTreeMap::<i64, usize>::new();
    let mut incoming_edges = BTreeMap::<i64, usize>::new();
    let mut route_targets = BTreeSet::<i64>::new();

    for (src, dst, kind) in edges {
        let (src, dst) = (*src, *dst);
        if known_ids.contains(&dst) {
            *incoming_edges.entry(dst).or_default() += 1;
            if kind == "calls" {
                *incoming_calls.entry(dst).or_default() += 1;
            }
        }
        if !is_reachability_edge(kind) {
            continue;
        }
        if kind == "route_handler" && known_ids.contains(&dst) {
            route_targets.insert(dst);
            if known_ids.contains(&src) {
                out.entry(src).or_default().push(dst);
            }
            continue;
        }
        if !known_ids.contains(&src) || !known_ids.contains(&dst) {
            continue;
        }
        out.entry(src).or_default().push(dst);
    }

    for targets in out.values_mut() {
        targets.sort_unstable();
        targets.dedup();
    }

    let dcp_by_id: HashMap<i64, String> = dcp_pairs
        .into_iter()
        .map(|(dcp, node_id)| (node_id, dcp))
        .collect();
    let has_explicit_entry = symbols.iter().any(|(id, symbol)| {
        is_named_entry_point(&symbol.name)
            || is_test_name(&symbol.name)
            || route_targets.contains(id)
    });

    let mut roots = BTreeSet::<i64>::new();
    for (id, symbol) in &symbols {
        if is_named_entry_point(&symbol.name)
            || is_test_name(&symbol.name)
            || route_targets.contains(id)
        {
            roots.insert(*id);
            continue;
        }
        if incoming_calls.get(id).copied().unwrap_or(0) == 0
            && likely_exported(
                &symbol.name,
                dcp_by_id.get(id).map(String::as_str),
                has_explicit_entry,
            )
        {
            roots.insert(*id);
        }
    }

    let reachable = reachable_from(&roots, &out);
    Reachability {
        symbols,
        reachable,
        incoming_calls,
        incoming_edges,
    }
}

fn is_excluded_symbol(symbol: &SymbolRecord) -> bool {
    is_override_symbol(&symbol.data)
        || (is_function(symbol) && is_build_script_path(&symbol.path))
        || (is_function(symbol) && is_shell_entrypoint_path(&symbol.path, &symbol.lang))
}

fn is_function(symbol: &SymbolRecord) -> bool {
    symbol.kind == "function"
}

fn is_override_symbol(data: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| value.get("override").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn is_build_script_path(path: &str) -> bool {
    const BUILD_SCRIPT_NAMES: &[&str] = &[
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "CMakeLists.txt",
        "Makefile",
        "makefile",
        "GNUmakefile",
    ];
    const BUILD_SCRIPT_SUFFIXES: &[&str] = &[".gradle", ".gradle.kts", ".cmake", ".mk"];
    let normalized = normalize_path(path);
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    BUILD_SCRIPT_NAMES.iter().any(|name| basename == *name)
        || BUILD_SCRIPT_SUFFIXES
            .iter()
            .any(|suffix| basename.ends_with(suffix))
}

fn is_shell_entrypoint_path(path: &str, lang: &str) -> bool {
    let normalized = normalize_path(path).to_ascii_lowercase();
    let segments = normalized.split('/').collect::<Vec<_>>();
    let basename = segments.last().copied().unwrap_or(normalized.as_str());
    let parent = segments.iter().rev().nth(1).copied().unwrap_or_default();
    let Some(stem) = basename.strip_suffix(".sh") else {
        return false;
    };
    (lang == "bash" || basename.ends_with(".sh"))
        && (stem.starts_with("install")
            || stem.starts_with("setup")
            || parent == "install"
            || parent == "setup")
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn is_reachability_edge(kind: &str) -> bool {
    matches!(kind, "calls" | "inherits" | "route_handler")
}

fn reachable_from(roots: &BTreeSet<i64>, out: &BTreeMap<i64, Vec<i64>>) -> BTreeSet<i64> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();

    for root in roots {
        seen.insert(*root);
        queue.push_back(*root);
    }

    while let Some(id) = queue.pop_front() {
        if let Some(targets) = out.get(&id) {
            for target in targets {
                if seen.insert(*target) {
                    queue.push_back(*target);
                }
            }
        }
    }

    seen
}

fn is_named_entry_point(name: &str) -> bool {
    matches!(name, "main" | "run")
}

fn is_test_name(name: &str) -> bool {
    name.ends_with("_test") || name.starts_with("test_") || name.ends_with("Test")
}

fn likely_exported(name: &str, dcp: Option<&str>, has_explicit_entry: bool) -> bool {
    starts_uppercase(name) || (!has_explicit_entry && is_top_level_symbol(dcp))
}

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().map(char::is_uppercase).unwrap_or(false)
}

fn is_top_level_symbol(dcp: Option<&str>) -> bool {
    dcp.map(|path| path.split("::").count() <= 2)
        .unwrap_or(false)
}

fn confidence_for(incoming_edges: usize) -> f64 {
    let mut confidence: f64 = 0.4;
    if incoming_edges == 0 {
        confidence += 0.1;
    }
    confidence.clamp(0.1, 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(src: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.index_file_graph("src/lib.rs", src, "rust").unwrap();
        store.connect_usages().unwrap();
        store
    }

    #[test]
    fn orphan_is_flagged_but_main_and_used_are_not() {
        let store = store_with("fn main(){ used(); } fn used(){} fn orphan(){}");
        let dead = dead_code(&store).unwrap();
        let names: Vec<&str> = dead.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"orphan"), "dead symbols: {dead:?}");
        assert!(!names.contains(&"used"), "dead symbols: {dead:?}");
        assert!(!names.contains(&"main"), "dead symbols: {dead:?}");
    }

    #[test]
    fn inherited_base_is_reachable_but_orphan_stays_dead() {
        let store = Store::open_in_memory().unwrap();
        let main = store
            .insert_node("function", "src/lib.rs", "main", "rust", 1, 1)
            .unwrap();
        let sub = store
            .insert_node("class", "src/lib.rs", "sub", "rust", 2, 2)
            .unwrap();
        let base = store
            .insert_node("class", "src/lib.rs", "base", "rust", 3, 3)
            .unwrap();
        let orphan = store
            .insert_node("class", "src/lib.rs", "orphan", "rust", 4, 4)
            .unwrap();
        store.add_symbol("src/lib.rs::main", main).unwrap();
        store.add_symbol("src/lib.rs::sub", sub).unwrap();
        store.add_symbol("src/lib.rs::base", base).unwrap();
        store.add_symbol("src/lib.rs::orphan", orphan).unwrap();
        store.add_edge(main, sub, "calls", 1.0).unwrap();
        store.add_edge(sub, base, "inherits", 1.0).unwrap();

        let dead = dead_code(&store).unwrap();
        let names: Vec<&str> = dead.iter().map(|s| s.name.as_str()).collect();

        assert!(!names.contains(&"base"), "dead symbols: {dead:?}");
        assert!(names.contains(&"orphan"), "dead symbols: {dead:?}");
    }

    #[test]
    fn reachable_graph_has_empty_dead_list() {
        let store = store_with("fn main(){ used(); } fn used(){ leaf(); } fn leaf(){}");
        assert!(dead_code(&store).unwrap().is_empty());
        assert_eq!(reachable_count(&store).unwrap(), (3, 3));
    }

    #[test]
    fn override_methods_not_dead() {
        let store = Store::open_in_memory().unwrap();
        let src = "class Plugin {
    override fun dispose() {
    }

    fun orphan() {
    }
}

fun main() {
}
";
        store
            .index_file_graph("src/Plugin.kt", src, "kotlin")
            .unwrap();
        store.connect_usages().unwrap();

        let dead = dead_code(&store).unwrap();
        let names: Vec<&str> = dead.iter().map(|s| s.name.as_str()).collect();

        assert!(!names.contains(&"dispose"), "dead symbols: {dead:?}");
        assert!(names.contains(&"orphan"), "dead symbols: {dead:?}");
    }

    #[test]
    fn bash_command_called_function_not_dead() {
        let store = Store::open_in_memory().unwrap();
        let src = "main() { :; }\nfoo() { :; }\nbar() { :; }\nfoo\n";
        store
            .index_file_graph("scripts/tool.sh", src, "bash")
            .unwrap();
        store.connect_usages().unwrap();

        let dead = dead_code(&store).unwrap();
        let names: Vec<&str> = dead.iter().map(|s| s.name.as_str()).collect();

        assert!(!names.contains(&"foo"), "dead symbols: {dead:?}");
        assert!(names.contains(&"bar"), "dead symbols: {dead:?}");
    }

    #[test]
    fn build_script_functions_are_excluded() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph(
                "build.gradle.kts",
                "fun getVersionString() = \"1\"\n",
                "kotlin",
            )
            .unwrap();
        store.connect_usages().unwrap();

        let dead = dead_code(&store).unwrap();

        assert!(dead.is_empty(), "dead symbols: {dead:?}");
    }

    #[test]
    fn zero_incoming_edge_confidence_is_above_non_call_incoming_edge() {
        let store = Store::open_in_memory().unwrap();
        let main = store
            .insert_node("function", "src/lib.rs", "main", "rust", 1, 1)
            .unwrap();
        let imported = store
            .insert_node("function", "src/lib.rs", "imported", "rust", 2, 2)
            .unwrap();
        let orphan = store
            .insert_node("function", "src/lib.rs", "orphan", "rust", 3, 3)
            .unwrap();
        store.add_symbol("src/lib.rs::main", main).unwrap();
        store.add_symbol("src/lib.rs::imported", imported).unwrap();
        store.add_symbol("src/lib.rs::orphan", orphan).unwrap();
        store.add_edge(main, imported, "imports", 1.0).unwrap();

        let dead = dead_code(&store).unwrap();
        let imported = dead.iter().find(|s| s.name == "imported").unwrap();
        let orphan = dead.iter().find(|s| s.name == "orphan").unwrap();

        assert!(
            orphan.confidence > imported.confidence,
            "dead symbols: {dead:?}"
        );
    }
}
