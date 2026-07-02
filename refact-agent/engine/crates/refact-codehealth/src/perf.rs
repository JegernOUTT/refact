use crate::biomarkers::{Dimension, Finding, Severity};
use tree_sitter::Node;

const CATEGORY: &str = "performance";

const LOOP_KINDS: &[&str] = &[
    "for_statement",
    "for_expression",
    "for_in_statement",
    "enhanced_for_statement",
    "while_statement",
    "while_expression",
    "do_statement",
    "loop_expression",
    "list_comprehension",
    "set_comprehension",
    "dictionary_comprehension",
    "generator_expression",
    "comprehension",
];

const CALL_KINDS: &[&str] = &[
    "call",
    "call_expression",
    "method_invocation",
    "invocation_expression",
    "object_creation_expression",
];

const FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "method_declaration",
    "method_definition",
    "lambda",
];

pub fn detect_perf(lang: &str, text: &str) -> Vec<Finding> {
    let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) else {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let mut loops = Vec::new();
    collect_loops(tree.root_node(), &mut loops);

    let mut out = Vec::new();
    for lp in loops {
        detect_loop(lp, bytes, &mut out);
    }
    out.sort_by(|a, b| a.line.cmp(&b.line).then(a.biomarker.cmp(&b.biomarker)));
    out.dedup_by(|a, b| a.line == b.line && a.biomarker == b.biomarker && a.detail == b.detail);
    out
}

pub fn io_kind(name: &str) -> bool {
    let n = name
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .to_ascii_lowercase();
    let last = n.rsplit(['.', ':']).next().unwrap_or(&n);
    let receiver = n
        .rsplit_once('.')
        .or_else(|| n.rsplit_once("::"))
        .map(|(receiver, _)| receiver)
        .unwrap_or("");
    matches!(
        last,
        "open"
            | "read"
            | "write"
            | "connect"
            | "execute"
            | "query"
            | "fetch"
            | "request"
            | "glob"
            | "listdir"
            | "stat"
            | "subprocess"
            | "popen"
    ) || n.contains("cursor.execute")
        || (matches!(last, "get" | "post") && looks_like_http_receiver(receiver))
}

fn looks_like_http_receiver(receiver: &str) -> bool {
    let receiver = receiver.rsplit(['.', ':']).next().unwrap_or(receiver);
    matches!(
        receiver,
        "requests"
            | "reqwest"
            | "ureq"
            | "surf"
            | "http"
            | "https"
            | "client"
            | "session"
            | "axios"
            | "fetch"
    ) || receiver.ends_with("client")
        || receiver.ends_with("session")
}

