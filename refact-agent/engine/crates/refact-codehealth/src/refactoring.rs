use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tree_sitter::Node;

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
    "class_declaration",
    "class_definition",
    "class",
    "struct_item",
    "impl_item",
    "interface_declaration",
    "object_creation_expression",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefactoringKind {
    ExtractMethod,
    ExtractClass,
    ExtractHelper,
    MoveMethod,
    BreakCycle,
    SplitFile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefactoringSuggestion {
    pub kind: RefactoringKind,
    pub target: String,
    pub line: usize,
    pub rationale: String,
    pub impact: f64,
    pub effort: String,
}

pub fn suggest_refactorings(lang: &str, text: &str) -> Vec<RefactoringSuggestion> {
    let health = crate::analyze(lang, text);
    let mut suggestions = Vec::new();

    for function in &health.functions {
        if function.complexity > 10 || function.loc > 60 {
            let complexity_impact = function.complexity as f64 / 10.0;
            let loc_impact = function.loc as f64 / 60.0;
            suggestions.push(RefactoringSuggestion {
                kind: RefactoringKind::ExtractMethod,
                target: function.name.clone(),
                line: function.line1,
                rationale: format!(
                    "Function has complexity {} and {} LOC; extract a cohesive block into a smaller method.",
                    function.complexity, function.loc
                ),
                impact: (complexity_impact + loc_impact).max(1.0),
                effort: String::new(),
            });
        }

        if function.nesting >= 4 {
            suggestions.push(RefactoringSuggestion {
                kind: RefactoringKind::ExtractHelper,
                target: function.name.clone(),
                line: function.line1,
                rationale: format!(
                    "Function reaches nesting depth {}; extract the deeply nested inner block into a helper.",
                    function.nesting
                ),
                impact: 0.8 + function.nesting as f64 / 4.0,
                effort: String::new(),
            });
        }
    }

    let file_loc = text.lines().count();
    if health.functions.len() > 12 || file_loc > 400 {
        suggestions.push(RefactoringSuggestion {
            kind: RefactoringKind::SplitFile,
            target: "file".to_string(),
            line: 1,
            rationale: format!(
                "File contains {} detected functions and {} LOC; split cohesive symbol groups into smaller files.",
                health.functions.len(), file_loc
            ),
            impact: health.functions.len() as f64 / 6.0 + file_loc as f64 / 400.0,
            effort: String::new(),
        });
    }

    if let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) {
        let root = tree.root_node();
        let bytes = text.as_bytes();
        let top_level_functions = count_top_level_functions(root);
        if top_level_functions > 12
            && !suggestions
                .iter()
                .any(|s| s.kind == RefactoringKind::SplitFile)
        {
            suggestions.push(RefactoringSuggestion {
                kind: RefactoringKind::SplitFile,
                target: "file".to_string(),
                line: 1,
                rationale: format!(
                    "File contains {top_level_functions} top-level functions; split related functions into focused files."
                ),
                impact: top_level_functions as f64 / 6.0,
                effort: String::new(),
            });
        }
        suggestions.extend(class_and_move_method_suggestions(root, bytes));
    }

    rank(suggestions)
}

pub fn break_cycle_suggestions(cycles: &[Vec<String>]) -> Vec<RefactoringSuggestion> {
    rank(
        cycles
            .iter()
            .filter(|cycle| cycle.len() >= 2)
            .map(|cycle| RefactoringSuggestion {
                kind: RefactoringKind::BreakCycle,
                target: cycle.join(" -> "),
                line: 1,
                rationale: format!(
                    "Dependency cycle across {} members; introduce an interface, invert one dependency, or extract shared code.",
                    cycle.len()
                ),
                impact: cycle.len() as f64,
                effort: String::new(),
            })
            .collect(),
    )
}

pub fn rank(mut s: Vec<RefactoringSuggestion>) -> Vec<RefactoringSuggestion> {
    for suggestion in &mut s {
        suggestion.effort = if suggestion.impact >= 3.0 {
            "high"
        } else if suggestion.impact >= 1.5 {
            "medium"
        } else {
            "low"
        }
        .to_string();
    }
    s.sort_by(|a, b| {
        b.impact
            .partial_cmp(&a.impact)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
            .then_with(|| a.target.cmp(&b.target))
    });
    s
}

fn count_top_level_functions(root: Node) -> usize {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if FUNCTION_KINDS.contains(&child.kind()) {
            count += 1;
        }
    }
    count
}

