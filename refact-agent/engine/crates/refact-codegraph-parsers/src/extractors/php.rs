use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct PhpExtractor;

impl PhpExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for PhpExtractor {
    fn default() -> Self {
        PhpExtractor
    }
}

impl LangExtractor for PhpExtractor {
    fn language(&self) -> &'static str {
        "php"
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

fn heritage(node: Node, bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "base_clause" | "class_interface_clause") {
            let mut c = child.walk();
            for n in child.named_children(&mut c) {
                if matches!(n.kind(), "name" | "qualified_name") {
                    out.push(node_text(n, bytes).to_string());
                }
            }
        }
    }
    out
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let mut path = prefix.to_vec();
                path.push(name.clone());
                symbols.push(SymbolNode {
                    official_path: path.clone(),
                    kind: SymbolKind::Struct,
                    cpath: String::new(),
                    decl_line1: line1(node),
                    decl_line2: line1(node),
                    body_line1: line1(node),
                    body_line2: line2(node),
                    this_is_a_class: name,
                    this_class_derived_from: heritage(node, bytes),
                    is_override: false,
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "method_declaration" | "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mut path = prefix.to_vec();
                path.push(node_text(name_node, bytes).to_string());
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
                    is_override: false,
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "function_call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let name = node_text(func, bytes)
                    .rsplit('\\')
                    .next()
                    .unwrap_or("")
                    .to_string();
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
        "member_call_expression" | "scoped_call_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
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
        let tree = PhpExtractor::parse(source).expect("parse php");
        PhpExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_methods_and_heritage() {
        let src = "<?php
class Animal {
    public function speak() {}
}

class Dog extends Animal {
    public function speak() {
        helper();
    }
}

function helper() {}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()), "got {paths:?}");
        assert!(paths.contains(&"Dog".to_string()), "got {paths:?}");
        assert!(paths.contains(&"Dog::speak".to_string()), "got {paths:?}");
        assert!(paths.contains(&"helper".to_string()), "got {paths:?}");
        let dog = symbols.iter().find(|s| s.name() == "Dog").unwrap();
        assert!(
            dog.this_class_derived_from.contains(&"Animal".to_string()),
            "got {:?}",
            dog.this_class_derived_from
        );
        assert!(refs.iter().any(|r| r.name == "helper"));
    }
}
