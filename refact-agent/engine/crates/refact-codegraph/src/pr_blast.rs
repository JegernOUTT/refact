use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlastImpact {
    pub path: String,
    pub symbol: String,
    pub distance: usize,
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlastReport {
    pub changed_files: Vec<String>,
    pub directly_impacted: Vec<BlastImpact>,
    pub transitively_impacted: Vec<BlastImpact>,
    pub impacted_file_count: usize,
    pub risk_score: f64,
}

#[derive(Debug, Clone)]
struct NodeInfo {
    name: String,
    path: String,
}

pub fn blast_radius(
    store: &Store,
    changed_files: &[String],
    max_depth: usize,
) -> Result<BlastReport, String> {
    let changed_set: BTreeSet<String> = changed_files.iter().cloned().collect();
    let changed_files: Vec<String> = changed_set.iter().cloned().collect();

    if changed_set.is_empty() || max_depth == 0 {
        return Ok(BlastReport {
            changed_files,
            directly_impacted: Vec::new(),
            transitively_impacted: Vec::new(),
            impacted_file_count: 0,
            risk_score: 0.0,
        });
    }

    let nodes = store.node_names()?;
    let mut graph: DiGraph<i64, String> = DiGraph::new();
    let mut id_to_index: HashMap<i64, NodeIndex> = HashMap::new();
    let mut info_by_id: HashMap<i64, NodeInfo> = HashMap::new();

    for (id, name, path) in nodes {
        let index = *id_to_index.entry(id).or_insert_with(|| graph.add_node(id));
        graph[index] = id;
        info_by_id.insert(id, NodeInfo { name, path });
    }

    for (src, dst, kind) in store.graph_edges()? {
        if !is_blast_edge(&kind) {
            continue;
        }
        let Some(&src_index) = id_to_index.get(&src) else {
            continue;
        };
        let Some(&dst_index) = id_to_index.get(&dst) else {
            continue;
        };
        graph.add_edge(src_index, dst_index, kind);
    }

    let mut starts: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|idx| {
            info_by_id
                .get(&graph[*idx])
                .map(|info| changed_set.contains(&info.path))
                .unwrap_or(false)
        })
        .collect();
    starts.sort_by_key(|idx| graph[*idx]);

    let start_ids: HashSet<i64> = starts.iter().map(|idx| graph[*idx]).collect();
    let mut seen: HashMap<i64, usize> = HashMap::new();
    let mut best_impacts: HashMap<i64, BlastImpact> = HashMap::new();
    let mut queue = VecDeque::new();

    for start in starts {
        seen.entry(graph[start]).or_insert(0);
        queue.push_back((start, 0usize));
    }

    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let mut incoming: Vec<(String, String, String, i64, NodeIndex)> = graph
            .edges_directed(node, Direction::Incoming)
            .filter_map(|edge| {
                let source = edge.source();
                let source_id = graph[source];
                info_by_id.get(&source_id).map(|info| {
                    (
                        edge.weight().clone(),
                        info.path.clone(),
                        info.name.clone(),
                        source_id,
                        source,
                    )
                })
            })
            .collect();
        incoming.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
        });

        for (via, path, symbol, source_id, source) in incoming {
            let next_depth = depth + 1;
            let should_visit = match seen.get(&source_id) {
                Some(&known_depth) => next_depth < known_depth,
                None => true,
            };

            if should_visit {
                seen.insert(source_id, next_depth);
                if next_depth < max_depth {
                    queue.push_back((source, next_depth));
                }
            }

            if start_ids.contains(&source_id) || changed_set.contains(&path) {
                continue;
            }

            let candidate = BlastImpact {
                path,
                symbol,
                distance: next_depth,
                via,
            };
            let replace = best_impacts
                .get(&source_id)
                .map(|current| impact_cmp(&candidate, current).is_lt())
                .unwrap_or(true);
            if replace {
                best_impacts.insert(source_id, candidate);
            }
        }
    }

    let mut impacts: Vec<BlastImpact> = best_impacts.into_values().collect();
    impacts.sort_by(impact_cmp);

    let impacted_paths: BTreeSet<String> =
        impacts.iter().map(|impact| impact.path.clone()).collect();
    let impacted_file_count = impacted_paths.len();
    let max_distance = impacts
        .iter()
        .map(|impact| impact.distance)
        .max()
        .unwrap_or(0);
    let risk_score = risk_score(impacted_file_count, max_distance, max_depth);

    let (directly_impacted, transitively_impacted): (Vec<_>, Vec<_>) =
        impacts.into_iter().partition(|impact| impact.distance == 1);

    Ok(BlastReport {
        changed_files,
        directly_impacted,
        transitively_impacted,
        impacted_file_count,
        risk_score,
    })
}

