use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dimension {
    Defect,
    Maintainability,
    Performance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub biomarker: String,
    pub category: String,
    pub dimension: Dimension,
    pub severity: Severity,
    pub line: usize,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduction: Option<f64>,
}

#[derive(Clone, Copy)]
struct FunctionMetric<'a> {
    node: Node<'a>,
    name: &'a str,
    line: usize,
    nloc: u32,
    ccn: u32,
    max_nesting: u32,
    bumps: u32,
    params: usize,
}

#[derive(Clone)]
struct ClassMetric<'a> {
    name: String,
    line: usize,
    total_nloc: u32,
    method_count: usize,
    max_method_ccn: u32,
    lcom4: usize,
    field_count: usize,
    methods: Vec<FunctionMetric<'a>>,
}

const FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "generator_function_declaration",
    "method_declaration",
    "constructor_declaration",
    "method_definition",
    "method",
    "singleton_method",
    "local_function_statement",
];

const CLASS_KINDS: &[&str] = &[
    "class_definition",
    "class_declaration",
    "impl_item",
    "struct_item",
    "interface_declaration",
];

const CONTROL_KINDS: &[&str] = &[
    "if_statement",
    "if_expression",
    "elif_clause",
    "else_if_clause",
    "for_statement",
    "for_expression",
    "for_in_statement",
    "enhanced_for_statement",
    "while_statement",
    "while_expression",
    "do_statement",
    "loop_expression",
    "match_expression",
    "switch_statement",
    "switch_expression",
    "catch_clause",
    "except_clause",
    "case",
    "case_statement",
    "match_arm",
    "conditional_expression",
    "ternary_expression",
];

const MEMBER_ACCESS_KINDS: &[&str] = &[
    "attribute",
    "field_expression",
    "member_expression",
    "member_access_expression",
    "scoped_identifier",
];

pub fn detect_biomarkers(lang: &str, text: &str) -> Vec<Finding> {
    let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) else {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let root = tree.root_node();
    let file_nloc = count_file_nloc(text);
    let mut functions = Vec::new();
    collect_functions(root, bytes, &mut functions);
    let classes = collect_classes(root, bytes);

    let mut out = Vec::new();
    for f in &functions {
        detect_function_biomarkers(*f, file_nloc, &mut out);
    }
    for c in &classes {
        detect_class_biomarkers(c, &mut out);
    }
    collect_error_handling(root, bytes, text, &mut out);
    out.sort_by(|a, b| a.line.cmp(&b.line).then(a.biomarker.cmp(&b.biomarker)));
    out
}

fn finding(
    biomarker: &str,
    category: &str,
    dimension: Dimension,
    severity: Severity,
    line: usize,
    detail: String,
) -> Finding {
    Finding {
        biomarker: biomarker.to_string(),
        category: category.to_string(),
        dimension,
        severity,
        line,
        detail,
        deduction: None,
    }
}

fn detect_function_biomarkers(f: FunctionMetric<'_>, file_nloc: u32, out: &mut Vec<Finding>) {
    if f.nloc >= 70 && f.ccn >= 9 {
        let sev = if f.ccn >= 20 && f.nloc >= 150 {
            Severity::Critical
        } else if f.ccn >= 14 || f.nloc >= 120 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "brain_method",
            "structural_complexity",
            Dimension::Maintainability,
            sev,
            f.line,
            format!("{} is {} lines with CCN {}", f.name, f.nloc, f.ccn),
        ));
    }
    if f.bumps >= 3 && f.ccn >= 5 {
        let sev = if f.bumps >= 5 {
            Severity::High
        } else if f.bumps >= 4 {
            Severity::Medium
        } else {
            Severity::Low
        };
        out.push(finding(
            "bumpy_road",
            "structural_complexity",
            Dimension::Defect,
            sev,
            f.line,
            format!("{} has {} nested blocks at the same level", f.name, f.bumps),
        ));
    }
    for (line, ops, kind) in complex_conditions(f.node) {
        if ops >= 3 {
            let sev = if ops >= 6 {
                Severity::Critical
            } else if ops >= 5 {
                Severity::High
            } else if ops >= 4 {
                Severity::Medium
            } else {
                Severity::Low
            };
            out.push(finding(
                "complex_conditional",
                "structural_complexity",
                Dimension::Defect,
                sev,
                line,
                format!("{kind} condition combines {ops} boolean operators"),
            ));
        }
    }
    if f.ccn >= 9 {
        let sev = if f.ccn >= 25 {
            Severity::Critical
        } else if f.ccn >= 15 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "complex_method",
            "size_and_complexity",
            Dimension::Defect,
            sev,
            f.line,
            format!("{} has cyclomatic complexity {}", f.name, f.ccn),
        ));
    }
    if f.nloc >= 60 && f.ccn >= 2 {
        let sev = if f.nloc >= 200 {
            Severity::Critical
        } else if f.nloc >= 120 {
            Severity::High
        } else if f.nloc >= 90 {
            Severity::Medium
        } else {
            Severity::Low
        };
        out.push(finding(
            "large_method",
            "size_and_complexity",
            Dimension::Defect,
            sev,
            f.line,
            format!("{} is {} lines long", f.name, f.nloc),
        ));
    }
    if f.max_nesting >= 4 {
        let sev = if f.max_nesting >= 7 {
            Severity::Critical
        } else if f.max_nesting >= 5 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "nested_complexity",
            "structural_complexity",
            Dimension::Defect,
            sev,
            f.line,
            format!("{} nests {} levels deep", f.name, f.max_nesting),
        ));
    }
    let threshold = if matches!(f.name, "__init__" | "init" | "constructor") {
        7
    } else {
        5
    };
    if file_nloc >= 60 && f.params >= threshold {
        let sev = if f.params >= threshold + 4 {
            Severity::High
        } else if f.params >= threshold + 2 {
            Severity::Medium
        } else {
            Severity::Low
        };
        out.push(finding(
            "primitive_obsession",
            "size_and_complexity",
            Dimension::Maintainability,
            sev,
            f.line,
            format!("{} takes {} parameters", f.name, f.params),
        ));
    }
}

