use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct ElixirExtractor;

impl ElixirExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_elixir::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for ElixirExtractor {
    fn default() -> Self {
        ElixirExtractor
    }
}

impl LangExtractor for ElixirExtractor {
    fn language(&self) -> &'static str {
        "elixir"
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

fn make_symbol(
    prefix: &[String],
    name: String,
    kind: SymbolKind,
    node: Node,
) -> (SymbolNode, Vec<String>) {
    let mut path = prefix.to_vec();
    path.push(name);
    let symbol = SymbolNode {
        official_path: path.clone(),
        kind,
        cpath: String::new(),
        decl_line1: line1(node),
        decl_line2: line1(node),
        body_line1: line1(node),
        body_line2: line2(node),
        this_is_a_class: String::new(),
        this_class_derived_from: Vec::new(),
    };
    (symbol, path)
}

fn target_node(node: Node) -> Option<Node> {
    node.child_by_field_name("target")
        .or_else(|| node.named_child(0))
}

fn call_target(node: Node, bytes: &[u8]) -> Option<String> {
    let target = target_node(node)?;
    match target.kind() {
        "identifier" => Some(node_text(target, bytes).to_string()),
        "dot" => {
            let mut cursor = target.walk();
            target
                .named_children(&mut cursor)
                .last()
                .map(|n| node_text(n, bytes).to_string())
        }
        _ => None,
    }
}

fn find_named_descendant(node: Node, bytes: &[u8], kinds: &[&str]) -> Option<String> {
    if kinds.contains(&node.kind()) {
        let text = node_text(node, bytes).to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(name) = find_named_descendant(child, bytes, kinds) {
            return Some(name);
        }
    }
    None
}

fn module_name(node: Node, bytes: &[u8]) -> Option<String> {
    let target = target_node(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if Some(child) == target {
            continue;
        }
        if let Some(name) = find_named_descendant(child, bytes, &["alias", "atom"]) {
            return Some(name.trim_start_matches(':').to_string());
        }
    }
    None
}

fn function_name(node: Node, bytes: &[u8]) -> Option<String> {
    let target = target_node(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if Some(child) == target {
            continue;
        }
        if child.kind() == "call" {
            if let Some(name) = call_target(child, bytes) {
                return Some(name);
            }
        }
        if child.kind() == "identifier" {
            return Some(node_text(child, bytes).to_string());
        }
        if let Some(name) = find_named_descendant(child, bytes, &["identifier"]) {
            return Some(name);
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
    if node.kind() == "call" {
        if let Some(target) = call_target(node, bytes) {
            match target.as_str() {
                "defmodule" => {
                    if let Some(name) = module_name(node, bytes) {
                        let (symbol, child_prefix) =
                            make_symbol(prefix, name, SymbolKind::Module, node);
                        symbols.push(symbol);
                        walk_children(node, bytes, &child_prefix, symbols, refs);
                        return;
                    }
                }
                "def" | "defp" | "defmacro" | "defmacrop" => {
                    if let Some(name) = function_name(node, bytes) {
                        let (symbol, child_prefix) =
                            make_symbol(prefix, name, SymbolKind::Function, node);
                        symbols.push(symbol);
                        walk_children(node, bytes, &child_prefix, symbols, refs);
                        return;
                    }
                }
                _ => {
                    refs.push(RawRef {
                        from: prefix.join("::"),
                        name: target,
                        kind: EdgeKind::Calls,
                        line: line1(node),
                    });
                }
            }
        }
    }

    walk_children(node, bytes, prefix, symbols, refs);
}

fn walk_children(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, bytes, prefix, symbols, refs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = ElixirExtractor::parse(source).expect("parse elixir");
        ElixirExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_modules_functions_and_calls() {
        let src = "defmodule M do\n  def run do\n    helper()\n  end\n  def helper do\n  end\nend";
        let (symbols, refs) = extract(src);
        let names: Vec<String> = symbols.iter().map(|s| s.name().to_string()).collect();
        assert!(names.contains(&"M".to_string()), "got {names:?}");
        assert!(
            names.contains(&"run".to_string()) || names.contains(&"helper".to_string()),
            "got {names:?}"
        );
        assert!(refs.iter().any(|r| r.name == "helper"), "got {refs:?}");
    }
}