pub fn reviewers_for_blast(report: &BlastReport) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for impact in report
        .directly_impacted
        .iter()
        .chain(report.transitively_impacted.iter())
    {
        *counts.entry(impact.path.clone()).or_insert(0) += 1;
    }

    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(10);
    ranked.into_iter().map(|(path, _)| path).collect()
}

fn is_blast_edge(kind: &str) -> bool {
    matches!(kind, "calls" | "inherits" | "route_handler")
}

fn impact_cmp(a: &BlastImpact, b: &BlastImpact) -> std::cmp::Ordering {
    a.distance
        .cmp(&b.distance)
        .then_with(|| a.path.cmp(&b.path))
        .then_with(|| a.symbol.cmp(&b.symbol))
        .then_with(|| a.via.cmp(&b.via))
}

fn risk_score(impacted_file_count: usize, max_distance: usize, max_depth: usize) -> f64 {
    if impacted_file_count == 0 {
        return 0.0;
    }

    let breadth = 1.0 - (-(impacted_file_count as f64) / 10.0).exp();
    let depth = if max_depth == 0 {
        0.0
    } else {
        (max_distance as f64 / max_depth as f64).clamp(0.0, 1.0)
    };
    (0.8 * breadth + 0.2 * depth).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/c.rs", "pub fn c() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph("src/b.rs", "pub fn b() { c(); }\n", "rust")
            .unwrap();
        store
            .index_file_graph("src/a.rs", "pub fn a() { b(); }\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();
        store
    }

    #[test]
    fn reverse_calls_find_direct_and_transitive_impacts() {
        let store = chain_store();
        let report = blast_radius(&store, &["src/c.rs".to_string()], 3).unwrap();

        assert_eq!(report.changed_files, vec!["src/c.rs".to_string()]);
        assert!(
            report
                .directly_impacted
                .iter()
                .any(|impact| impact.path == "src/b.rs"
                    && impact.symbol == "b"
                    && impact.distance == 1
                    && impact.via == "calls"),
            "direct impacts: {:?}",
            report.directly_impacted
        );
        assert!(
            report
                .transitively_impacted
                .iter()
                .any(|impact| impact.path == "src/a.rs"
                    && impact.symbol == "a"
                    && impact.distance == 2
                    && impact.via == "calls"),
            "transitive impacts: {:?}",
            report.transitively_impacted
        );
    }

    #[test]
    fn changing_leaf_with_no_callers_has_no_impacts() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/leaf.rs", "pub fn leaf() {}\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();

        let report = blast_radius(&store, &["src/leaf.rs".to_string()], 3).unwrap();

        assert!(report.directly_impacted.is_empty());
        assert!(report.transitively_impacted.is_empty());
        assert_eq!(report.impacted_file_count, 0);
        assert_eq!(report.risk_score, 0.0);
    }

    #[test]
    fn impacted_file_count_counts_unique_files() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/core.rs", "pub fn c1() {}\npub fn c2() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph(
                "src/caller.rs",
                "pub fn b1() { c1(); }\npub fn b2() { c2(); }\n",
                "rust",
            )
            .unwrap();
        store.connect_usages().unwrap();

        let report = blast_radius(&store, &["src/core.rs".to_string()], 2).unwrap();

        assert_eq!(report.directly_impacted.len(), 2);
        assert_eq!(report.impacted_file_count, 1);
        assert_eq!(
            reviewers_for_blast(&report),
            vec!["src/caller.rs".to_string()]
        );
    }
}
