use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct JavaExtractor;

impl JavaExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for JavaExtractor {
    fn default() -> Self {
        JavaExtractor
    }
}

impl LangExtractor for JavaExtractor {
    fn language(&self) -> &'static str {
        "java"
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
        match child.kind() {
            "superclass" | "super_interfaces" | "extends_interfaces" => {
                let mut tc = child.walk();
                for t in child.named_children(&mut tc) {
                    collect_type_names(t, bytes, &mut out);
                }
            }
            _ => {}
        }
    }
    out
}

fn collect_type_names(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "type_identifier" | "scoped_type_identifier" | "identifier" => {
            out.push(node_text(node, bytes).to_string())
        }
        "type_list" | "generic_type" => {
            let mut c = node.walk();
            for ch in node.named_children(&mut c) {
                collect_type_names(ch, bytes, out);
            }
        }
        _ => {}
    }
}

fn push(
    symbols: &mut Vec<SymbolNode>,
    prefix: &[String],
    name: String,
    kind: SymbolKind,
    is_class: bool,
    derived: Vec<String>,
    node: Node,
) -> Vec<String> {
    let mut path = prefix.to_vec();
    path.push(name.clone());
    symbols.push(SymbolNode {
        official_path: path.clone(),
        kind,
        cpath: String::new(),
        decl_line1: line1(node),
        decl_line2: line1(node),
        body_line1: line1(node),
        body_line2: line2(node),
        this_is_a_class: if is_class { name } else { String::new() },
        this_class_derived_from: derived,
    });
    path
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
        | "enum_declaration"
        | "record_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let derived = heritage(node, bytes);
                let child_prefix = push(
                    symbols,
                    prefix,
                    name,
                    SymbolKind::Struct,
                    true,
                    derived,
                    node,
                );
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let child_prefix = push(
                    symbols,
                    prefix,
                    name,
                    SymbolKind::Function,
                    false,
                    vec![],
                    node,
                );
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "method_invocation" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                refs.push(RawRef {
                    from: prefix.join("::"),
                    name: node_text(name_node, bytes).to_string(),
                    kind: EdgeKind::Calls,
                    line: line1(node),
                });
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
        let tree = JavaExtractor::parse(source).expect("parse java");
        JavaExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_methods_and_heritage() {
        let src = "\
class Animal {
    void speak() {}
}

class Dog extends Animal {
    void speak() {
        bark();
    }
    void bark() {}
}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()));
        assert!(paths.contains(&"Dog".to_string()));
        assert!(paths.contains(&"Dog::speak".to_string()));
        assert!(paths.contains(&"Dog::bark".to_string()));
        let dog = symbols.iter().find(|s| s.name() == "Dog").unwrap();
        assert_eq!(dog.this_class_derived_from, vec!["Animal".to_string()]);
        assert!(refs
            .iter()
            .any(|r| r.name == "bark" && r.from == "Dog::speak"));
    }
}