fn detect_loop(node: Node<'_>, bytes: &[u8], out: &mut Vec<Finding>) {
    let nested = direct_nested_loops(node);
    for inner in nested {
        if subtree_has_io(inner, bytes) {
            push(
                out,
                "nested_loop_with_io",
                Severity::High,
                node,
                "nested loop performs I/O".to_string(),
            );
        } else {
            push(
                out,
                "nested_loop_quadratic",
                Severity::Medium,
                node,
                "loop directly contains another loop".to_string(),
            );
        }
    }

    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if cur != node && FUNCTION_KINDS.contains(&cur.kind()) {
            continue;
        }
        let txt = text_of(cur, bytes);
        if is_call(cur) {
            let name = callee_name(cur, bytes).unwrap_or_else(|| txt.to_string());
            if io_kind(&name) {
                push(
                    out,
                    "io_in_loop",
                    Severity::High,
                    cur,
                    format!("I/O call `{name}` inside loop"),
                );
            }
            if is_resource_construction(&name, txt) {
                push(
                    out,
                    "resource_construction_in_loop",
                    Severity::Low,
                    cur,
                    format!("resource construction `{name}` inside loop"),
                );
            }
            if is_json_parse(&name, txt) {
                push(
                    out,
                    "json_parse_in_loop",
                    Severity::Low,
                    cur,
                    format!("JSON parsing `{name}` inside loop"),
                );
            }
            if is_lock(&name, txt) {
                push(
                    out,
                    "lock_in_loop",
                    Severity::Medium,
                    cur,
                    format!("lock acquisition `{name}` inside loop"),
                );
            }
        }
        if is_string_concat(cur, txt) {
            push(
                out,
                "string_concat_in_loop",
                Severity::Medium,
                cur,
                "string concatenation inside loop".to_string(),
            );
        }
        if is_membership_against_list(cur, txt) {
            push(
                out,
                "membership_test_against_list_in_loop",
                Severity::Medium,
                cur,
                "membership test inside loop may scan a list".to_string(),
            );
        }
        if is_await(cur, txt) {
            push(
                out,
                "serial_await_in_loop",
                Severity::Medium,
                cur,
                "await inside loop may serialize work".to_string(),
            );
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn collect_loops<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    if is_loop(node) {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_loops(child, out);
    }
}

fn direct_nested_loops(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut stack = named_children(node);
    while let Some(cur) = stack.pop() {
        if cur == node {
            continue;
        }
        if is_loop(cur) {
            out.push(cur);
            continue;
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    out
}

fn subtree_has_io(node: Node<'_>, bytes: &[u8]) -> bool {
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if cur != node && FUNCTION_KINDS.contains(&cur.kind()) {
            continue;
        }
        if is_call(cur) {
            if let Some(name) = callee_name(cur, bytes) {
                if io_kind(&name) {
                    return true;
                }
            }
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

fn is_loop(node: Node<'_>) -> bool {
    LOOP_KINDS.contains(&node.kind()) || node.kind().contains("comprehension")
}

fn is_call(node: Node<'_>) -> bool {
    CALL_KINDS.contains(&node.kind()) || node.kind().ends_with("call_expression")
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.is_named())
        .collect()
}

fn text_of<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn callee_name(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    if let Some(f) = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("name"))
    {
        return Some(text_of(f, bytes).trim().to_string());
    }
    let mut best = None;
    for child in named_children(node) {
        let k = child.kind();
        if k.contains("identifier")
            || k.contains("attribute")
            || k.contains("member")
            || k.contains("field")
        {
            best = Some(text_of(child, bytes).trim().to_string());
        }
    }
    best.or_else(|| {
        text_of(node, bytes)
            .split('(')
            .next()
            .map(|s| s.trim().to_string())
    })
}

fn is_string_concat(node: Node<'_>, txt: &str) -> bool {
    let kind = node.kind();
    if !(kind.contains("assignment") || kind == "assignment_expression") {
        return false;
    }
    if txt.contains("+=") {
        return txt.contains('"') || txt.contains('\'') || txt.to_ascii_lowercase().contains("str");
    }
    if let Some((left, right)) = txt.split_once('=') {
        let lhs = left.trim().split_whitespace().last().unwrap_or(left.trim());
        return right.contains('+') && !lhs.is_empty() && right.contains(lhs);
    }
    false
}

fn is_membership_against_list(node: Node<'_>, txt: &str) -> bool {
    let kind = node.kind();
    if !(kind.contains("comparison") || kind.contains("binary") || kind == "in") {
        return false;
    }
    let compact = txt.replace('\n', " ");
    compact.contains(" in [")
        || compact.contains(" in list(")
        || compact.contains(" in Array")
        || (compact.contains(" in ")
            && !compact.contains(" in set(")
            && !compact.contains(" in {")
            && !compact.contains(" in dict("))
}

fn is_await(node: Node<'_>, txt: &str) -> bool {
    node.kind().contains("await")
        || txt.contains(".await")
        || txt.trim_start().starts_with("await ")
}

fn is_resource_construction(name: &str, txt: &str) -> bool {
    let low = name.to_ascii_lowercase();
    low.ends_with(".new")
        || low == "new"
        || low.ends_with("client")
        || low.contains("client::new")
        || low.ends_with(".open")
        || txt.contains("new ")
        || txt.contains("Client(")
        || txt.contains("Client::new")
}

fn is_json_parse(name: &str, txt: &str) -> bool {
    let low = format!("{} {}", name, txt).to_ascii_lowercase();
    low.contains("json.loads")
        || low.contains("json.parse")
        || low.contains("serde_json::from_")
        || low.contains("from_json")
}

fn is_lock(name: &str, txt: &str) -> bool {
    let low = format!("{} {}", name, txt).to_ascii_lowercase();
    low.ends_with("lock")
        || low.ends_with(".lock")
        || low.ends_with("acquire")
        || low.contains("mutex")
        || low.contains("rwlock")
}

fn push(
    out: &mut Vec<Finding>,
    biomarker: &str,
    severity: Severity,
    node: Node<'_>,
    detail: String,
) {
    out.push(Finding {
        biomarker: biomarker.to_string(),
        category: CATEGORY.to_string(),
        dimension: Dimension::Performance,
        severity,
        line: node.start_position().row + 1,
        detail,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_nested_loop_with_io() {
        let src = r#"
import requests
for u in urls:
    for p in pages:
        requests.get(p)
"#;
        let findings = detect_perf("python", src);
        assert!(
            findings
                .iter()
                .any(|f| f.biomarker == "nested_loop_with_io"),
            "{findings:?}"
        );
        assert!(
            findings.iter().any(|f| f.biomarker == "io_in_loop"),
            "{findings:?}"
        );
    }

    #[test]
    fn detects_string_concat_in_loop() {
        let src = r#"
s = ""
for i in range(3):
    s += "x"
"#;
        let findings = detect_perf("python", src);
        assert!(
            findings
                .iter()
                .any(|f| f.biomarker == "string_concat_in_loop"),
            "{findings:?}"
        );
    }

    #[test]
    fn hash_map_get_is_not_io() {
        let src = "fn f(map: std::collections::HashMap<String, String>, keys: Vec<String>) {\n    for key in keys {\n        let _ = map.get(&key);\n    }\n}\n";
        let findings = detect_perf("rust", src);

        assert!(
            findings.iter().all(|f| f.biomarker != "io_in_loop"),
            "{findings:?}"
        );
    }

    #[test]
    fn known_http_get_is_io() {
        let src = r#"
import requests
for url in urls:
    requests.get(url)
"#;
        let findings = detect_perf("python", src);

        assert!(
            findings.iter().any(|f| f.biomarker == "io_in_loop"),
            "{findings:?}"
        );
    }

    #[test]
    fn known_rust_http_get_is_io() {
        let src = "async fn f(urls: Vec<String>) {\n    for url in urls {\n        let _ = reqwest::get(url).await;\n    }\n}\n";
        let findings = detect_perf("rust", src);

        assert!(
            findings.iter().any(|f| f.biomarker == "io_in_loop"),
            "{findings:?}"
        );
    }
}
