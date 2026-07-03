pub mod assertions;
pub mod biomarkers;
pub mod coverage;
pub mod coverage_biomarkers;
pub mod dry;
pub mod duplication;
pub mod git_biomarkers;
pub mod markers;
pub mod perf;
pub mod refactoring;
pub mod scoring;
pub mod test_smells;
pub mod trends;

use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionHealth {
    pub name: String,
    pub line1: usize,
    pub complexity: u32,
    pub nesting: u32,
    pub loc: u32,
    pub maintainability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileHealth {
    pub functions: Vec<FunctionHealth>,
    pub max_complexity: u32,
    pub avg_maintainability: f64,
}

pub fn maintainability_score(complexity: u32, nesting: u32, loc: u32) -> f64 {
    let raw = 100.0
        - 2.5 * (complexity.saturating_sub(1) as f64)
        - 6.0 * (nesting as f64)
        - 0.25 * (loc as f64);
    raw.clamp(0.0, 100.0)
}

fn function_name(node: Node, bytes: &[u8]) -> String {
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(bytes).unwrap_or("").to_string();
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "simple_identifier" | "field_identifier"
        ) {
            return child.utf8_text(bytes).unwrap_or("").to_string();
        }
    }
    String::new()
}

fn collect_functions(node: Node, bytes: &[u8], out: &mut Vec<FunctionHealth>) {
    if FUNCTION_KINDS.contains(&node.kind()) {
        let complexity = markers::cyclomatic_complexity(node);
        let nesting = markers::max_nesting(node);
        let loc = markers::lines_of_code(node);
        out.push(FunctionHealth {
            name: function_name(node, bytes),
            line1: node.start_position().row + 1,
            complexity,
            nesting,
            loc,
            maintainability: maintainability_score(complexity, nesting, loc),
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, bytes, out);
    }
}

pub fn analyze(lang: &str, text: &str) -> FileHealth {
    let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) else {
        return FileHealth {
            functions: vec![],
            max_complexity: 0,
            avg_maintainability: 100.0,
        };
    };
    let mut functions = Vec::new();
    collect_functions(tree.root_node(), text.as_bytes(), &mut functions);

    let max_complexity = functions.iter().map(|f| f.complexity).max().unwrap_or(0);
    let avg_maintainability = if functions.is_empty() {
        100.0
    } else {
        functions.iter().map(|f| f.maintainability).sum::<f64>() / functions.len() as f64
    };

    FileHealth {
        functions,
        max_complexity,
        avg_maintainability,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_function_is_low_complexity_high_maintainability() {
        let src = "fn simple() -> i32 { 1 }\n";
        let health = analyze("rust", src);
        let f = health
            .functions
            .iter()
            .find(|f| f.name == "simple")
            .unwrap();
        assert_eq!(f.complexity, 1, "no branches => complexity 1");
        assert_eq!(f.nesting, 0);
        assert!(f.maintainability > 90.0, "got {}", f.maintainability);
    }

    #[test]
    fn branchy_nested_function_is_higher_complexity_lower_maintainability() {
        let simple_src = "fn simple() -> i32 { 1 }\n";
        let branchy_src = "\
fn branchy(x: i32) -> i32 {
    if x > 0 {
        if x > 10 {
            for _i in 0..x {
                if x % 2 == 0 && x > 5 {
                    return 1;
                }
            }
        }
    }
    0
}
";
        let simple = analyze("rust", simple_src);
        let branchy = analyze("rust", branchy_src);
        let s = &simple.functions[0];
        let b = branchy
            .functions
            .iter()
            .find(|f| f.name == "branchy")
            .unwrap();
        assert!(
            b.complexity > s.complexity,
            "branchy {} > simple {}",
            b.complexity,
            s.complexity
        );
        assert!(b.nesting >= 3, "deeply nested, got {}", b.nesting);
        assert!(
            b.maintainability < s.maintainability,
            "branchy {} < simple {}",
            b.maintainability,
            s.maintainability
        );
    }

    #[test]
    fn python_methods_are_detected() {
        let src = "\
class Foo:
    def method_a(self):
        if True:
            pass

    def method_b(self):
        pass
";
        let health = analyze("python", src);
        let names: Vec<String> = health.functions.iter().map(|f| f.name.clone()).collect();
        assert!(names.contains(&"method_a".to_string()), "got {names:?}");
        assert!(names.contains(&"method_b".to_string()), "got {names:?}");
    }

    #[test]
    fn nested_local_functions_are_not_enumerated_separately() {
        let src = "\
def outer():
    def inner():
        return 1
    return inner()
";
        let health = analyze("python", src);
        let names: Vec<String> = health.functions.iter().map(|f| f.name.clone()).collect();

        assert_eq!(names, vec!["outer".to_string()]);
    }

    #[test]
    fn empty_or_unknown_lang_is_safe() {
        let health = analyze("unknown", "whatever");
        assert!(health.functions.is_empty());
        assert_eq!(health.avg_maintainability, 100.0);
    }
}
