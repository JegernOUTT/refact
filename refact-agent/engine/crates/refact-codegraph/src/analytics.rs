use std::collections::{HashMap, VecDeque};

use petgraph::algo::{connected_components, tarjan_scc};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;
use serde::{Deserialize, Serialize};

use crate::store::Store;

const BETWEENNESS_EXACT_NODE_LIMIT: usize = 1024;
const BETWEENNESS_SAMPLE_LIMIT: usize = 512;

pub type GraphNode = (i64, String, String);
pub type GraphEdge = (i64, i64, String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl GraphData {
    pub fn from_store(store: &Store) -> Result<Self, String> {
        Ok(Self {
            nodes: store.node_names()?,
            edges: store.graph_edges()?,
        })
    }
}

pub fn build_petgraph(
    store: &Store,
) -> Result<(DiGraph<i64, ()>, HashMap<i64, NodeIndex>), String> {
    let data = GraphData::from_store(store)?;
    Ok(build_petgraph_from_data(&data))
}

pub fn build_petgraph_from_data(data: &GraphData) -> (DiGraph<i64, ()>, HashMap<i64, NodeIndex>) {
    let mut g: DiGraph<i64, ()> = DiGraph::new();
    let mut idx: HashMap<i64, NodeIndex> = HashMap::new();
    for (id, _name, _path) in &data.nodes {
        idx.entry(*id).or_insert_with(|| g.add_node(*id));
    }
    for (src, dst, _kind) in &data.edges {
        let s = *idx.entry(*src).or_insert_with(|| g.add_node(*src));
        let d = *idx.entry(*dst).or_insert_with(|| g.add_node(*dst));
        g.add_edge(s, d, ());
    }
    (g, idx)
}

pub fn pagerank(g: &DiGraph<i64, ()>, damping: f64, iters: usize) -> HashMap<NodeIndex, f64> {
    let n = g.node_count();
    if n == 0 {
        return HashMap::new();
    }
    let nf = n as f64;
    let base = 1.0 / nf;
    let mut rank: HashMap<NodeIndex, f64> = g.node_indices().map(|i| (i, base)).collect();
    let outdeg: HashMap<NodeIndex, usize> = g
        .node_indices()
        .map(|i| (i, g.neighbors_directed(i, Direction::Outgoing).count()))
        .collect();

    for _ in 0..iters {
        let dangling: f64 = g
            .node_indices()
            .filter(|i| outdeg[i] == 0)
            .map(|i| rank[&i])
            .sum();
        let mut next: HashMap<NodeIndex, f64> = HashMap::with_capacity(n);
        for v in g.node_indices() {
            let mut incoming = 0.0;
            for u in g.neighbors_directed(v, Direction::Incoming) {
                let od = outdeg[&u];
                if od > 0 {
                    incoming += rank[&u] / od as f64;
                }
            }
            let score = (1.0 - damping) / nf + damping * (dangling / nf + incoming);
            next.insert(v, score);
        }
        rank = next;
    }
    rank
}

pub fn betweenness_centrality(g: &DiGraph<i64, ()>) -> HashMap<NodeIndex, f64> {
    let sources: Vec<NodeIndex> = g.node_indices().collect();
    betweenness_centrality_for_sources(g, &sources, 1.0)
}

fn analytics_betweenness_centrality(g: &DiGraph<i64, ()>) -> HashMap<NodeIndex, f64> {
    let nodes: Vec<NodeIndex> = g.node_indices().collect();
    if nodes.len() <= BETWEENNESS_EXACT_NODE_LIMIT {
        return betweenness_centrality_for_sources(g, &nodes, 1.0);
    }
    let stride = nodes.len().div_ceil(BETWEENNESS_SAMPLE_LIMIT);
    let sources: Vec<NodeIndex> = nodes
        .into_iter()
        .step_by(stride.max(1))
        .take(BETWEENNESS_SAMPLE_LIMIT)
        .collect();
    if sources.is_empty() {
        return HashMap::new();
    }
    let scale = g.node_count() as f64 / sources.len() as f64;
    betweenness_centrality_for_sources(g, &sources, scale)
}

fn betweenness_centrality_for_sources(
    g: &DiGraph<i64, ()>,
    sources: &[NodeIndex],
    scale: f64,
) -> HashMap<NodeIndex, f64> {
    let mut cb: HashMap<NodeIndex, f64> = g.node_indices().map(|i| (i, 0.0)).collect();
    for &s in sources {
        let mut stack: Vec<NodeIndex> = Vec::new();
        let mut pred: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();
        let mut sigma: HashMap<NodeIndex, f64> = g.node_indices().map(|i| (i, 0.0)).collect();
        let mut dist: HashMap<NodeIndex, i64> = g.node_indices().map(|i| (i, -1)).collect();
        sigma.insert(s, 1.0);
        dist.insert(s, 0);
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        queue.push_back(s);
        while let Some(v) = queue.pop_front() {
            stack.push(v);
            for w in g.neighbors_directed(v, Direction::Outgoing) {
                if dist[&w] < 0 {
                    queue.push_back(w);
                    dist.insert(w, dist[&v] + 1);
                }
                if dist[&w] == dist[&v] + 1 {
                    *sigma.get_mut(&w).unwrap() += sigma[&v];
                    pred.entry(w).or_default().push(v);
                }
            }
        }
        let mut delta: HashMap<NodeIndex, f64> = g.node_indices().map(|i| (i, 0.0)).collect();
        while let Some(w) = stack.pop() {
            if let Some(ps) = pred.get(&w) {
                for &v in ps {
                    if sigma[&w] > 0.0 {
                        let contrib = (sigma[&v] / sigma[&w]) * (1.0 + delta[&w]);
                        *delta.get_mut(&v).unwrap() += contrib;
                    }
                }
            }
            if w != s {
                *cb.get_mut(&w).unwrap() += delta[&w] * scale;
            }
        }
    }
    cb
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolScore {
    pub symbol: String,
    pub path: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphOverview {
    pub node_count: usize,
    pub edge_count: usize,
    pub scc_count: usize,
    pub largest_scc: usize,
    pub component_count: usize,
    pub top_pagerank: Vec<SymbolScore>,
    pub top_betweenness: Vec<SymbolScore>,
}

impl GraphOverview {
    pub fn truncated(&self, top_n: usize) -> Self {
        let mut overview = self.clone();
        overview.top_pagerank.truncate(top_n);
        overview.top_betweenness.truncate(top_n);
        overview
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileCentrality {
    pub top_pagerank: Vec<(String, f64)>,
    pub top_betweenness: Vec<(String, f64)>,
}

impl FileCentrality {
    pub fn truncated(&self, top_n: usize) -> Self {
        let mut centrality = self.clone();
        centrality.top_pagerank.truncate(top_n);
        centrality.top_betweenness.truncate(top_n);
        centrality
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeCentrality {
    pub top_pagerank: Vec<(i64, f64)>,
    pub top_betweenness: Vec<(i64, f64)>,
}

impl NodeCentrality {
    pub fn truncated(&self, top_n: usize) -> Self {
        let mut centrality = self.clone();
        centrality.top_pagerank.truncate(top_n);
        centrality.top_betweenness.truncate(top_n);
        centrality
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphAnalytics {
    pub overview: GraphOverview,
    pub file_centrality: FileCentrality,
    pub node_centrality: NodeCentrality,
}

pub fn compute_graph_analytics(store: &Store) -> Result<GraphAnalytics, String> {
    let data = GraphData::from_store(store)?;
    Ok(compute_graph_analytics_from_data(&data))
}

pub fn compute_graph_analytics_from_data(data: &GraphData) -> GraphAnalytics {
    let (g, _idx) = build_petgraph_from_data(data);
    let names: HashMap<i64, String> = data
        .nodes
        .iter()
        .map(|(id, name, _)| (*id, name.clone()))
        .collect();
    let paths: HashMap<i64, String> = data
        .nodes
        .iter()
        .map(|(id, _name, path)| (*id, path.clone()))
        .collect();

    let pr = pagerank(&g, 0.85, 40);
    let bc = analytics_betweenness_centrality(&g);
    let sccs = tarjan_scc(&g);
    let largest_scc = sccs.iter().map(|c| c.len()).max().unwrap_or(0);
    let component_count = connected_components(&g);

    let rank_symbols = |m: &HashMap<NodeIndex, f64>| -> Vec<SymbolScore> {
        let mut scored: Vec<(i64, SymbolScore)> = m
            .iter()
            .map(|(ni, score)| {
                let node_id = g[*ni];
                (
                    node_id,
                    SymbolScore {
                        symbol: names.get(&node_id).cloned().unwrap_or_default(),
                        path: paths.get(&node_id).cloned().unwrap_or_default(),
                        score: *score,
                    },
                )
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.score
                .total_cmp(&a.1.score)
                .then_with(|| a.1.symbol.cmp(&b.1.symbol))
                .then_with(|| a.1.path.cmp(&b.1.path))
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.into_iter().map(|(_, score)| score).collect()
    };

    let rank_nodes = |m: &HashMap<NodeIndex, f64>| -> Vec<(i64, f64)> {
        let mut scored: Vec<(i64, f64)> = m.iter().map(|(ni, score)| (g[*ni], *score)).collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored
    };

    let rank_files = |m: &HashMap<NodeIndex, f64>| -> Vec<(String, f64)> {
        let mut by_path: HashMap<String, f64> = HashMap::new();
        for (ni, score) in m {
            let node_id = g[*ni];
            let Some(path) = paths.get(&node_id) else {
                continue;
            };
            if path.is_empty() {
                continue;
            }
            *by_path.entry(path.clone()).or_insert(0.0) += *score;
        }
        let mut scored: Vec<(String, f64)> = by_path.into_iter().collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored
    };

    GraphAnalytics {
        overview: GraphOverview {
            node_count: g.node_count(),
            edge_count: g.edge_count(),
            scc_count: sccs.len(),
            largest_scc,
            component_count,
            top_pagerank: rank_symbols(&pr),
            top_betweenness: rank_symbols(&bc),
        },
        file_centrality: FileCentrality {
            top_pagerank: rank_files(&pr),
            top_betweenness: rank_files(&bc),
        },
        node_centrality: NodeCentrality {
            top_pagerank: rank_nodes(&pr),
            top_betweenness: rank_nodes(&bc),
        },
    }
}

pub fn per_file_centrality(store: &Store, top_n: usize) -> Result<FileCentrality, String> {
    Ok(compute_graph_analytics(store)?
        .file_centrality
        .truncated(top_n))
}

pub fn compute_overview(store: &Store, top_n: usize) -> Result<GraphOverview, String> {
    Ok(compute_graph_analytics(store)?.overview.truncated(top_n))
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
    fn pagerank_ranks_most_called_symbol_highest() {
        let src = "\
fn a() { helper(); }
fn b() { helper(); }
fn c() { helper(); a(); }
fn helper() {}
";
        let store = store_with(src);
        let overview = compute_overview(&store, 10).unwrap();
        assert!(overview.node_count >= 4, "got {}", overview.node_count);
        assert!(overview.edge_count >= 4, "got {}", overview.edge_count);
        let top = &overview.top_pagerank[0];
        assert_eq!(
            top.symbol, "helper",
            "most-called symbol should rank first: {:?}",
            overview.top_pagerank
        );
    }

    #[test]
    fn scc_detects_cycle() {
        let src = "\
fn x() { y(); }
fn y() { x(); }
";
        let store = store_with(src);
        let overview = compute_overview(&store, 10).unwrap();
        assert!(
            overview.largest_scc >= 2,
            "x<->y cycle should form an SCC of size >=2, got {}",
            overview.largest_scc
        );
    }

    #[test]
    fn betweenness_identifies_bridge_node() {
        let src = "fn a() { b(); }\nfn b() { c(); }\nfn c() {}\n";
        let store = store_with(src);
        let overview = compute_overview(&store, 10).unwrap();
        assert!(!overview.top_betweenness.is_empty());
        assert_eq!(
            overview.top_betweenness[0].symbol, "b",
            "b is the bridge between a and c: {:?}",
            overview.top_betweenness
        );
    }

    #[test]
    fn empty_graph_overview_is_safe() {
        let store = Store::open_in_memory().unwrap();
        let overview = compute_overview(&store, 10).unwrap();
        assert_eq!(overview.node_count, 0);
        assert_eq!(overview.edge_count, 0);
        assert_eq!(overview.scc_count, 0);
        assert!(overview.top_pagerank.is_empty());
    }

    #[test]
    fn score_entries_carry_paths() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph("src/b.rs", "fn run() { helper(); }\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();

        let overview = compute_overview(&store, 10).unwrap();

        assert!(overview
            .top_pagerank
            .iter()
            .any(|entry| entry.symbol == "helper" && entry.path == "src/a.rs"));
        assert!(overview
            .top_pagerank
            .iter()
            .all(|entry| !entry.path.is_empty()));
    }

    #[test]
    fn per_file_centrality_returns_path_keyed_entries() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph("src/b.rs", "fn run() { helper(); }\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();

        let centrality = per_file_centrality(&store, 10).unwrap();
        assert!(centrality
            .top_pagerank
            .iter()
            .any(|(path, _)| path == "src/a.rs"));
        assert!(centrality
            .top_pagerank
            .iter()
            .any(|(path, _)| path == "src/b.rs"));
        assert!(centrality
            .top_betweenness
            .iter()
            .all(|(path, _)| path.starts_with("src/")));
    }
}