fn detect_class_biomarkers(c: &ClassMetric<'_>, out: &mut Vec<Finding>) {
    if c.total_nloc >= 200
        && c.method_count >= 15
        && c.methods.iter().any(|m| m.nloc >= 70 && m.ccn >= 9)
    {
        let sev = if c.total_nloc >= 400 && c.method_count >= 25 {
            Severity::Critical
        } else if c.total_nloc >= 300 || c.method_count >= 20 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "god_class",
            "structural_complexity",
            Dimension::Defect,
            sev,
            c.line,
            format!(
                "{} is {} lines across {} methods (max method CCN {})",
                c.name, c.total_nloc, c.method_count, c.max_method_ccn
            ),
        ));
    }
    if c.lcom4 >= 2 && c.method_count >= 5 {
        let sev = if c.lcom4 >= 4 && c.method_count >= 15 {
            Severity::Critical
        } else if c.lcom4 >= 3 || c.method_count >= 20 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "low_cohesion",
            "structural_complexity",
            Dimension::Maintainability,
            sev,
            c.line,
            format!(
                "{} has low cohesion (LCOM4={}): {} methods, {} fields",
                c.name, c.lcom4, c.method_count, c.field_count
            ),
        ));
    }
}

fn collect_functions<'a>(node: Node<'a>, bytes: &'a [u8], out: &mut Vec<FunctionMetric<'a>>) {
    if FUNCTION_KINDS.contains(&node.kind()) {
        out.push(function_metric(node, bytes));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, bytes, out);
    }
}

fn function_metric<'a>(node: Node<'a>, bytes: &'a [u8]) -> FunctionMetric<'a> {
    let (ccn, max_nesting, bumps) = walk_complexity(node);
    FunctionMetric {
        node,
        name: function_name(node, bytes),
        line: node.start_position().row + 1,
        nloc: lines_of_code(node),
        ccn,
        max_nesting,
        bumps,
        params: parameter_count(node),
    }
}

fn function_name<'a>(node: Node<'a>, bytes: &'a [u8]) -> &'a str {
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(bytes).unwrap_or("");
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "simple_identifier" | "field_identifier" | "property_identifier"
        ) {
            return child.utf8_text(bytes).unwrap_or("");
        }
    }
    ""
}

fn class_name(node: Node<'_>, bytes: &[u8]) -> String {
    for field in ["name", "type"] {
        if let Some(n) = node.child_by_field_name(field) {
            return n.utf8_text(bytes).unwrap_or("").to_string();
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "type_identifier" | "simple_identifier"
        ) {
            return child.utf8_text(bytes).unwrap_or("").to_string();
        }
    }
    String::new()
}

fn collect_classes<'a>(root: Node<'a>, bytes: &'a [u8]) -> Vec<ClassMetric<'a>> {
    let mut class_nodes = Vec::new();
    collect_class_nodes(root, &mut class_nodes);
    class_nodes
        .into_iter()
        .map(|node| {
            let mut methods = Vec::new();
            collect_class_methods(node, bytes, &mut methods);
            let (lcom4, field_count) = compute_lcom4(&methods, bytes);
            ClassMetric {
                name: class_name(node, bytes),
                line: node.start_position().row + 1,
                total_nloc: lines_of_code(node),
                method_count: methods.len(),
                max_method_ccn: methods.iter().map(|m| m.ccn).max().unwrap_or(0),
                lcom4,
                field_count,
                methods,
            }
        })
        .collect()
}

