use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct PythonExtractor;

impl PythonExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for PythonExtractor {
    fn default() -> Self {
        PythonExtractor
    }
}

impl LangExtractor for PythonExtractor {
    fn language(&self) -> &'static str {
        "python"
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

fn superclasses(node: Node, bytes: &[u8]) -> Vec<String> {
    let Some(args) = node.child_by_field_name("superclasses") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        match child.kind() {
            "identifier" | "attribute" => out.push(node_text(child, bytes).to_string()),
            _ => {}
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
        "decorated_definition" => {
            if let Some(def) = node.child_by_field_name("definition") {
                walk(def, bytes, prefix, symbols, refs);
            }
            return;
        }
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let mut child_prefix = prefix.to_vec();
                child_prefix.push(name.clone());
                symbols.push(SymbolNode {
                    official_path: child_prefix.clone(),
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
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let mut child_prefix = prefix.to_vec();
                child_prefix.push(name.clone());
                symbols.push(SymbolNode {
                    official_path: child_prefix.clone(),
                    kind: SymbolKind::Struct,
                    cpath: String::new(),
                    decl_line1: line1(node),
                    decl_line2: line1(node),
                    body_line1: line1(node),
                    body_line2: line2(node),
                    this_is_a_class: name,
                    this_class_derived_from: superclasses(node, bytes),
                    is_override: false,
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &child_prefix, symbols, refs);
                }
            }
            return;
        }
        "call" => {
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
        "attribute" => func
            .child_by_field_name("attribute")
            .map(|n| node_text(n, bytes).to_string())
            .unwrap_or_default(),
        _ => node_text(func, bytes).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = PythonExtractor::parse(source).expect("parse python");
        PythonExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_functions_classes_and_methods() {
        let src = "\
class Animal:
    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        bark()

def bark():
    pass
";
        let (symbols, _refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()));
        assert!(paths.contains(&"Animal::speak".to_string()));
        assert!(paths.contains(&"Dog".to_string()));
        assert!(paths.contains(&"Dog::speak".to_string()));
        assert!(paths.contains(&"bark".to_string()));
    }

    #[test]
    fn extracts_class_heritage() {
        let src = "\
class Base:
    pass

class Derived(Base):
    pass
";
        let (symbols, _refs) = extract(src);
        let derived = symbols.iter().find(|s| s.name() == "Derived").unwrap();
        assert_eq!(derived.this_is_a_class, "Derived");
        assert_eq!(derived.this_class_derived_from, vec!["Base".to_string()]);
    }

    #[test]
    fn extracts_calls_with_enclosing_symbol() {
        let src = "\
def caller():
    callee()

def callee():
    pass
";
        let (_symbols, refs) = extract(src);
        let call = refs.iter().find(|r| r.name == "callee").unwrap();
        assert_eq!(call.from, "caller");
        assert_eq!(call.kind, EdgeKind::Calls);
    }

    #[test]
    fn decorated_functions_are_extracted() {
        let src = "\
@decorator
def decorated_fn():
    pass
";
        let (symbols, _refs) = extract(src);
        assert!(symbols.iter().any(|s| s.name() == "decorated_fn"));
    }

    #[test]
    fn method_call_uses_attribute_name() {
        let src = "\
def run(self):
    self.helper()
";
        let (_symbols, refs) = extract(src);
        assert!(refs.iter().any(|r| r.name == "helper" && r.from == "run"));
    }
}
