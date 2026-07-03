use std::collections::HashSet;

use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct OcamlExtractor;

impl OcamlExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for OcamlExtractor {
    fn default() -> Self {
        OcamlExtractor
    }
}

impl LangExtractor for OcamlExtractor {
    fn language(&self) -> &'static str {
        "ocaml"
    }

    fn extract(&self, tree: &Tree, source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let mut symbols = Vec::new();
        let mut refs = Vec::new();
        let mut seen = HashSet::new();
        let bytes = source.as_bytes();
        walk(
            tree.root_node(),
            bytes,
            &[],
            &mut symbols,
            &mut refs,
            &mut seen,
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

fn make_symbol(
    prefix: &[String],
    name: String,
    kind: SymbolKind,
    node: Node,
) -> (SymbolNode, Vec<String>) {
    let mut official_path = prefix.to_vec();
    official_path.push(name);
    let symbol = SymbolNode {
        official_path: official_path.clone(),
        kind,
        cpath: String::new(),
        decl_line1: line1(node),
        decl_line2: line1(node),
        body_line1: line1(node),
        body_line2: line2(node),
        this_is_a_class: String::new(),
        this_class_derived_from: Vec::new(),
        is_override: false,
    };
    (symbol, official_path)
}

fn push_symbol(symbols: &mut Vec<SymbolNode>, seen: &mut HashSet<String>, symbol: SymbolNode) {
    let key = format!("{}:{:?}", symbol.double_colon_path(), symbol.kind);
    if seen.insert(key) {
        symbols.push(symbol);
    }
}

fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
    seen: &mut HashSet<String>,
) {
    match node.kind() {
        "let_binding" => {
            if let Some(name_node) = binding_name(node, bytes) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() {
                    let (symbol, child_prefix) =
                        make_symbol(prefix, name, SymbolKind::Function, node);
                    push_symbol(symbols, seen, symbol);
                    walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                    return;
                }
            }
        }
        "value_definition" => {
            if find_descendant(node, &["let_binding"]).is_none() {
                if let Some(name_node) = binding_name(node, bytes) {
                    let name = clean_name(node_text(name_node, bytes));
                    if !name.is_empty() {
                        let (symbol, child_prefix) =
                            make_symbol(prefix, name, SymbolKind::Function, node);
                        push_symbol(symbols, seen, symbol);
                        walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                        return;
                    }
                }
            }
        }
        "module_binding" => {
            if let Some(name_node) = module_name(node) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() {
                    let (symbol, child_prefix) =
                        make_symbol(prefix, name, SymbolKind::Module, node);
                    push_symbol(symbols, seen, symbol);
                    walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                    return;
                }
            }
        }
        "module_definition" => {
            if find_descendant(node, &["module_binding"]).is_none() {
                if let Some(name_node) = module_name(node) {
                    let name = clean_name(node_text(name_node, bytes));
                    if !name.is_empty() {
                        let (symbol, child_prefix) =
                            make_symbol(prefix, name, SymbolKind::Module, node);
                        push_symbol(symbols, seen, symbol);
                        walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                        return;
                    }
                }
            }
        }
        "type_definition" => {
            if let Some(name_node) = type_name(node) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() {
                    let kind = if node_text(node, bytes).contains('=') {
                        SymbolKind::Struct
                    } else {
                        SymbolKind::TypeAlias
                    };
                    let (symbol, _) = make_symbol(prefix, name, kind, node);
                    push_symbol(symbols, seen, symbol);
                }
            }
        }
        "application_expression" => {
            if let Some(func) = first_named_child(node) {
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

    walk_children(node, bytes, prefix, symbols, refs, seen);
}

fn walk_children(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
    seen: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, bytes, prefix, symbols, refs, seen);
    }
}

fn binding_name<'a>(node: Node<'a>, bytes: &[u8]) -> Option<Node<'a>> {
    for field in ["name", "pattern", "left"] {
        if let Some(child) = node.child_by_field_name(field) {
            if is_name_like(child.kind()) && is_value_like(node_text(child, bytes)) {
                return Some(child);
            }
            if let Some(found) = find_name_like(child, bytes, true) {
                return Some(found);
            }
        }
    }
    find_name_like(node, bytes, true)
}

fn module_name<'a>(node: Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("name").or_else(|| {
        find_descendant(
            node,
            &[
                "module_name",
                "module_identifier",
                "capitalized_identifier",
                "uident",
            ],
        )
    })
}

fn type_name<'a>(node: Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("name")
        .or_else(|| find_descendant(node, &["type_constructor", "type_identifier", "type_name"]))
}

fn find_name_like<'a>(node: Node<'a>, bytes: &[u8], value_only: bool) -> Option<Node<'a>> {
    if is_name_like(node.kind()) && (!value_only || is_value_like(node_text(node, bytes))) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_name_like(child, bytes, value_only) {
            return Some(found);
        }
    }
    None
}

fn find_descendant<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    if kinds.contains(&node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_descendant(child, kinds) {
            return Some(found);
        }
    }
    None
}

fn first_named_child(node: Node) -> Option<Node> {
    node.named_child(0)
}

fn is_name_like(kind: &str) -> bool {
    matches!(
        kind,
        "value_name"
            | "identifier"
            | "lowercase_identifier"
            | "lident"
            | "operator_name"
            | "module_name"
            | "module_identifier"
            | "capitalized_identifier"
            | "uident"
    )
}

fn is_value_like(text: &str) -> bool {
    text.chars()
        .next()
        .map(|c| c == '_' || c.is_ascii_lowercase() || c == '(')
        .unwrap_or(false)
}

fn callee_name(func: Node, bytes: &[u8]) -> String {
    let text = clean_name(node_text(func, bytes));
    let last = text
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(&text)
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_string();
    if is_value_like(&last) {
        last
    } else {
        String::new()
    }
}

fn clean_name(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = OcamlExtractor::parse(source).expect("parse ocaml");
        OcamlExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_lets_modules_types_and_calls() {
        let src = "let add x y = x + y\nlet main () = add 1 2\nmodule M = struct let f () = () end\ntype color = Red | Blue\n";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"add".to_string()));
        assert!(paths.contains(&"main".to_string()));
        assert!(paths.contains(&"M".to_string()));
        assert!(paths.contains(&"M::f".to_string()));
        assert!(paths.contains(&"color".to_string()));
        assert!(symbols
            .iter()
            .any(|s| s.name() == "add" && s.kind == SymbolKind::Function));
        assert!(refs
            .iter()
            .any(|r| r.name == "add" && r.kind == EdgeKind::Calls));
    }
}