fn collect_class_methods<'a>(node: Node<'a>, bytes: &'a [u8], out: &mut Vec<FunctionMetric<'a>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_class_methods_from_child(child, bytes, out);
    }
}

fn collect_class_methods_from_child<'a>(
    node: Node<'a>,
    bytes: &'a [u8],
    out: &mut Vec<FunctionMetric<'a>>,
) {
    if CLASS_KINDS.contains(&node.kind()) && node.is_named() {
        return;
    }
    if FUNCTION_KINDS.contains(&node.kind()) {
        out.push(function_metric(node, bytes));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_class_methods_from_child(child, bytes, out);
    }
}

fn collect_class_nodes<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    if CLASS_KINDS.contains(&node.kind()) && node.is_named() {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_class_nodes(child, out);
    }
}

fn compute_lcom4(methods: &[FunctionMetric<'_>], bytes: &[u8]) -> (usize, usize) {
    let n = methods.len();
    if n == 0 {
        return (0, 0);
    }
    let member_sets: Vec<BTreeSet<String>> = methods
        .iter()
        .map(|m| collect_self_members(m.node, bytes))
        .collect();
    let total_refs: usize = member_sets.iter().map(BTreeSet::len).sum();
    let method_names: BTreeSet<String> = methods.iter().map(|m| m.name.to_string()).collect();
    let all_members: BTreeSet<String> =
        member_sets.iter().flat_map(|s| s.iter().cloned()).collect();
    let field_count = all_members.difference(&method_names).count();
    if total_refs == 0 {
        return (n, field_count);
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, members) in member_sets.iter().enumerate() {
        for member in members {
            buckets.entry(member.clone()).or_default().push(i);
        }
    }
    for (name, mut idxs) in buckets {
        if let Some(callee) = methods.iter().position(|m| m.name == name) {
            idxs.push(callee);
        }
        if let Some((&first, rest)) = idxs.split_first() {
            for &other in rest {
                union(&mut parent, first, other);
            }
        }
    }
    let roots: BTreeSet<_> = (0..n).map(|i| find(&mut parent, i)).collect();
    (roots.len(), field_count)
}

fn collect_self_members(node: Node<'_>, bytes: &[u8]) -> BTreeSet<String> {
    let mut members = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if cur != node && CLASS_KINDS.contains(&cur.kind()) {
            continue;
        }
        if MEMBER_ACCESS_KINDS.contains(&cur.kind()) {
            if let Some(name) = self_member_name(cur, bytes) {
                members.insert(name);
            }
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    members
}

fn self_member_name(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let obj = ["object", "value", "argument", "operand", "expression"]
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .or_else(|| named_children(node).into_iter().next())?;
    let prop = ["property", "attribute", "field", "name"]
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .or_else(|| {
            named_children(node)
                .into_iter()
                .rev()
                .find(|c| c.kind().contains("identifier"))
        })?;
    let obj_text = obj.utf8_text(bytes).ok()?;
    if !matches!(obj_text, "self" | "this") {
        return None;
    }
    Some(prop.utf8_text(bytes).ok()?.to_string())
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.is_named())
        .collect()
}

fn walk_complexity(node: Node<'_>) -> (u32, u32, u32) {
    let mut ccn = 1;
    let mut max_nesting = 0;
    fn rec(node: Node<'_>, depth: u32, ccn: &mut u32, max_nesting: &mut u32) {
        let mut inc = 0;
        let mut nest = 0;
        if CONTROL_KINDS.contains(&node.kind()) {
            inc = 1;
            nest = 1;
        } else if is_boolean_operator(node) {
            inc = 1;
        }
        *ccn += inc;
        let new_depth = depth + nest;
        *max_nesting = (*max_nesting).max(new_depth);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            rec(child, new_depth, ccn, max_nesting);
        }
    }
    let mut bumps = 0;
    let body = node.child_by_field_name("body").unwrap_or(node);
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        let before = max_nesting;
        let mut child_peak = 0;
        rec(child, 0, &mut ccn, &mut child_peak);
        max_nesting = before.max(child_peak);
        if child_peak >= 2 {
            bumps += 1;
        }
    }
    (ccn, max_nesting, bumps)
}

