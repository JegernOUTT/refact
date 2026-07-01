use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::GitIntel;

const DEFAULT_TOP_N: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouplingEdge {
    pub a: String,
    pub b: String,
    pub strength: f64,
    pub co_changes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouplingNode {
    pub path: String,
    pub degree: usize,
    pub total_strength: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouplingGraph {
    pub edges: Vec<CouplingEdge>,
    pub nodes: Vec<CouplingNode>,
}

pub fn build_coupling_graph(intel: &GitIntel, top_n: usize) -> CouplingGraph {
    let mut edges = collapsed_edges(intel);
    let limit = if top_n == 0 { DEFAULT_TOP_N } else { top_n };
    edges.truncate(limit);

    let mut node_stats: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    for edge in &edges {
        let a = node_stats.entry(edge.a.clone()).or_default();
        a.0 += 1;
        a.1 += edge.strength;

        let b = node_stats.entry(edge.b.clone()).or_default();
        b.0 += 1;
        b.1 += edge.strength;
    }

    let nodes = node_stats
        .into_iter()
        .map(|(path, (degree, total_strength))| CouplingNode {
            path,
            degree,
            total_strength,
        })
        .collect();

    CouplingGraph { edges, nodes }
}

pub fn hidden_coupling(
    intel: &GitIntel,
    import_pairs: &HashSet<(String, String)>,
    min_co: u32,
) -> Vec<CouplingEdge> {
    collapsed_edges(intel)
        .into_iter()
        .filter(|edge| edge.co_changes >= min_co)
        .filter(|edge| !import_pairs.contains(&(edge.a.clone(), edge.b.clone())))
        .collect()
}

pub fn reviewer_suggestions(
    intel: &GitIntel,
    changed_files: &[String],
    top_n: usize,
) -> Vec<(String, f64)> {
    let changed: BTreeSet<String> = changed_files.iter().cloned().collect();
    if changed.is_empty() {
        return Vec::new();
    }

    let max_co_changes = max_collapsed_co_changes(intel);
    let mut scores: BTreeMap<String, f64> = BTreeMap::new();

    for file in &changed {
        add_owner_shares(intel, file, 1.0, &mut scores);
    }

    for edge in collapsed_edges(intel) {
        let neighbor = if changed.contains(&edge.a) && !changed.contains(&edge.b) {
            Some(edge.b.as_str())
        } else if changed.contains(&edge.b) && !changed.contains(&edge.a) {
            Some(edge.a.as_str())
        } else {
            None
        };

        if let Some(path) = neighbor {
            let weight = if max_co_changes == 0 {
                0.0
            } else {
                edge.co_changes as f64 / max_co_changes as f64
            };
            add_owner_shares(intel, path, weight, &mut scores);
        }
    }

    let mut ranked: Vec<(String, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(top_n);
    ranked
}

fn collapsed_edges(intel: &GitIntel) -> Vec<CouplingEdge> {
    let collapsed = collapsed_counts(intel);
    let max_co_changes = collapsed.values().copied().max().unwrap_or(0);
    let mut edges: Vec<CouplingEdge> = collapsed
        .into_iter()
        .map(|((a, b), co_changes)| CouplingEdge {
            a,
            b,
            strength: if max_co_changes == 0 {
                0.0
            } else {
                co_changes as f64 / max_co_changes as f64
            },
            co_changes,
        })
        .collect();

    edges.sort_by(|x, y| {
        y.strength
            .total_cmp(&x.strength)
            .then_with(|| x.a.cmp(&y.a))
            .then_with(|| x.b.cmp(&y.b))
    });
    edges
}

fn collapsed_counts(intel: &GitIntel) -> BTreeMap<(String, String), u32> {
    let mut collapsed: BTreeMap<(String, String), u32> = BTreeMap::new();
    for ((left, right), count) in &intel.co_change {
        if left == right || *count == 0 {
            continue;
        }
        let (a, b) = sorted_pair(left, right);
        *collapsed.entry((a, b)).or_default() += *count;
    }
    collapsed
}

fn max_collapsed_co_changes(intel: &GitIntel) -> u32 {
    collapsed_counts(intel).values().copied().max().unwrap_or(0)
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn add_owner_shares(intel: &GitIntel, path: &str, weight: f64, scores: &mut BTreeMap<String, f64>) {
    if weight == 0.0 {
        return;
    }

    let Some(authors) = intel.file_authors.get(path) else {
        return;
    };
    let total: u32 = authors.values().sum();
    if total == 0 {
        return;
    }

    for (author, commits) in authors {
        if *commits == 0 {
            continue;
        }
        *scores.entry(author.clone()).or_default() += weight * (*commits as f64 / total as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn authors(entries: &[(&str, u32)]) -> HashMap<String, u32> {
        entries
            .iter()
            .map(|(author, commits)| ((*author).to_string(), *commits))
            .collect()
    }

    #[test]
    fn build_coupling_graph_collapses_undirected_pairs_and_ranks_highest_first() {
        let mut intel = GitIntel::default();
        intel.co_change.insert(("b.rs".into(), "a.rs".into()), 2);
        intel.co_change.insert(("a.rs".into(), "b.rs".into()), 3);
        intel.co_change.insert(("a.rs".into(), "c.rs".into()), 4);

        let graph = build_coupling_graph(&intel, 200);

        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].a, "a.rs");
        assert_eq!(graph.edges[0].b, "b.rs");
        assert_eq!(graph.edges[0].co_changes, 5);
        assert!((graph.edges[0].strength - 1.0).abs() < 1e-9);
        assert_eq!(graph.edges[1].co_changes, 4);

        let a_node = graph.nodes.iter().find(|node| node.path == "a.rs").unwrap();
        assert_eq!(a_node.degree, 2);
        assert!((a_node.total_strength - 1.8).abs() < 1e-9);
    }

    #[test]
    fn hidden_coupling_excludes_imported_pairs_and_includes_absent_high_co_change() {
        let mut intel = GitIntel::default();
        intel.co_change.insert(("a.rs".into(), "b.rs".into()), 5);
        intel.co_change.insert(("c.rs".into(), "d.rs".into()), 4);
        intel.co_change.insert(("e.rs".into(), "f.rs".into()), 1);
        let import_pairs = HashSet::from([("a.rs".to_string(), "b.rs".to_string())]);

        let hidden = hidden_coupling(&intel, &import_pairs, 3);

        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].a, "c.rs");
        assert_eq!(hidden[0].b, "d.rs");
        assert_eq!(hidden[0].co_changes, 4);
    }

    #[test]
    fn reviewer_suggestions_returns_dominant_changed_file_owner_first() {
        let mut intel = GitIntel::default();
        intel.file_authors.insert(
            "changed.rs".into(),
            authors(&[("alice@example.com", 8), ("bob@example.com", 2)]),
        );
        intel
            .file_authors
            .insert("neighbor.rs".into(), authors(&[("bob@example.com", 3)]));
        intel
            .co_change
            .insert(("changed.rs".into(), "neighbor.rs".into()), 1);
        intel.co_change.insert(("x.rs".into(), "y.rs".into()), 10);

        let suggestions = reviewer_suggestions(&intel, &["changed.rs".into()], 2);

        assert_eq!(suggestions[0].0, "alice@example.com");
        assert!(suggestions[0].1 > suggestions[1].1);
    }
}
