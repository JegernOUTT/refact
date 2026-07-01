use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct RubyExtractor;

impl RubyExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for RubyExtractor {
    fn default() -> Self {
        RubyExtractor
    }
}

impl LangExtractor for RubyExtractor {
    fn language(&self) -> &'static str {
        "ruby"
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

fn superclass(node: Node, bytes: &[u8]) -> Vec<String> {
    if let Some(sc) = node.child_by_field_name("superclass") {
        let mut c = sc.walk();
        for ch in sc.named_children(&mut c) {
            if matches!(ch.kind(), "constant" | "scope_resolution") {
                return vec![node_text(ch, bytes).to_string()];
            }
        }
    }
    Vec::new()
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "class" | "module" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let mut path = prefix.to_vec();
                path.push(name.clone());
                let is_class = node.kind() == "class";
                symbols.push(SymbolNode {
                    official_path: path.clone(),
                    kind: if is_class {
                        SymbolKind::Struct
                    } else {
                        SymbolKind::Module
                    },
                    cpath: String::new(),
                    decl_line1: line1(node),
                    decl_line2: line1(node),
                    body_line1: line1(node),
                    body_line2: line2(node),
                    this_is_a_class: if is_class { name } else { String::new() },
                    this_class_derived_from: if is_class {
                        superclass(node, bytes)
                    } else {
                        Vec::new()
                    },
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "method" | "singleton_method" => {
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
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "call" => {
            if let Some(method) = node.child_by_field_name("method") {
                let name = node_text(method, bytes).to_string();
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
        let tree = RubyExtractor::parse(source).expect("parse ruby");
        RubyExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_methods_and_heritage() {
        let src = "\
class Animal
  def speak
  end
end

class Dog < Animal
  def speak
    bark
  end

  def bark
  end
end
";
        let (symbols, _refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()), "got {paths:?}");
        assert!(paths.contains(&"Dog".to_string()), "got {paths:?}");
        assert!(paths.contains(&"Dog::speak".to_string()), "got {paths:?}");
        let dog = symbols.iter().find(|s| s.name() == "Dog").unwrap();
        assert!(
            dog.this_class_derived_from.contains(&"Animal".to_string()),
            "got {:?}",
            dog.this_class_derived_from
        );
    }
}
