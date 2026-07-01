use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct SwiftExtractor;

impl SwiftExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for SwiftExtractor {
    fn default() -> Self {
        SwiftExtractor
    }
}

impl LangExtractor for SwiftExtractor {
    fn language(&self) -> &'static str {
        "swift"
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

fn heritage(node: Node, bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "type_inheritance_clause" | "inheritance_clause"
        ) {
            collect_types(child, bytes, &mut out);
        }
    }
    out
}

fn collect_types(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "type_identifier" | "user_type" => {
            let text = node_text(node, bytes).trim().to_string();
            if !text.is_empty() {
                out.push(text);
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

fn last_segment(text: &str) -> String {
    text.rsplit(|c| c == '.' || c == ':')
        .next()
        .unwrap_or(text)
        .trim()
        .to_string()
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
        | "protocol_declaration"
        | "struct_declaration"
        | "enum_declaration"
        | "actor_declaration" => {
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
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                } else {
                    let mut c = node.walk();
                    for child in node.children(&mut c) {
                        if child.kind().ends_with("body") || child.kind() == "class_body" {
                            walk(child, bytes, &path, symbols, refs);
                        }
                    }
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
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
                return;
            }
        }
        "call_expression" => {
            if let Some(callee) = node.child(0) {
                let name = last_segment(node_text(callee, bytes));
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
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
        let tree = SwiftExtractor::parse(source).expect("parse swift");
        SwiftExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_and_functions() {
        let src = "\
class Animal {
    func speak() {}
}

func standalone() {
    helper()
}

func helper() {}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()), "got {paths:?}");
        assert!(paths.iter().any(|p| p == "Animal::speak"), "got {paths:?}");
        assert!(paths.contains(&"standalone".to_string()), "got {paths:?}");
        assert!(paths.contains(&"helper".to_string()), "got {paths:?}");
        assert!(refs.iter().any(|r| r.name == "helper"), "got {refs:?}");
    }
}