fn complex_conditions(node: Node<'_>) -> Vec<(usize, u32, &'static str)> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if cur != node && FUNCTION_KINDS.contains(&cur.kind()) {
            continue;
        }
        if CONTROL_KINDS.contains(&cur.kind()) {
            let ops = condition_nodes(cur)
                .iter()
                .map(|c| count_boolean_ops(*c))
                .sum();
            if ops > 0 {
                out.push((
                    cur.start_position().row + 1,
                    ops,
                    construct_name(cur.kind()),
                ));
            }
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    out
}

fn condition_nodes(node: Node<'_>) -> Vec<Node<'_>> {
    if let Some(c) = node.child_by_field_name("condition") {
        return vec![c];
    }
    if let Some(v) = node.child_by_field_name("value") {
        return vec![v];
    }
    named_children(node)
        .into_iter()
        .filter(|c| {
            !matches!(
                c.kind(),
                "block" | "compound_statement" | "statement_block" | "body"
            )
        })
        .collect()
}

fn count_boolean_ops(node: Node<'_>) -> u32 {
    let mut count = 0;
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if cur != node && FUNCTION_KINDS.contains(&cur.kind()) {
            continue;
        }
        if is_boolean_operator(cur) {
            count += 1;
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    count
}

fn is_boolean_operator(node: Node<'_>) -> bool {
    matches!(node.kind(), "&&" | "||" | "and" | "or")
}

fn construct_name(kind: &str) -> &'static str {
    if kind.contains("for") {
        "for"
    } else if kind.contains("while") {
        "while"
    } else if kind.contains("case") || kind.contains("arm") {
        "case"
    } else if kind.contains("catch") || kind.contains("except") {
        "catch"
    } else if kind.contains("conditional") || kind.contains("ternary") {
        "ternary"
    } else {
        "if"
    }
}

fn parameter_count(node: Node<'_>) -> usize {
    let params = node
        .child_by_field_name("parameters")
        .or_else(|| node.child_by_field_name("parameter"));
    let Some(params) = params else {
        return 0;
    };
    named_children(params)
        .into_iter()
        .filter(|c| {
            let k = c.kind();
            k.contains("parameter")
                || matches!(k, "identifier" | "typed_identifier" | "self_parameter")
        })
        .filter(|c| c.kind() != "self_parameter")
        .count()
}

fn lines_of_code(node: Node<'_>) -> u32 {
    (node
        .end_position()
        .row
        .saturating_sub(node.start_position().row)
        + 1) as u32
}

fn count_file_nloc(text: &str) -> u32 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u32
}

fn collect_error_handling(root: Node<'_>, bytes: &[u8], text: &str, out: &mut Vec<Finding>) {
    let mut stack = vec![root];
    while let Some(cur) = stack.pop() {
        let kind = cur.kind();
        if matches!(kind, "catch_clause" | "except_clause") {
            if is_bare_except(cur, bytes) {
                out.push(finding(
                    "error_handling",
                    "error_handling",
                    Dimension::Maintainability,
                    Severity::Low,
                    cur.start_position().row + 1,
                    "catch-all exception hides every error".to_string(),
                ));
            }
            if is_empty_handler(cur) {
                out.push(finding(
                    "error_handling",
                    "error_handling",
                    Dimension::Maintainability,
                    Severity::Low,
                    cur.start_position().row + 1,
                    "caught exception is swallowed without handling".to_string(),
                ));
            }
        }
        if kind == "macro_invocation" && cur.utf8_text(bytes).unwrap_or("").starts_with("panic!") {
            out.push(finding(
                "error_handling",
                "error_handling",
                Dimension::Maintainability,
                Severity::Low,
                cur.start_position().row + 1,
                "panic turns a recoverable error into a crash".to_string(),
            ));
        }
        if matches!(kind, "call_expression" | "method_invocation") {
            let t = cur.utf8_text(bytes).unwrap_or("");
            if t.contains(".unwrap(") || t.contains(".expect(") {
                out.push(finding(
                    "error_handling",
                    "error_handling",
                    Dimension::Maintainability,
                    Severity::Low,
                    cur.start_position().row + 1,
                    "unwrap/expect turns a recoverable error into a crash".to_string(),
                ));
            }
        }
        let mut cursor = cur.walk();
        for child in cur.children(&mut cursor) {
            stack.push(child);
        }
    }
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "panic!()" || trimmed.starts_with("panic!(") {
            out.push(finding(
                "error_handling",
                "error_handling",
                Dimension::Maintainability,
                Severity::Low,
                idx + 1,
                "panic turns a recoverable error into a crash".to_string(),
            ));
        }
    }
}

