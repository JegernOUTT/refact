use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct GoExtractor;

impl GoExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
        parser.parse(source, None)
    }
}

impl Default for GoExtractor {
    fn default() -> Self {
        GoExtractor
    }
}

impl LangExtractor for GoExtractor {
    fn language(&self) -> &'static str {
        "go"
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

fn receiver_type(node: Node, bytes: &[u8]) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    for param in receiver.named_children(&mut cursor) {
        if let Some(ty) = param.child_by_field_name("type") {
            let mut t = ty;
            if t.kind() == "pointer_type" {
                if let Some(inner) = t.named_child(0) {
                    t = inner;
                }
            }
            if matches!(t.kind(), "type_identifier" | "qualified_type") {
                return Some(node_text(t, bytes).to_string());
            }
        }
    }
    None
}

fn push(
    symbols: &mut Vec<SymbolNode>,
    path: Vec<String>,
    kind: SymbolKind,
    is_class: bool,
    node: Node,
) {
    let name = path.last().cloned().unwrap_or_default();
    symbols.push(SymbolNode {
        official_path: path,
        kind,
        cpath: String::new(),
        decl_line1: line1(node),
        decl_line2: line1(node),
        body_line1: line1(node),
        body_line2: line2(node),
        this_is_a_class: if is_class { name } else { String::new() },
        this_class_derived_from: Vec::new(),
        is_override: false,
    });
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mut path = prefix.to_vec();
                path.push(node_text(name_node, bytes).to_string());
                push(symbols, path.clone(), SymbolKind::Function, false, node);
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mut path = prefix.to_vec();
                if let Some(recv) = receiver_type(node, bytes) {
                    path.push(recv);
                }
                path.push(node_text(name_node, bytes).to_string());
                push(symbols, path.clone(), SymbolKind::Function, false, node);
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "type_spec" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mut path = prefix.to_vec();
                path.push(node_text(name_node, bytes).to_string());
                let is_struct_like = node
                    .child_by_field_name("type")
                    .map(|t| matches!(t.kind(), "struct_type" | "interface_type"))
                    .unwrap_or(false);
                let kind = if is_struct_like {
                    SymbolKind::Struct
                } else {
                    SymbolKind::TypeAlias
                };
                push(symbols, path, kind, is_struct_like, node);
            }
            return;
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let callee = match func.kind() {
                    "selector_expression" => func
                        .child_by_field_name("field")
                        .map(|n| node_text(n, bytes).to_string()),
                    "identifier" => Some(node_text(func, bytes).to_string()),
                    _ => None,
                };
                if let Some(name) = callee {
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
        let tree = GoExtractor::parse(source).expect("parse go");
        GoExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_funcs_types_and_methods() {
        let src = "\
package main

type Widget struct {
    name string
}

func (w Widget) Render() {
    helper()
}

func helper() {}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Widget".to_string()), "got {paths:?}");
        assert!(
            paths.contains(&"Widget::Render".to_string()),
            "got {paths:?}"
        );
        assert!(paths.contains(&"helper".to_string()), "got {paths:?}");
        assert!(
            refs.iter()
                .any(|r| r.name == "helper" && r.from == "Widget::Render"),
            "got {refs:?}"
        );
    }
}
