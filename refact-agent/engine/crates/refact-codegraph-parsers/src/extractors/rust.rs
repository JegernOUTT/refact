use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct RustExtractor;

impl RustExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for RustExtractor {
    fn default() -> Self {
        RustExtractor
    }
}

impl LangExtractor for RustExtractor {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn extract(&self, tree: &Tree, source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let mut symbols = Vec::new();
        let mut refs = Vec::new();
        let bytes = source.as_bytes();
        walk(tree.root_node(), bytes, &[], &mut symbols, &mut refs);
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
    is_class: bool,
    node: Node,
) -> (SymbolNode, Vec<String>) {
    let mut official_path = prefix.to_vec();
    official_path.push(name.clone());
    let symbol = SymbolNode {
        official_path: official_path.clone(),
        kind,
        cpath: String::new(),
        decl_line1: line1(node),
        decl_line2: line1(node),
        body_line1: line1(node),
        body_line2: line2(node),
        this_is_a_class: if is_class { name } else { String::new() },
        this_class_derived_from: Vec::new(),
    };
    (symbol, official_path)
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let (symbol, child_prefix) =
                    make_symbol(prefix, name, SymbolKind::Function, false, node);
                symbols.push(symbol);
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "struct_item" | "enum_item" | "union_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let (symbol, _) = make_symbol(prefix, name, SymbolKind::Struct, true, node);
                symbols.push(symbol);
            }
            return;
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let (symbol, _) = make_symbol(prefix, name, SymbolKind::TypeAlias, false, node);
                symbols.push(symbol);
            }
            return;
        }
        "trait_item" | "mod_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let is_class = node.kind() == "trait_item";
                let kind = if is_class {
                    SymbolKind::Struct
                } else {
                    SymbolKind::Module
                };
                let (symbol, child_prefix) = make_symbol(prefix, name, kind, is_class, node);
                symbols.push(symbol);
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "impl_item" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = node_text(type_node, bytes).to_string();
                let mut child_prefix = prefix.to_vec();
                child_prefix.push(type_name);
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let callee = callee_name(func, bytes);
                if !callee.is_empty() {
                    refs.push(RawRef {
                        from: prefix.join("::"),
                        name: callee,
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

fn callee_name(func: Node, bytes: &[u8]) -> String {
    match func.kind() {
        "field_expression" => func
            .child_by_field_name("field")
            .map(|n| node_text(n, bytes).to_string())
            .unwrap_or_default(),
        _ => node_text(func, bytes).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = RustExtractor::parse(source).expect("parse rust");
        RustExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_free_functions_and_structs() {
        let src = "\
struct User {
    name: String,
}

fn create_user() -> User {
    User { name: String::new() }
}
";
        let (symbols, _refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"User".to_string()));
        assert!(paths.contains(&"create_user".to_string()));
        let user = symbols.iter().find(|s| s.name() == "User").unwrap();
        assert_eq!(user.kind, SymbolKind::Struct);
        assert_eq!(user.this_is_a_class, "User");
        let func = symbols.iter().find(|s| s.name() == "create_user").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_impl_methods_with_type_prefix() {
        let src = "\
struct Widget;

impl Widget {
    fn new() -> Self {
        Widget
    }
    fn render(&self) {
        helper();
    }
}

fn helper() {}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Widget::new".to_string()));
        assert!(paths.contains(&"Widget::render".to_string()));
        let call = refs.iter().find(|r| r.name == "helper").unwrap();
        assert_eq!(call.from, "Widget::render");
        assert_eq!(call.kind, EdgeKind::Calls);
    }

    #[test]
    fn extracts_call_references_with_enclosing_symbol() {
        let src = "\
fn a() {
    b();
}

fn b() {}
";
        let (_symbols, refs) = extract(src);
        let call = refs.iter().find(|r| r.name == "b").unwrap();
        assert_eq!(call.from, "a");
    }
}
