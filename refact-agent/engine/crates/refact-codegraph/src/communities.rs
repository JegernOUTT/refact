use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::analytics::GraphData;
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Community {
    pub id: usize,
    pub members: Vec<i64>,
    pub label: String,
    pub cohesion: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecFlow {
    pub entry: String,
    pub entry_id: i64,
    pub reached: usize,
    pub depth: usize,
    pub nodes: Vec<i64>,
}

#[derive(Debug, Clone)]
struct LouvainGraph {
    members: Vec<Vec<i64>>,
    adj: Vec<BTreeMap<usize, f64>>,
}

impl LouvainGraph {
    fn total_edge_weight(&self) -> f64 {
        let mut total = 0.0;
        for (i, ns) in self.adj.iter().enumerate() {
            for (&j, &w) in ns {
                if i < j {
                    total += w;
                } else if i == j {
                    total += w;
                }
            }
        }
        total
    }

    fn degrees(&self) -> Vec<f64> {
        self.adj
            .iter()
            .enumerate()
            .map(|(i, ns)| {
                ns.iter()
                    .map(|(&j, &w)| if i == j { 2.0 * w } else { w })
                    .sum()
            })
            .collect()
    }
}

pub fn detect_communities(store: &Store) -> Result<Vec<Community>, String> {
    detect_communities_from_data(&GraphData::from_store(store)?)
}

pub fn detect_communities_from_data(data: &GraphData) -> Result<Vec<Community>, String> {
    let nodes = data.nodes.clone();
    if nodes.is_empty() {
        return Ok(Vec::new());
    }

    let mut ids: Vec<i64> = nodes.iter().map(|(id, _, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    let id_to_pos: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let paths: HashMap<i64, String> = nodes
        .into_iter()
        .map(|(id, _name, path)| (id, path))
        .collect();

    let mut graph = LouvainGraph {
        members: ids.iter().map(|id| vec![*id]).collect(),
        adj: vec![BTreeMap::new(); ids.len()],
    };
    for (src, dst, kind) in &data.edges {
        if kind != "calls" && kind != "inherits" {
            continue;
        }
        let (Some(&a), Some(&b)) = (id_to_pos.get(src), id_to_pos.get(dst)) else {
            continue;
        };
        if a == b {
            *graph.adj[a].entry(a).or_insert(0.0) += 1.0;
        } else {
            *graph.adj[a].entry(b).or_insert(0.0) += 1.0;
            *graph.adj[b].entry(a).or_insert(0.0) += 1.0;
        }
    }

    if graph.total_edge_weight() == 0.0 {
        return Ok(ids
            .into_iter()
            .enumerate()
            .map(|(id, node_id)| Community {
                id,
                members: vec![node_id],
                label: label_for(id, &[node_id], &paths),
                cohesion: 0.0,
            })
            .collect());
    }

    let original_adj = graph.adj.clone();
    let mut best_modularity = modularity(&graph, &(0..graph.members.len()).collect::<Vec<_>>());

    for _level in 0..10 {
        let partition = local_moving(&graph);
        let q = modularity(&graph, &partition);
        if q <= best_modularity + 1e-9 {
            break;
        }
        best_modularity = q;

        let aggregated = aggregate_graph(&graph, &partition);
        if aggregated.members.len() == graph.members.len() {
            graph = aggregated;
            break;
        }
        graph = aggregated;
    }

    let mut communities: Vec<Vec<i64>> = graph.members;
    communities.iter_mut().for_each(|m| m.sort_unstable());
    communities.sort_by_key(|m| m.first().copied().unwrap_or(i64::MAX));

    Ok(communities
        .into_iter()
        .enumerate()
        .map(|(id, members)| Community {
            id,
            label: label_for(id, &members, &paths),
            cohesion: cohesion_for(&members, &id_to_pos, &original_adj),
            members,
        })
        .collect())
}

fn local_moving(graph: &LouvainGraph) -> Vec<usize> {
    let n = graph.members.len();
    let m = graph.total_edge_weight();
    if n == 0 || m == 0.0 {
        return (0..n).collect();
    }

    let degree = graph.degrees();
    let mut comm: Vec<usize> = (0..n).collect();
    let mut totals = degree.clone();

    for _pass in 0..20 {
        let mut changed = false;
        for node in 0..n {
            let old = comm[node];
            totals[old] -= degree[node];

            let mut candidate_weights: BTreeMap<usize, f64> = BTreeMap::new();
            candidate_weights.insert(old, 0.0);
            for (&neighbor, &w) in &graph.adj[node] {
                if neighbor == node {
                    continue;
                }
                *candidate_weights.entry(comm[neighbor]).or_insert(0.0) += w;
            }

            let mut best_comm = old;
            let mut best_gain = 0.0;
            for (&candidate, &k_i_in) in &candidate_weights {
                let gain = (k_i_in / m) - (totals[candidate] * degree[node]) / (2.0 * m * m);
                if gain > best_gain + 1e-12
                    || ((gain - best_gain).abs() <= 1e-12 && candidate < best_comm)
                {
                    best_gain = gain;
                    best_comm = candidate;
                }
            }

            totals[best_comm] += degree[node];
            if best_comm != old {
                comm[node] = best_comm;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    renumber_partition(&comm)
}

fn renumber_partition(partition: &[usize]) -> Vec<usize> {
    let mut map = BTreeMap::new();
    let mut next = 0usize;
    partition
        .iter()
        .map(|comm| {
            *map.entry(*comm).or_insert_with(|| {
                let id = next;
                next += 1;
                id
            })
        })
        .collect()
}

fn aggregate_graph(graph: &LouvainGraph, partition: &[usize]) -> LouvainGraph {
    let comm_count = partition.iter().copied().max().map(|v| v + 1).unwrap_or(0);
    let mut members = vec![Vec::new(); comm_count];
    for (node, &comm) in partition.iter().enumerate() {
        members[comm].extend(graph.members[node].iter().copied());
    }
    for m in &mut members {
        m.sort_unstable();
    }

    let mut adj = vec![BTreeMap::new(); comm_count];
    for (i, ns) in graph.adj.iter().enumerate() {
        for (&j, &w) in ns {
            if i > j {
                continue;
            }
            let a = partition[i];
            let b = partition[j];
            if a == b {
                *adj[a].entry(a).or_insert(0.0) += w;
            } else {
                *adj[a].entry(b).or_insert(0.0) += w;
                *adj[b].entry(a).or_insert(0.0) += w;
            }
        }
    }

    LouvainGraph { members, adj }
}

fn modularity(graph: &LouvainGraph, partition: &[usize]) -> f64 {
    let m = graph.total_edge_weight();
    if m == 0.0 {
        return 0.0;
    }
    let degree = graph.degrees();
    let mut internal = BTreeMap::<usize, f64>::new();
    let mut totals = BTreeMap::<usize, f64>::new();
    for (i, &comm) in partition.iter().enumerate() {
        *totals.entry(comm).or_insert(0.0) += degree[i];
    }
    for (i, ns) in graph.adj.iter().enumerate() {
        for (&j, &w) in ns {
            if i <= j && partition[i] == partition[j] {
                *internal.entry(partition[i]).or_insert(0.0) += w;
            }
        }
    }
    totals
        .into_iter()
        .map(|(comm, total)| {
            internal.get(&comm).copied().unwrap_or(0.0) / m - (total / (2.0 * m)).powi(2)
        })
        .sum()
}

pub(crate) fn cohesion_for(
    members: &[i64],
    id_to_pos: &HashMap<i64, usize>,
    adj: &[BTreeMap<usize, f64>],
) -> f64 {
    let member_pos: HashSet<usize> = members
        .iter()
        .filter_map(|id| id_to_pos.get(id).copied())
        .collect();
    if member_pos.is_empty() {
        return 0.0;
    }

    let mut internal = 0.0;
    for &i in &member_pos {
        for (&j, &w) in &adj[i] {
            if member_pos.contains(&j) && i < j {
                internal += w;
            }
        }
    }
    let possible = member_pos
        .len()
        .saturating_mul(member_pos.len().saturating_sub(1)) as f64
        / 2.0;
    if possible == 0.0 {
        0.0
    } else {
        (internal / possible).clamp(0.0, 1.0)
    }
}

fn label_for(id: usize, members: &[i64], paths: &HashMap<i64, String>) -> String {
    let mut counts = BTreeMap::<String, usize>::new();
    for node_id in members {
        if let Some(path) = paths.get(node_id) {
            for segment in path.split(&['/', ':'][..]).filter(|s| !s.is_empty()) {
                *counts.entry(segment.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(segment, _)| segment)
        .unwrap_or_else(|| format!("community-{id}"))
}

pub fn execution_flows(store: &Store, max_flows: usize) -> Result<Vec<ExecFlow>, String> {
    execution_flows_from_data(&GraphData::from_store(store)?, max_flows)
}

pub fn execution_flows_from_data(
    data: &GraphData,
    max_flows: usize,
) -> Result<Vec<ExecFlow>, String> {
    if max_flows == 0 {
        return Ok(Vec::new());
    }

    let nodes = data.nodes.clone();
    if nodes.is_empty() {
        return Ok(Vec::new());
    }

    let names: HashMap<i64, String> = nodes
        .iter()
        .map(|(id, name, _path)| (*id, name.clone()))
        .collect();
    let known_ids: HashSet<i64> = names.keys().copied().collect();
    let mut out = BTreeMap::<i64, Vec<i64>>::new();
    let mut indeg = BTreeMap::<i64, usize>::new();
    let mut outdeg = BTreeMap::<i64, usize>::new();
    let mut entries = BTreeSet::<i64>::new();

    for (src, dst, kind) in &data.edges {
        if kind != "calls" && kind != "route_handler" {
            continue;
        }
        if !known_ids.contains(src) || !known_ids.contains(dst) {
            continue;
        }
        out.entry(*src).or_default().push(*dst);
        *outdeg.entry(*src).or_insert(0) += 1;
        *indeg.entry(*dst).or_insert(0) += 1;
        if kind == "route_handler" {
            entries.insert(*dst);
        }
    }
    for targets in out.values_mut() {
        targets.sort_unstable();
        targets.dedup();
    }

    for id in names.keys().copied() {
        let name = names.get(&id).map(String::as_str).unwrap_or_default();
        if (indeg.get(&id).copied().unwrap_or(0) == 0 && outdeg.get(&id).copied().unwrap_or(0) > 0)
            || matches!(name, "main" | "run" | "handler")
        {
            entries.insert(id);
        }
    }

    let mut flows = Vec::new();
    for entry_id in entries {
        let (nodes, depth) = bfs_flow(entry_id, &out, 500);
        let reached = nodes.len().saturating_sub(1);
        flows.push(ExecFlow {
            entry: names.get(&entry_id).cloned().unwrap_or_default(),
            entry_id,
            reached,
            depth,
            nodes,
        });
    }

    flows.sort_by(|a, b| {
        b.reached
            .cmp(&a.reached)
            .then_with(|| a.entry.cmp(&b.entry))
            .then_with(|| a.entry_id.cmp(&b.entry_id))
    });
    flows.truncate(max_flows);
    Ok(flows)
}

fn bfs_flow(entry_id: i64, out: &BTreeMap<i64, Vec<i64>>, cap: usize) -> (Vec<i64>, usize) {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    let mut queue = VecDeque::new();
    let mut max_depth = 0usize;

    seen.insert(entry_id);
    order.push(entry_id);
    queue.push_back((entry_id, 0usize));

    while let Some((node, depth)) = queue.pop_front() {
        max_depth = max_depth.max(depth);
        if seen.len() >= cap {
            continue;
        }
        if let Some(targets) = out.get(&node) {
            for &target in targets {
                if seen.len() >= cap {
                    break;
                }
                if seen.insert(target) {
                    order.push(target);
                    queue.push_back((target, depth + 1));
                }
            }
        }
    }

    (order, max_depth)
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
    fn detects_two_tightly_connected_clusters() {
        let store =
            store_with("fn a() { b(); }\nfn b() { a(); }\nfn x() { y(); }\nfn y() { x(); }\n");
        let communities = detect_communities(&store).unwrap();
        let multi_member = communities.iter().filter(|c| c.members.len() >= 2).count();
        assert!(
            communities.len() >= 2 && multi_member >= 2,
            "expected at least two communities, got {:?}",
            communities
        );
    }

    #[test]
    fn self_loops_do_not_change_cluster_assignment() {
        let store = store_with(
            "fn a() { b(); a(); }\nfn b() { a(); }\nfn x() { y(); x(); }\nfn y() { x(); }\n",
        );
        let communities = detect_communities(&store).unwrap();
        let clusters: Vec<&Vec<i64>> = communities
            .iter()
            .filter(|c| c.members.len() >= 2)
            .map(|c| &c.members)
            .collect();
        assert!(
            clusters.len() >= 2,
            "self-loops must not merge or dissolve clusters, got {:?}",
            communities
        );
    }

    #[test]
    fn execution_flow_follows_main_chain() {
        let store = store_with("fn main() { a(); }\nfn a() { b(); }\nfn b() { c(); }\nfn c() {}\n");
        let flows = execution_flows(&store, 10).unwrap();
        let main = flows
            .iter()
            .find(|flow| flow.entry == "main")
            .expect("main flow should be present");
        assert!(main.reached >= 3, "got {:?}", flows);
        assert!(main.depth >= 3, "got {:?}", flows);
    }

    #[test]
    fn empty_graph_is_safe() {
        let store = Store::open_in_memory().unwrap();
        assert!(detect_communities(&store).unwrap().is_empty());
        assert!(execution_flows(&store, 10).unwrap().is_empty());
    }

    #[test]
    fn community_cohesion_below_one_for_loose_community() {
        let mut adj = vec![BTreeMap::new(); 4];
        for (a, b) in [(0usize, 1usize), (1, 2), (2, 3)] {
            adj[a].insert(b, 1.0);
            adj[b].insert(a, 1.0);
        }
        let id_to_pos = (1_i64..=4_i64)
            .enumerate()
            .map(|(pos, id)| (id, pos))
            .collect::<HashMap<_, _>>();

        let cohesion = cohesion_for(&[1, 2, 3, 4], &id_to_pos, &adj);

        assert!((cohesion - 0.5).abs() < f64::EPSILON, "{cohesion}");
    }
}
