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
    let root = tree.root_node();
    let mut loops = Vec::new();
    collect_loops(root, &mut loops);

    let mut out = Vec::new();
    for lp in loops {
        detect_loop(lp, bytes, &mut out);
    }
    detect_blocking_sync_in_async(root, lang, bytes, &mut out);
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

fn detect_blocking_sync_in_async(root: Node<'_>, lang: &str, bytes: &[u8], out: &mut Vec<Finding>) {
    let mut stack = vec![root];
    while let Some(cur) = stack.pop() {
        if is_call(cur) && has_async_function_ancestor(cur, bytes) {
            let txt = text_of(cur, bytes);
            let name = callee_name(cur, bytes).unwrap_or_else(|| txt.to_string());
            if let Some((severity, detail)) = blocking_sync_call(lang, &name, txt) {
                push(out, "blocking_sync_in_async", severity, cur, detail);
            }
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn has_async_function_ancestor(node: Node<'_>, bytes: &[u8]) -> bool {
    let mut cur = node.parent();
    while let Some(parent) = cur {
        if is_function_like(parent) && is_async_function(parent, bytes) {
            return true;
        }
        cur = parent.parent();
    }
    false
}

fn is_function_like(node: Node<'_>) -> bool {
    FUNCTION_KINDS.contains(&node.kind())
        || matches!(
            node.kind(),
            "function" | "function_expression" | "arrow_function" | "closure_expression"
        )
}

fn is_async_function(node: Node<'_>, bytes: &[u8]) -> bool {
    let txt = text_of(node, bytes).trim_start();
    txt.starts_with("async ")
        || txt.starts_with("async\n")
        || txt.starts_with("pub async ")
        || txt.starts_with("pub(crate) async ")
        || txt.starts_with("pub(super) async ")
        || txt.starts_with("pub(in ") && txt.contains(") async ")
        || node_has_async_child(node, bytes)
}

fn node_has_async_child(node: Node<'_>, bytes: &[u8]) -> bool {
    let mut cursor = node.walk();
    let has_async = node
        .children(&mut cursor)
        .any(|child| text_of(child, bytes).trim() == "async");
    has_async
}

fn blocking_sync_call(lang: &str, name: &str, txt: &str) -> Option<(Severity, String)> {
    let low_name = normalize_call_name(name);
    let low_txt = txt.to_ascii_lowercase();
    match lang {
        "rust" => blocking_sync_rust(&low_name, &low_txt),
        "python" => blocking_sync_python(&low_name, &low_txt),
        "typescript" | "tsx" | "javascript" | "jsx" => blocking_sync_js(&low_name, &low_txt),
        _ => None,
    }
}

fn normalize_call_name(name: &str) -> String {
    name.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != ':')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn blocking_sync_rust(name: &str, txt: &str) -> Option<(Severity, String)> {
    if name.starts_with("std::fs::") || txt.starts_with("std::fs::") {
        return Some((
            Severity::Medium,
            format!("blocking filesystem call `{name}` inside async function"),
        ));
    }
    if name == "std::thread::sleep" || txt.starts_with("std::thread::sleep") {
        return Some((
            Severity::High,
            format!("blocking sleep `{name}` inside async function"),
        ));
    }
    if name == "std::io::stdin" || txt.starts_with("std::io::stdin") {
        return Some((
            Severity::Medium,
            format!("blocking stdin call `{name}` inside async function"),
        ));
    }
    if name == "std::net::tcpstream::connect" || txt.starts_with("std::net::tcpstream::connect") {
        return Some((
            Severity::High,
            format!("blocking network connect `{name}` inside async function"),
        ));
    }
    if name.starts_with("reqwest::blocking::") || txt.starts_with("reqwest::blocking::") {
        return Some((
            Severity::High,
            format!("blocking HTTP call `{name}` inside async function"),
        ));
    }
    None
}

fn blocking_sync_python(name: &str, txt: &str) -> Option<(Severity, String)> {
    if name == "time.sleep" || txt.starts_with("time.sleep") {
        return Some((
            Severity::High,
            format!("blocking sleep `{name}` inside async function"),
        ));
    }
    if name.starts_with("requests.") || txt.starts_with("requests.") {
        return Some((
            Severity::High,
            format!("blocking HTTP call `{name}` inside async function"),
        ));
    }
    if name == "open" || txt.starts_with("open(") {
        return Some((
            Severity::Medium,
            format!("blocking file open `{name}` inside async function"),
        ));
    }
    None
}

fn blocking_sync_js(name: &str, txt: &str) -> Option<(Severity, String)> {
    let is_sync_fs = name.starts_with("fs.") && name.ends_with("sync")
        || txt.starts_with("fs.") && txt.split('(').next().unwrap_or("").ends_with("sync");
    if is_sync_fs {
        return Some((
            Severity::Medium,
            format!("blocking filesystem call `{name}` inside async function"),
        ));
    }
    if name == "child_process.execsync" || txt.starts_with("child_process.execsync") {
        return Some((
            Severity::High,
            format!("blocking child process call `{name}` inside async function"),
        ));
    }
    None
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
    fn rust_blocking_fs_in_async_is_detected() {
        let src = r#"async fn load() {
    let _ = std::fs::read_to_string("Cargo.toml");
}
"#;
        let findings = detect_perf("rust", src);

        assert!(
            findings.iter().any(|f| {
                f.biomarker == "blocking_sync_in_async" && f.severity == Severity::Medium
            }),
            "{findings:?}"
        );
    }

    #[test]
    fn rust_sync_fs_outside_async_is_ignored() {
        let src = r#"fn load() {
    let _ = std::fs::read_to_string("Cargo.toml");
}
"#;
        let findings = detect_perf("rust", src);

        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "blocking_sync_in_async"),
            "{findings:?}"
        );
    }

    #[test]
    fn rust_sleep_in_sync_closure_inside_async_is_detected() {
        let src = r#"async fn wait() {
    let f = || { std::thread::sleep(std::time::Duration::from_millis(1)); };
    f();
}
"#;
        let findings = detect_perf("rust", src);

        assert!(
            findings.iter().any(|f| {
                f.biomarker == "blocking_sync_in_async" && f.severity == Severity::High
            }),
            "{findings:?}"
        );
    }

    #[test]
    fn python_blocking_calls_in_async_are_detected() {
        let src = r#"
import time
import requests
async def load(path):
    time.sleep(1)
    requests.get("https://example.com")
    return open(path).read()
"#;
        let findings = detect_perf("python", src);

        assert!(
            findings
                .iter()
                .filter(|f| f.biomarker == "blocking_sync_in_async")
                .count()
                >= 3,
            "{findings:?}"
        );
        assert!(
            findings.iter().any(|f| {
                f.biomarker == "blocking_sync_in_async" && f.severity == Severity::High
            }),
            "{findings:?}"
        );
        assert!(
            findings.iter().any(|f| {
                f.biomarker == "blocking_sync_in_async" && f.severity == Severity::Medium
            }),
            "{findings:?}"
        );
    }

    #[test]
    fn python_sync_requests_are_ignored() {
        let src = r#"
import requests
def load(url):
    return requests.get(url)
"#;
        let findings = detect_perf("python", src);

        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "blocking_sync_in_async"),
            "{findings:?}"
        );
    }

    #[test]
    fn typescript_sync_fs_and_child_process_in_async_are_detected() {
        let src = r#"import fs from 'fs';
import child_process from 'child_process';
async function load(path: string) {
    fs.readFileSync(path);
    child_process.execSync('pwd');
}
"#;
        let findings = detect_perf("typescript", src);

        assert!(
            findings.iter().any(|f| {
                f.biomarker == "blocking_sync_in_async" && f.severity == Severity::Medium
            }),
            "{findings:?}"
        );
        assert!(
            findings.iter().any(|f| {
                f.biomarker == "blocking_sync_in_async" && f.severity == Severity::High
            }),
            "{findings:?}"
        );
    }

    #[test]
    fn javascript_sync_fs_outside_async_is_ignored() {
        let src = r#"const fs = require('fs');
function load(path) {
    return fs.readFileSync(path);
}
"#;
        let findings = detect_perf("javascript", src);

        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "blocking_sync_in_async"),
            "{findings:?}"
        );
    }

    #[test]
    fn blocking_lock_is_not_flagged() {
        let src = r#"async fn lock_it(m: tokio::sync::Mutex<i32>) {
    let _guard = m.blocking_lock();
}
"#;
        let findings = detect_perf("rust", src);

        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "blocking_sync_in_async"),
            "{findings:?}"
        );
    }

    #[test]
    fn blocking_weight_now_reachable() {
        let src = r#"async fn wait() {
    std::thread::sleep(std::time::Duration::from_millis(1));
}
"#;
        let blocking = detect_perf("rust", src)
            .into_iter()
            .find(|f| f.biomarker == "blocking_sync_in_async")
            .unwrap();
        let control = Finding {
            biomarker: "unweighted_control".to_string(),
            category: blocking.category.clone(),
            dimension: blocking.dimension,
            severity: blocking.severity,
            line: blocking.line,
            detail: String::new(),
        };

        let blocking_score = crate::scoring::score_file(&[blocking]).defect;
        let control_score = crate::scoring::score_file(&[control]).defect;

        assert!((blocking_score - 9.16).abs() < 1e-9, "got {blocking_score}");
        assert!((control_score - 9.0).abs() < 1e-9, "got {control_score}");
        assert!(blocking_score > control_score);
    }
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

    #[test]
    fn javascript_fetch_is_io() {
        let src = "async function f(urls) {\n    for (const url of urls) {\n        await fetch(url);\n    }\n}\n";
        let findings = detect_perf("javascript", src);

        assert!(
            findings.iter().any(|f| f.biomarker == "io_in_loop"),
            "{findings:?}"
        );
    }

    #[test]
    fn typescript_client_get_is_io() {
        let src = "async function f(client: HttpClient, urls: string[]) {\n    for (const url of urls) {\n        await client.get(url);\n    }\n}\n";
        let findings = detect_perf("typescript", src);

        assert!(
            findings.iter().any(|f| f.biomarker == "io_in_loop"),
            "{findings:?}"
        );
    }
}
