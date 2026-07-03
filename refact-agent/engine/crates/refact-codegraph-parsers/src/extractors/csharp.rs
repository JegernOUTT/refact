use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct CSharpExtractor;

impl CSharpExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for CSharpExtractor {
    fn default() -> Self {
        CSharpExtractor
    }
}

impl LangExtractor for CSharpExtractor {
    fn language(&self) -> &'static str {
        "csharp"
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
        if matches!(child.kind(), "base_list" | "primary_constructor_base_type") {
            let mut c = child.walk();
            for b in child.named_children(&mut c) {
                match b.kind() {
                    "identifier" | "generic_name" | "qualified_name" => {
                        out.push(node_text(b, bytes).to_string())
                    }
                    _ => {}
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
        | "struct_declaration"
        | "record_declaration"
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
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
            }
            return;
        }
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mut path = prefix.to_vec();
                path.push(node_text(name_node, bytes).to_string());
                symbols.push(SymbolNode {
                    official_path: path.clone(),
                    kind: SymbolKind::Module,
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
        "method_declaration" | "constructor_declaration" | "local_function_statement" => {
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
        "invocation_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let callee = match func.kind() {
                    "member_access_expression" => func
                        .child_by_field_name("name")
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
        let tree = CSharpExtractor::parse(source).expect("parse csharp");
        CSharpExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_methods_and_heritage() {
        let src = "\
class Animal {
    public void Speak() {}
}

class Dog : Animal {
    public void Bark() {
        Helper();
    }
    void Helper() {}
}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()), "got {paths:?}");
        assert!(paths.contains(&"Dog".to_string()), "got {paths:?}");
        assert!(paths.contains(&"Dog::Bark".to_string()), "got {paths:?}");
        let dog = symbols.iter().find(|s| s.name() == "Dog").unwrap();
        assert!(
            dog.this_class_derived_from.contains(&"Animal".to_string()),
            "got {:?}",
            dog.this_class_derived_from
        );
        assert!(refs
            .iter()
            .any(|r| r.name == "Helper" && r.from == "Dog::Bark"));
    }
}