fn class_and_move_method_suggestions(root: Node, bytes: &[u8]) -> Vec<RefactoringSuggestion> {
    let mut classes = Vec::new();
    collect_classes(root, bytes, &mut classes);
    let type_names: Vec<String> = classes
        .iter()
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .collect();
    let mut out = Vec::new();

    for class in classes {
        if class.methods.len() > 10 {
            out.push(RefactoringSuggestion {
                kind: RefactoringKind::ExtractClass,
                target: class.name.clone(),
                line: class.line,
                rationale: format!(
                    "Type has {} methods; extract cohesive responsibilities into a smaller class/type.",
                    class.methods.len()
                ),
                impact: class.methods.len() as f64 / 5.0,
                effort: String::new(),
            });
        }

        for method in &class.methods {
            if let Some((target_type, foreign_count, own_count)) =
                feature_envy(method, &class.name, &type_names)
            {
                out.push(RefactoringSuggestion {
                    kind: RefactoringKind::MoveMethod,
                    target: format!("{}.{}", class.name, method.name),
                    line: method.line,
                    rationale: format!(
                        "Method references {target_type} {foreign_count} times versus its own type {own_count} times; consider moving it closer to {target_type}."
                    ),
                    impact: foreign_count as f64 / 3.0,
                    effort: String::new(),
                });
            }
        }
    }

    out
}

#[derive(Debug, Clone)]
struct ClassInfo {
    name: String,
    line: usize,
    methods: Vec<MethodInfo>,
}

#[derive(Debug, Clone)]
struct MethodInfo {
    name: String,
    line: usize,
    body: String,
}

fn collect_classes(node: Node, bytes: &[u8], out: &mut Vec<ClassInfo>) {
    if CLASS_KINDS.contains(&node.kind()) {
        let mut methods = Vec::new();
        collect_methods(node, bytes, &mut methods);
        out.push(ClassInfo {
            name: class_name(node, bytes),
            line: node.start_position().row + 1,
            methods,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_classes(child, bytes, out);
    }
}

fn collect_methods(node: Node, bytes: &[u8], out: &mut Vec<MethodInfo>) {
    if FUNCTION_KINDS.contains(&node.kind()) {
        out.push(MethodInfo {
            name: symbol_name(node, bytes),
            line: node.start_position().row + 1,
            body: node.utf8_text(bytes).unwrap_or("").to_string(),
        });
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_methods(child, bytes, out);
    }
}

fn class_name(node: Node, bytes: &[u8]) -> String {
    if let Some(name) = node.child_by_field_name("name") {
        return name.utf8_text(bytes).unwrap_or("").to_string();
    }
    if node.kind() == "impl_item" {
        if let Some(ty) = node.child_by_field_name("type") {
            return ty.utf8_text(bytes).unwrap_or("").to_string();
        }
    }
    symbol_name(node, bytes)
}

fn symbol_name(node: Node, bytes: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier"
                | "type_identifier"
                | "simple_identifier"
                | "field_identifier"
                | "property_identifier"
        ) {
            return child.utf8_text(bytes).unwrap_or("").to_string();
        }
    }
    String::new()
}

fn feature_envy(
    method: &MethodInfo,
    own_type: &str,
    type_names: &[String],
) -> Option<(String, usize, usize)> {
    if own_type.is_empty() {
        return None;
    }
    let own_count = identifier_count(&method.body, own_type);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for ty in type_names {
        if ty == own_type || ty.is_empty() {
            continue;
        }
        let count = identifier_count(&method.body, ty);
        if count > 0 {
            counts.insert(ty.clone(), count);
        }
    }
    let (target, foreign_count) = counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
    let threshold = own_count.saturating_mul(3).max(3);
    (foreign_count >= threshold).then_some((target, foreign_count, own_count))
}

fn identifier_count(text: &str, ident: &str) -> usize {
    text.split(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .filter(|token| *token == ident)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_complex_function_ranks_extract_method_first() {
        let mut src = String::from("fn hard(x: i32) -> i32 {\n    let mut y = x;\n");
        for i in 0..14 {
            src.push_str(&format!("    if y > {i} {{ y += 1; }}\n"));
        }
        for _ in 0..65 {
            src.push_str("    y += 0;\n");
        }
        src.push_str("    y\n}\n");

        let suggestions = suggest_refactorings("rust", &src);
        assert_eq!(suggestions[0].kind, RefactoringKind::ExtractMethod);
        assert_eq!(suggestions[0].target, "hard");
    }

    #[test]
    fn many_functions_yield_split_file() {
        let src = (0..15)
            .map(|i| format!("fn f{i}() -> i32 {{ {i} }}\n"))
            .collect::<String>();

        let suggestions = suggest_refactorings("rust", &src);
        assert!(suggestions
            .iter()
            .any(|s| s.kind == RefactoringKind::SplitFile));
    }

    #[test]
    fn cycle_members_yield_one_break_cycle() {
        let suggestions = break_cycle_suggestions(&[vec!["a".into(), "b".into(), "c".into()]]);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].kind, RefactoringKind::BreakCycle);
    }
}
