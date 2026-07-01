use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct KotlinExtractor;

impl KotlinExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for KotlinExtractor {
    fn default() -> Self {
        KotlinExtractor
    }
}

impl LangExtractor for KotlinExtractor {
    fn language(&self) -> &'static str {
        "kotlin"
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

fn name_of(node: Node, bytes: &[u8]) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(node_text(n, bytes).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "type_identifier" | "simple_identifier") {
            return Some(node_text(child, bytes).to_string());
        }
    }
    None
}

fn body_of<'a>(node: Node<'a>) -> Option<Node<'a>> {
    for field in ["body", "class_body", "members"] {
        if let Some(b) = node.child_by_field_name(field) {
            return Some(b);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "class_body" | "function_body" | "enum_class_body"
        ) {
            return Some(child);
        }
    }
    None
}

fn heritage(node: Node, bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "delegation_specifiers" | "delegation_specifier" | "supertype_list"
        ) {
            collect_types(child, bytes, &mut out);
        }
    }
    out
}

fn collect_types(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "type_identifier" => out.push(node_text(node, bytes).to_string()),
        "user_type" => {
            if let Some(t) = node.child(0).filter(|c| c.kind() == "type_identifier") {
                out.push(node_text(t, bytes).to_string());
            } else {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    collect_types(ch, bytes, out);
                }
            }
        }
        _ => {
            let mut c = node.walk();
            for ch in node.named_children(&mut c) {
                collect_types(ch, bytes, out);
            }
        }
    }
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "class_declaration" | "object_declaration" | "interface_declaration" => {
            if let Some(name) = name_of(node, bytes) {
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
                });
                if let Some(body) = body_of(node) {
                    walk(body, bytes, &path, symbols, refs);
                }
                return;
            }
        }
        "function_declaration" => {
            if let Some(name) = name_of(node, bytes) {
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
                if let Some(body) = body_of(node) {
                    walk(body, bytes, &path, symbols, refs);
                }
                return;
            }
        }
        "call_expression" => {
            if let Some(callee) = node.child(0) {
                let text = node_text(callee, bytes);
                let last = text
                    .rsplit(|c| c == '.' || c == ':')
                    .next()
                    .unwrap_or(text)
                    .trim();
                if !last.is_empty() && last.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    refs.push(RawRef {
                        from: prefix.join("::"),
                        name: last.to_string(),
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
        let tree = KotlinExtractor::parse(source).expect("parse kotlin");
        KotlinExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_and_functions() {
        let src = "\
class Animal {
    fun speak() {}
}

fun standalone() {
    helper()
}

fun helper() {}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()), "got {paths:?}");
        assert!(
            paths.contains(&"Animal::speak".to_string()),
            "got {paths:?}"
        );
        assert!(paths.contains(&"standalone".to_string()), "got {paths:?}");
        assert!(paths.contains(&"helper".to_string()), "got {paths:?}");
        assert!(
            refs.iter().any(|r| r.name == "helper"),
            "expected helper call, got {refs:?}"
        );
    }
}
