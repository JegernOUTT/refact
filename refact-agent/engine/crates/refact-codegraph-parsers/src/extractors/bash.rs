use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct BashExtractor;

impl BashExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for BashExtractor {
    fn default() -> Self {
        BashExtractor
    }
}

impl LangExtractor for BashExtractor {
    fn language(&self) -> &'static str {
        "bash"
    }

    fn extract(&self, tree: &Tree, source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let mut symbols = Vec::new();
        let mut refs = Vec::new();
        walk(
            tree.root_node(),
            source.as_bytes(),
            &[],
            &mut symbols,
            &mut refs,
        );
        (symbols, refs)
    }
}

fn node_text<'a>(node: Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn line1(node: Node) -> usize {
    node.start_position().row + 1
}

fn line2(node: Node) -> usize {
    node.end_position().row + 1
}

fn command_name(node: Node, bytes: &[u8]) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(node_text(name, bytes).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "command_name" | "word" | "identifier") {
            return Some(node_text(child, bytes).to_string());
        }
    }
    None
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let mut path = prefix.to_vec();
                path.push(name);
                symbols.push(SymbolNode {
                    official_path: path.clone(),
                    kind: SymbolKind::Function,
                    cpath: String::new(),
                    decl_line1: line1(node),
                    decl_line2: line1(node),
                    body_line1: line1(node),
                    body_line2: line2(node),
                    this_is_a_class: String::new(),
                    this_class_derived_from: Vec::new(),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
                return;
            }
        }
        "command" => {
            if let Some(name) = command_name(node, bytes) {
                if !name.is_empty() {
                    refs.push(RawRef {
                        from: prefix.join("::"),
                        name,
                        kind: EdgeKind::Calls,
                        line: line1(node),
                    });
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, bytes, prefix, symbols, refs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = BashExtractor::parse(source).expect("parse bash");
        BashExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_functions_and_command_calls() {
        let src = "greet() { echo hi; }\ngreet";
        let (symbols, refs) = extract(src);
        assert!(symbols.iter().any(|s| s.name() == "greet"));
        assert!(refs.iter().any(|r| r.name == "greet"), "got {refs:?}");
    }
}