fn is_bare_except(node: Node<'_>, bytes: &[u8]) -> bool {
    let text = node.utf8_text(bytes).unwrap_or("").trim_start();
    text.starts_with("except:") || text.starts_with("except :") || text.starts_with("catch {")
}

fn is_empty_handler(node: Node<'_>) -> bool {
    let body = node.child_by_field_name("body").unwrap_or(node);
    let named = named_children(body);
    named.is_empty()
        || named
            .iter()
            .all(|c| matches!(c.kind(), "pass_statement" | "comment"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_function_has_no_findings() {
        let findings = detect_biomarkers("rust", "fn simple() -> i32 { 1 }\n");
        assert!(findings.is_empty(), "got {findings:?}");
    }

    #[test]
    fn branchy_deep_function_emits_findings() {
        let src = format!("fn brain(x: i32) -> i32 {{\n{}\n    0\n}}\n", (0..75).map(|i| format!("    if x > {i} {{ if x % 2 == 0 {{ if x < 100 {{ if x != 42 {{ return {i}; }} }} }} }}")).collect::<Vec<_>>().join("\n"));
        let findings = detect_biomarkers("rust", &src);
        assert!(
            findings.iter().any(|f| f.biomarker == "brain_method"),
            "got {findings:?}"
        );
        assert!(findings.iter().any(|f| f.biomarker == "nested_complexity"));
    }

    #[test]
    fn lcom4_detects_two_method_groups() {
        let src = "class Foo:\n    def a(self):\n        return self.x\n    def b(self):\n        return self.x + 1\n    def c(self):\n        return self.y\n    def d(self):\n        return self.y + 1\n    def e(self):\n        return self.y + 2\n";
        let findings = detect_biomarkers("python", src);
        assert!(
            findings
                .iter()
                .any(|f| f.biomarker == "low_cohesion" && f.detail.contains("LCOM4=2")),
            "got {findings:?}"
        );
    }

    #[test]
    fn lcom4_counts_disconnected_methods_without_member_refs() {
        let src = "class Foo:\n    def a(self):\n        return 1\n    def b(self):\n        return 2\n    def c(self):\n        return 3\n    def d(self):\n        return 4\n    def e(self):\n        return 5\n";
        let findings = detect_biomarkers("python", src);
        assert!(
            findings
                .iter()
                .any(|f| f.biomarker == "low_cohesion" && f.detail.contains("LCOM4=5")),
            "got {findings:?}"
        );
    }

    #[test]
    fn class_methods_exclude_nested_class_methods() {
        let src = "class Outer:\n    def a(self):\n        return 1\n    class Inner:\n        def hidden(self):\n            return self.x\n        def also_hidden(self):\n            return self.y\n    def b(self):\n        return 2\n    def c(self):\n        return 3\n    def d(self):\n        return 4\n";
        let tree = refact_codegraph_parsers::parse_tree("python", src).unwrap();
        let classes = collect_classes(tree.root_node(), src.as_bytes());
        let outer = classes.iter().find(|c| c.name == "Outer").unwrap();
        let inner = classes.iter().find(|c| c.name == "Inner").unwrap();

        assert_eq!(outer.method_count, 4);
        assert_eq!(outer.lcom4, 4);
        assert_eq!(inner.method_count, 2);
    }

    #[test]
    fn boolean_condition_counts_operator_tokens_once() {
        let src = "fn check(a: bool, b: bool, c: bool) -> bool {\n    if a && b || c {\n        return true;\n    }\n    false\n}\n";
        let findings = detect_biomarkers("rust", src);
        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "complex_conditional"),
            "got {findings:?}"
        );
    }

    #[test]
    fn python_boolean_condition_counts_operator_tokens_once() {
        let src =
            "def check(a, b, c):\n    if a and b or c:\n        return True\n    return False\n";
        let findings = detect_biomarkers("python", src);
        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "complex_conditional"),
            "got {findings:?}"
        );
    }

    #[test]
    fn javascript_boolean_condition_counts_operator_tokens_once() {
        let src = "function check(a, b, c) {\n    if (a && b || c) {\n        return true;\n    }\n    return false;\n}\n";
        let findings = detect_biomarkers("javascript", src);
        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "complex_conditional"),
            "got {findings:?}"
        );
    }

    #[test]
    fn typescript_boolean_condition_counts_operator_tokens_once() {
        let src = "function check(a: boolean, b: boolean, c: boolean): boolean {\n    if (a && b || c) {\n        return true;\n    }\n    return false;\n}\n";
        let findings = detect_biomarkers("typescript", src);
        assert!(
            findings
                .iter()
                .all(|f| f.biomarker != "complex_conditional"),
            "got {findings:?}"
        );
    }
}
