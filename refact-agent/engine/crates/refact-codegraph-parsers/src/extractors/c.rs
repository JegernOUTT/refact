use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct CExtractor;

impl CExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_c::LANGUAGE.into()).ok()?;
        parser.parse(source, None)
    }
}

impl Default for CExtractor {
    fn default() -> Self {
        CExtractor
    }
}

impl LangExtractor for CExtractor {
    fn language(&self) -> &'static str {
        "c"
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

fn declarator_name(node: Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" => {
            Some(node_text(node, bytes).to_string())
        }
        "function_declarator"
        | "pointer_declarator"
        | "parenthesized_declarator"
        | "init_declarator"
        | "array_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|n| declarator_name(n, bytes)),
        _ => None,
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
        "function_definition" => {
            if let Some(decl) = node.child_by_field_name("declarator") {
                if let Some(name) = declarator_name(decl, bytes) {
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
        }
        "struct_specifier" | "enum_specifier" | "union_specifier" => {
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
                    this_class_derived_from: Vec::new(),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
                return;
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let callee = match func.kind() {
                    "field_expression" => func
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
        let tree = CExtractor::parse(source).expect("parse c");
        CExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_functions_and_calls() {
        let src = "int helper(){return 1;} int main(){return helper();}";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"helper".to_string()), "got {paths:?}");
        assert!(paths.contains(&"main".to_string()), "got {paths:?}");
        assert!(
            refs.iter().any(|r| r.name == "helper" && r.from == "main"),
            "got {refs:?}"
        );
    }

    #[test]
    fn extracts_structs() {
        let src = "struct Point { int x; };";
        let (symbols, _refs) = extract(src);
        assert!(symbols.iter().any(|s| s.name() == "Point"));
    }
}
