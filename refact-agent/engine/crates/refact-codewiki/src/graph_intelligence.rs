use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub node_type: String,
    pub file_path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    pub src: String,
    pub dst: String,
    pub edge_type: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallEntry {
    pub caller: String,
    pub callee: String,
    pub callee_file: String,
    pub caller_file: String,
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeritageEntry {
    pub child: String,
    pub parent: String,
    pub kind: String,
    pub parent_file: String,
    pub child_file: String,
}

pub fn build_symbol_index(graph: &CodeGraph) -> HashMap<String, Vec<usize>> {
    let mut index = HashMap::new();

    for (node_index, node) in graph.nodes.iter().enumerate() {
        if node.node_type == "symbol" && !node.file_path.is_empty() {
            index
                .entry(node.file_path.clone())
                .or_insert_with(Vec::new)
                .push(node_index);
        }
    }

    index
}

pub fn extract_call_graph(file_path: &str, graph: &CodeGraph) -> Vec<CallEntry> {
    let node_files = build_node_file_lookup(graph);
    let mut calls = Vec::new();
    let mut seen = HashMap::new();

    for node in graph
        .nodes
        .iter()
        .filter(|node| node.node_type == "symbol" && node.file_path == file_path)
    {
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.edge_type == "calls" && edge.src == node.id)
        {
            let entry = CallEntry {
                caller: node.id.clone(),
                callee: edge.dst.clone(),
                callee_file: node_files.get(&edge.dst).cloned().unwrap_or_default(),
                caller_file: file_path.to_string(),
                confidence: 1.0,
            };
            push_unique_call(&mut calls, &mut seen, entry);
            if calls.len() >= 15 {
                return calls;
            }
        }

        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.edge_type == "calls" && edge.dst == node.id)
        {
            let entry = CallEntry {
                caller: edge.src.clone(),
                callee: node.id.clone(),
                callee_file: file_path.to_string(),
                caller_file: node_files.get(&edge.src).cloned().unwrap_or_default(),
                confidence: 1.0,
            };
            push_unique_call(&mut calls, &mut seen, entry);
            if calls.len() >= 15 {
                return calls;
            }
        }
    }

    calls
}

pub fn extract_heritage(file_path: &str, graph: &CodeGraph) -> Vec<HeritageEntry> {
    let node_files = build_node_file_lookup(graph);
    let mut heritage = Vec::new();

    for node in graph
        .nodes
        .iter()
        .filter(|node| node.node_type == "symbol" && node.file_path == file_path)
    {
        for edge in graph
            .edges
            .iter()
            .filter(|edge| is_heritage_edge(edge) && edge.src == node.id)
        {
            heritage.push(HeritageEntry {
                child: node.id.clone(),
                parent: edge.dst.clone(),
                kind: edge.edge_type.clone(),
                parent_file: node_files.get(&edge.dst).cloned().unwrap_or_default(),
                child_file: file_path.to_string(),
            });
            if heritage.len() >= 10 {
                return heritage;
            }
        }

        for edge in graph
            .edges
            .iter()
            .filter(|edge| is_heritage_edge(edge) && edge.dst == node.id)
        {
            heritage.push(HeritageEntry {
                child: edge.src.clone(),
                parent: node.id.clone(),
                kind: edge.edge_type.clone(),
                parent_file: file_path.to_string(),
                child_file: node_files.get(&edge.src).cloned().unwrap_or_default(),
            });
            if heritage.len() >= 10 {
                return heritage;
            }
        }
    }

    heritage
}

pub fn extract_community_meta(file_path: &str, community_meta_json: Option<&str>) -> (String, f64) {
    let _ = file_path;
    let Some(json) = community_meta_json else {
        return (String::new(), 0.0);
    };

    match (
        extract_json_string(json, "label"),
        extract_json_number(json, "cohesion"),
    ) {
        (Some(label), Some(cohesion)) => (label, cohesion),
        _ => (String::new(), 0.0),
    }
}

fn build_node_file_lookup(graph: &CodeGraph) -> HashMap<String, String> {
    let mut lookup = HashMap::new();

    for node in &graph.nodes {
        lookup
            .entry(node.id.clone())
            .or_insert(node.file_path.clone());
    }

    lookup
}

fn push_unique_call(calls: &mut Vec<CallEntry>, seen: &mut HashMap<String, ()>, entry: CallEntry) {
    let key = format!("{}->{}", entry.caller, entry.callee);
    if !seen.contains_key(&key) {
        seen.insert(key, ());
        calls.push(entry);
    }
}

fn is_heritage_edge(edge: &GraphEdge) -> bool {
    edge.edge_type == "extends" || edge.edge_type == "implements"
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let mut index = find_key_value_start(json, key)?;
    let bytes = json.as_bytes();

    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    index += 1;

    let mut value = String::new();
    let mut escaped = false;

    for ch in json[index..].chars() {
        if escaped {
            match ch {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                '/' => value.push('/'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                'b' => value.push('\u{0008}'),
                'f' => value.push('\u{000c}'),
                other => value.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(value);
        } else {
            value.push(ch);
        }
    }

    None
}

fn extract_json_number(json: &str, key: &str) -> Option<f64> {
    let mut index = find_key_value_start(json, key)?;
    let bytes = json.as_bytes();

    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let start = index;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit()
            || byte == b'-'
            || byte == b'+'
            || byte == b'.'
            || byte == b'e'
            || byte == b'E'
        {
            index += 1;
        } else {
            break;
        }
    }

    if start == index {
        return None;
    }

    json[start..index].parse::<f64>().ok()
}

fn find_key_value_start(json: &str, key: &str) -> Option<usize> {
    let key_pattern = format!("\"{}\"", key);
    let key_start = json.find(&key_pattern)?;
    let after_key = key_start + key_pattern.len();
    let colon_offset = json[after_key..].find(':')?;
    Some(after_key + colon_offset + 1)
}
