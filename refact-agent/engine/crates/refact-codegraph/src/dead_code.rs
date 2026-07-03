use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadSymbol {
    pub node_id: i64,
    pub name: String,
    pub path: String,
    pub reason: String,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
struct Reachability {
    symbols: BTreeMap<i64, (String, String)>,
    reachable: BTreeSet<i64>,
    incoming_calls: BTreeMap<i64, usize>,
}

pub fn dead_code(store: &Store) -> Result<Vec<DeadSymbol>, String> {
    let reachability = analyze_reachability(store)?;
    let mut dead = Vec::new();

    for (node_id, (name, path)) in &reachability.symbols {
        if reachability.reachable.contains(node_id) {
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
        dead.push(DeadSymbol {
            node_id: *node_id,
            name: name.clone(),
            path: path.clone(),
            reason: "no callers, unreachable from entry points".to_string(),
            confidence: confidence_for(name),
        });
    }

    dead.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    dead.truncate(500);
    Ok(dead)
}

pub fn reachable_count(store: &Store) -> Result<(usize, usize), String> {
    let reachability = analyze_reachability(store)?;
    Ok((reachability.reachable.len(), reachability.symbols.len()))
}

fn analyze_reachability(store: &Store) -> Result<Reachability, String> {
    let nodes = store.node_names()?;
    let symbol_ids: BTreeSet<i64> = store
        .all_symbols()?
        .into_iter()
        .map(|(_dcp, node_id)| node_id)
        .collect();
    let symbols: BTreeMap<i64, (String, String)> = nodes
        .into_iter()
        .filter(|(id, _, _)| symbol_ids.contains(id))
        .map(|(id, name, path)| (id, (name, path)))
        .collect();
    let known_ids: HashSet<i64> = symbols.keys().copied().collect();

    let mut out = BTreeMap::<i64, Vec<i64>>::new();
    let mut incoming_calls = BTreeMap::<i64, usize>::new();
    let mut route_targets = BTreeSet::<i64>::new();

    for (src, dst, kind) in store.graph_edges()? {
        if !is_reachability_edge(&kind) {
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
        if kind == "calls" {
            *incoming_calls.entry(dst).or_default() += 1;
        }
    }

    for targets in out.values_mut() {
        targets.sort_unstable();
        targets.dedup();
    }

    let dcp_by_id: HashMap<i64, String> = store
        .all_symbols()?
        .into_iter()
        .map(|(dcp, node_id)| (node_id, dcp))
        .collect();
    let has_explicit_entry = symbols.iter().any(|(id, (name, _))| {
        is_named_entry_point(name) || is_test_name(name) || route_targets.contains(id)
    });

    let mut roots = BTreeSet::<i64>::new();
    for (id, (name, _path)) in &symbols {
        if is_named_entry_point(name) || is_test_name(name) || route_targets.contains(id) {
            roots.insert(*id);
            continue;
        }
        if incoming_calls.get(id).copied().unwrap_or(0) == 0
            && likely_exported(
                name,
                dcp_by_id.get(id).map(String::as_str),
                has_explicit_entry,
            )
        {
            roots.insert(*id);
        }
    }

    let reachable = reachable_from(&roots, &out);
    Ok(Reachability {
        symbols,
        reachable,
        incoming_calls,
    })
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

fn confidence_for(name: &str) -> f64 {
    if looks_dynamic(name) {
        0.3
    } else {
        0.5
    }
}

fn looks_dynamic(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with("__")
        || name.ends_with("__")
        || matches!(
            lower.as_str(),
            "new" | "default" | "drop" | "clone" | "fmt" | "serialize" | "deserialize"
        )
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
    fn dunder_name_has_lower_confidence_than_plain_orphan() {
        let store = store_with("fn main(){} fn orphan(){} fn __init__(){}");
        let dead = dead_code(&store).unwrap();
        let plain = dead.iter().find(|s| s.name == "orphan").unwrap();
        let dunder = dead.iter().find(|s| s.name == "__init__").unwrap();

        assert!(
            dunder.confidence < plain.confidence,
            "dead symbols: {dead:?}"
        );
    }
}
