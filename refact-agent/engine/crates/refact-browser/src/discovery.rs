//! Search the real ARIA snapshot, never mint refs from arbitrary matching text.
use super::*;
use regex::RegexBuilder;

const MAX_BYTES: usize = 256 * 1024;
const MAX_NODES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotSearchOptions {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub subtree: Option<String>,
    pub max_results: usize,
}

impl Default for SnapshotSearchOptions {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            regex: false,
            case_sensitive: false,
            subtree: None,
            max_results: 50,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotSearchMatch {
    pub reference: String,
    pub role: String,
    pub name: Option<String>,
    pub ancestor_refs: Vec<String>,
    pub frame_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotSearchResult {
    pub matches: Vec<SnapshotSearchMatch>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

pub(super) fn bound_snapshot(snapshot: &mut AriaSnapshot) {
    if snapshot.yaml.len() > MAX_BYTES || snapshot.nodes.len() > MAX_NODES {
        if snapshot.nodes.len() > MAX_NODES {
            let allowed: std::collections::HashSet<_> = snapshot
                .nodes
                .iter()
                .take(MAX_NODES)
                .filter_map(|node| node.reference.as_deref())
                .collect();
            let mut offset = 0;
            for line in snapshot.yaml.split_inclusive('\n') {
                let body = line.trim().strip_prefix("- ");
                let reference = body
                    .and_then(|body| serde_yaml::from_str::<serde_yaml::Value>(body).ok())
                    .and_then(|value| {
                        yaml_key(&value)
                            .and_then(key_ref)
                            .map(|(_, _, r)| r.to_owned())
                    });
                if reference.as_deref().is_some_and(|r| !allowed.contains(r)) {
                    snapshot.yaml.truncate(offset);
                    break;
                }
                offset += line.len();
            }
        }
        // Truncate at a full line so no partial ref can be exposed as actionable.
        if snapshot.yaml.len() > MAX_BYTES {
            let mut end = MAX_BYTES;
            while !snapshot.yaml.is_char_boundary(end) {
                end -= 1;
            }
            let end = snapshot.yaml[..end].rfind('\n').unwrap_or(0);
            snapshot.yaml.truncate(end);
        }
        snapshot.nodes.truncate(MAX_NODES);
        let listed: std::collections::HashSet<_> = paths(&snapshot.yaml, &snapshot.nodes)
            .into_iter()
            .map(|(node, _)| node.reference.clone())
            .collect();
        snapshot
            .nodes
            .retain(|node| listed.contains(&node.reference));
        snapshot.warnings.push("Snapshot output limited to 256 KiB / 4096 nodes; renderer DOM traversal is not bounded".into());
    }
}

pub(super) fn bind_nodes(
    nodes: &[SnapshotNode],
    frame: &crate::FrameHandle,
    bindings: &mut HashMap<Ref, crate::refs::RefFrameBinding>,
) {
    let Some(context) = &frame.utility_context else {
        return;
    };
    for node in nodes {
        if let Some(reference) = node
            .reference
            .as_ref()
            .and_then(|reference| reference.parse().ok())
        {
            bindings.insert(
                reference,
                crate::refs::RefFrameBinding {
                    frame_id: frame.id.clone(),
                    context_id: context.context_id,
                    generation: frame.generation,
                    local_reference: None,
                },
            );
        }
    }
}

// Renderer caches refs on elements and may ignore a changed refPrefix. Public child refs
// therefore use native aliases mapped back to exact renderer-local refs.
pub(super) fn alias_frame(
    snapshot: &mut AriaSnapshot,
    prefix: &str,
    frame: &crate::FrameHandle,
    bindings: &mut HashMap<Ref, crate::refs::RefFrameBinding>,
) {
    let Some(context) = &frame.utility_context else {
        return;
    };
    let mut aliases = HashMap::new();
    for (index, node) in snapshot.nodes.iter_mut().enumerate() {
        let Some(local) = node
            .reference
            .as_ref()
            .and_then(|reference| reference.parse::<Ref>().ok())
        else {
            continue;
        };
        let public: Ref = format!("{prefix}e{}", index + 1)
            .parse()
            .expect("allocated frame alias");
        aliases.insert(local.to_string(), public.to_string());
        node.reference = Some(public.to_string());
        bindings.insert(
            public,
            crate::refs::RefFrameBinding {
                frame_id: frame.id.clone(),
                context_id: context.context_id,
                generation: frame.generation,
                local_reference: Some(local),
            },
        );
    }
    snapshot.yaml = snapshot
        .yaml
        .split('\n')
        .map(|line| {
            let Some(body) = line.trim_start().strip_prefix("- ") else {
                return line.to_string();
            };
            let Ok(original) = serde_yaml::from_str::<serde_yaml::Value>(body) else {
                return line.to_string();
            };
            let Some(key) = yaml_key(&original) else {
                return line.to_string();
            };
            let Some((start, end, local)) = key_ref(key) else {
                return line.to_string();
            };
            let Some(alias) = aliases.get(local) else {
                return line.to_string();
            };
            if !snapshot.nodes.iter().any(|node| {
                node.reference.as_deref() == Some(alias)
                    && key.split_whitespace().next() == Some(node.role.as_str())
            }) {
                return line.to_string();
            }
            let expected = format!("{}{}{}", &key[..start], alias, &key[end..]);
            let token = format!("[ref={local}]");
            for (offset, _) in line.rmatch_indices(&token) {
                let begin = offset + 5;
                let candidate = format!(
                    "{}{}{}",
                    &line[..begin],
                    alias,
                    &line[begin + local.len()..]
                );
                let candidate_body = candidate.trim_start().strip_prefix("- ").unwrap();
                if let Ok(parsed) = serde_yaml::from_str::<serde_yaml::Value>(candidate_body) {
                    if yaml_key(&parsed) == Some(expected.as_str()) {
                        return candidate;
                    }
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
}

fn yaml_key(value: &serde_yaml::Value) -> Option<&str> {
    match value {
        serde_yaml::Value::String(key) => Some(key),
        serde_yaml::Value::Mapping(map) => map.keys().next()?.as_str(),
        _ => None,
    }
}

fn key_ref(key: &str) -> Option<(usize, usize, &str)> {
    let start = key.rfind(" [ref=")? + 6;
    let end = start + key[start..].find(']')?;
    let rest = &key[end + 1..];
    if !rest.is_empty() && !rest.starts_with(" [cursor=") && !rest.starts_with(" [box=") {
        return None;
    }
    Some((start, end, &key[start..end]))
}

// The renderer's YAML is the authoritative hierarchy; the flat nodes are the authoritative
// identity list. Parse each bounded YAML line independently, without recursive YAML loading.
fn paths<'a>(yaml: &str, nodes: &'a [SnapshotNode]) -> Vec<(&'a SnapshotNode, Vec<String>)> {
    let by_ref: HashMap<_, _> = nodes
        .iter()
        .filter_map(|node| node.reference.as_deref().map(|reference| (reference, node)))
        .collect();
    let mut ancestors: Vec<(usize, String)> = Vec::new();
    let mut output = Vec::new();
    for line in yaml.lines().take(MAX_NODES * 4) {
        let indent = line.len() - line.trim_start().len();
        while ancestors.last().is_some_and(|(depth, _)| *depth >= indent) {
            ancestors.pop();
        }
        let Some(body) = line.trim_start().strip_prefix("- ") else {
            continue;
        };
        let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(body) else {
            continue;
        };
        let Some(key) = yaml_key(&value) else {
            continue;
        };
        let Some((_, _, reference)) = key_ref(key) else {
            continue;
        };
        let Some(node) = by_ref.get(reference) else {
            continue;
        };
        if key.split_whitespace().next() != Some(node.role.as_str()) {
            continue;
        }
        output.push((
            *node,
            ancestors
                .iter()
                .map(|(_, reference)| reference.clone())
                .collect(),
        ));
        if ancestors.len() < 128 {
            ancestors.push((indent, reference.to_string()));
        }
    }
    output
}

impl AriaSnapshot {
    pub fn search(&self, options: &SnapshotSearchOptions) -> Result<SnapshotSearchResult, String> {
        if options.pattern.is_empty() || options.pattern.len() > 4096 {
            return Err("Search pattern must contain 1..4096 bytes".into());
        }
        if !(1..=200).contains(&options.max_results) {
            return Err("max_results must be between 1 and 200".into());
        }
        let pattern = if options.regex {
            options.pattern.clone()
        } else {
            regex::escape(&options.pattern)
        };
        let matcher = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case_sensitive)
            .size_limit(1024 * 1024)
            .dfa_size_limit(1024 * 1024)
            .build()
            .map_err(|e| format!("Invalid search pattern: {e}"))?;
        let mut result = SnapshotSearchResult {
            matches: Vec::new(),
            truncated: false,
            warnings: self.warnings.clone(),
        };
        let documents = std::iter::once((&self.yaml, &self.nodes, self.root_frame_id.clone()))
            .chain(
                self.frames
                    .iter()
                    .take(32)
                    .map(|frame| (&frame.yaml, &frame.nodes, Some(frame.frame_id.clone()))),
            );
        let mut subtree_seen = options.subtree.is_none();
        let mut bytes = 0;
        for (yaml, nodes, frame_id) in documents {
            bytes += yaml.len();
            if bytes > MAX_BYTES * 4 {
                result.truncated = true;
                break;
            }
            for (node, ancestors) in paths(yaml, nodes) {
                let reference = node.reference.as_ref().unwrap();
                if let Some(subtree) = &options.subtree {
                    if reference == subtree {
                        subtree_seen = true;
                    }
                    if reference != subtree && !ancestors.contains(subtree) {
                        continue;
                    }
                }
                if !matcher.is_match(node.name.as_deref().unwrap_or(""))
                    && !matcher.is_match(&node.role)
                {
                    continue;
                }
                if result.matches.len() >= options.max_results {
                    result.truncated = true;
                    return Ok(result);
                }
                result.matches.push(SnapshotSearchMatch {
                    reference: reference.clone(),
                    role: node.role.clone(),
                    name: node.name.clone(),
                    ancestor_refs: ancestors,
                    frame_id: frame_id.clone(),
                });
            }
        }
        if !subtree_seen {
            return Err("Subtree ref is not present in this snapshot".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> AriaSnapshot {
        serde_json::from_value(serde_json::json!({
            "yaml": "- region \"Main\" [ref=e1]:\n  - button \"Save\" [ref=e2]\n  - text: '[ref=e999]'\n- button \"Save other\" [ref=e3]",
            "nodes": [
                {"role":"region","name":"Main","ref":"e1","box":null},
                {"role":"button","name":"Save","ref":"e2","box":null},
                {"role":"button","name":"Save other","ref":"e3","box":null}
            ], "root_frame_id":"main"
        })).unwrap()
    }
    #[test]
    fn snapshot_search_preserves_real_refs_and_ancestor_scope() {
        let snapshot = snapshot();
        let options = SnapshotSearchOptions {
            pattern: "save".into(),
            subtree: Some("e1".into()),
            ..Default::default()
        };
        let found = snapshot.search(&options).unwrap();
        assert_eq!(found.matches.len(), 1);
        assert_eq!(found.matches[0].reference, "e2");
        assert_eq!(found.matches[0].ancestor_refs, vec!["e1"]);
        assert_eq!(found.matches[0].frame_id.as_deref(), Some("main"));
        assert!(snapshot
            .search(&SnapshotSearchOptions {
                pattern: "e999".into(),
                ..Default::default()
            })
            .unwrap()
            .matches
            .is_empty());
    }
    #[test]
    fn snapshot_search_validates_regex_and_limits() {
        let snapshot = snapshot();
        assert!(snapshot
            .search(&SnapshotSearchOptions {
                pattern: "[".into(),
                regex: true,
                ..Default::default()
            })
            .is_err());
        assert!(snapshot
            .search(&SnapshotSearchOptions {
                pattern: "x".into(),
                max_results: 0,
                ..Default::default()
            })
            .is_err());
        let found = snapshot
            .search(&SnapshotSearchOptions {
                pattern: "^Save".into(),
                regex: true,
                max_results: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.matches.len(), 1);
        assert!(found.truncated);
        assert!(snapshot
            .search(&SnapshotSearchOptions {
                pattern: "x".into(),
                subtree: Some("e999".into()),
                ..Default::default()
            })
            .is_err());
    }
    #[test]
    fn snapshot_output_budget_drops_unlisted_refs_on_utf8_boundary() {
        let mut snapshot = snapshot();
        snapshot.yaml.push_str(&"é".repeat(MAX_BYTES));
        bound_snapshot(&mut snapshot);
        assert!(snapshot.yaml.len() <= MAX_BYTES);
        assert!(!snapshot.warnings.is_empty());
        assert_eq!(snapshot.nodes.len(), 2);
    }

    #[test]
    fn alias_rewrites_only_actual_yaml_key_ref() {
        let mut snapshot = snapshot();
        snapshot.yaml =
            "- button \"Name [ref=e2]\" [checked] [ref=e2]: '[ref=e2]'\n- text: 'button [ref=e2]'"
                .into();
        let mut tree = crate::FrameTree::default();
        tree.attach("child".into(), None);
        tree.set_utility_context("child", None, 7);
        let mut bindings = HashMap::new();
        alias_frame(
            &mut snapshot,
            "f9",
            tree.frame("child").unwrap(),
            &mut bindings,
        );
        assert_eq!(snapshot.yaml, "- button \"Name [ref=e2]\" [checked] [ref=f9e2]: '[ref=e2]'\n- text: 'button [ref=e2]'");
        assert_eq!(paths(&snapshot.yaml, &snapshot.nodes).len(), 1);
    }

    #[test]
    fn node_cap_cuts_before_excluded_ref_not_quoted_text() {
        let mut snapshot = snapshot();
        snapshot.nodes = (1..=MAX_NODES + 1)
            .map(|i| SnapshotNode {
                role: "button".into(),
                name: Some(format!("Item {i}")),
                reference: Some(format!("e{i}")),
                geometry: None,
            })
            .collect();
        snapshot.yaml = (1..=MAX_NODES + 1)
            .map(|i| format!("- button \"Item {i}\" [ref=e{i}]\n"))
            .collect();
        bound_snapshot(&mut snapshot);
        assert_eq!(snapshot.nodes.len(), MAX_NODES);
        assert!(!snapshot.yaml.contains("[ref=e4097]"));
        assert!(snapshot.yaml.contains("[ref=e4096]"));
        assert!(!snapshot.warnings.is_empty());
    }
}
