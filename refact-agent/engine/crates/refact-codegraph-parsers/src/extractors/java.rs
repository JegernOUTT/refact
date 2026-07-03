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

fn has_override_annotation(node: Node, bytes: &[u8]) -> bool {
    let name_start = node
        .child_by_field_name("name")
        .map(|name| name.start_byte())
        .unwrap_or(node.end_byte());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() >= name_start {
            break;
        }
        if child.end_byte() <= name_start && node_has_override_annotation(child, bytes) {
            return true;
        }
    }
    false
}

fn node_has_override_annotation(node: Node, bytes: &[u8]) -> bool {
    if matches!(node.kind(), "annotation" | "marker_annotation") {
        return annotation_is_override(node, bytes);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if node_has_override_annotation(child, bytes) {
            return true;
        }
    }
    false
}

fn annotation_is_override(node: Node, bytes: &[u8]) -> bool {
    let name = node
        .child_by_field_name("name")
        .map(|name| node_text(name, bytes).to_string())
        .unwrap_or_else(|| node_text(node, bytes).trim_start_matches('@').to_string());
    name.rsplit('.').next().unwrap_or(&name) == "Override"
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
    is_override: bool,
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
        is_override,
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
                    false,
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
                let is_override = has_override_annotation(node, bytes);
                let child_prefix = push(
                    symbols,
                    prefix,
                    name,
                    SymbolKind::Function,
                    false,
                    vec![],
                    is_override,
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
    use serde_json::Value;

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

    #[test]
    fn marks_override_annotated_methods_in_data_json() {
        let src = "\
interface Command {
    void execute();
}

class RunCommand implements Command {
    @Override
    public void execute() {}

    void helper() {}
}
";
        let (symbols, _refs) = extract(src);
        let execute = symbols
            .iter()
            .find(|s| s.double_colon_path() == "RunCommand::execute")
            .unwrap();
        assert!(execute.is_override);
        let data = serde_json::to_value(execute).unwrap();
        assert_eq!(data.get("override"), Some(&Value::Bool(true)));

        let helper = symbols
            .iter()
            .find(|s| s.double_colon_path() == "RunCommand::helper")
            .unwrap();
        assert!(!helper.is_override);
        let data = serde_json::to_value(helper).unwrap();
        assert!(data.get("override").is_none());
    }
}
