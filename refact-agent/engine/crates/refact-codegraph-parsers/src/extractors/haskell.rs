use std::collections::HashSet;

use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct HaskellExtractor;

impl HaskellExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_haskell::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for HaskellExtractor {
    fn default() -> Self {
        HaskellExtractor
    }
}

impl LangExtractor for HaskellExtractor {
    fn language(&self) -> &'static str {
        "haskell"
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
        "function" | "bind" | "signature" => {
            if let Some(name_node) = binding_name(node, bytes) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() && is_variable_name(&name) {
                    let (symbol, child_prefix) =
                        make_symbol(prefix, name, SymbolKind::Function, false, node);
                    push_symbol(symbols, seen, symbol);
                    walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                    return;
                }
            }
        }
        "data_type" | "newtype" | "type_synonym" => {
            if let Some(name_node) = type_name(node, bytes) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() {
                    let kind = if node.kind() == "type_synonym" {
                        SymbolKind::TypeAlias
                    } else {
                        SymbolKind::Struct
                    };
                    let (symbol, _) = make_symbol(prefix, name, kind, false, node);
                    push_symbol(symbols, seen, symbol);
                }
            }
        }
        "class" | "class_declaration" | "class_decl" => {
            if let Some(name_node) = type_name(node, bytes) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() {
                    let (symbol, child_prefix) =
                        make_symbol(prefix, name, SymbolKind::Struct, true, node);
                    push_symbol(symbols, seen, symbol);
                    walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                    return;
                }
            }
        }
        "instance" | "instance_declaration" | "instance_decl" => {
            if let Some(name_node) = instance_name(node, bytes) {
                let name = clean_name(node_text(name_node, bytes));
                if !name.is_empty() {
                    let (symbol, child_prefix) =
                        make_symbol(prefix, name, SymbolKind::Struct, true, node);
                    push_symbol(symbols, seen, symbol);
                    walk_children(node, bytes, &child_prefix, symbols, refs, seen);
                    return;
                }
            }
        }
        "apply" | "exp_apply" => {
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
    for field in ["name", "lhs", "pattern", "variable"] {
        if let Some(child) = node.child_by_field_name(field) {
            if is_variable_node(child.kind()) && is_variable_name(node_text(child, bytes)) {
                return Some(child);
            }
            if let Some(found) = find_variable(child, bytes) {
                return Some(found);
            }
        }
    }
    find_variable(node, bytes)
}

fn type_name<'a>(node: Node<'a>, bytes: &[u8]) -> Option<Node<'a>> {
    for field in ["name", "type", "head"] {
        if let Some(child) = node.child_by_field_name(field) {
            if is_type_node(child.kind()) && is_type_name(node_text(child, bytes)) {
                return Some(child);
            }
            if let Some(found) = find_type(child, bytes) {
                return Some(found);
            }
        }
    }
    find_type(node, bytes)
}

fn instance_name<'a>(node: Node<'a>, bytes: &[u8]) -> Option<Node<'a>> {
    for field in ["class", "name", "type"] {
        if let Some(child) = node.child_by_field_name(field) {
            if is_type_node(child.kind()) && is_type_name(node_text(child, bytes)) {
                return Some(child);
            }
            if let Some(found) = find_type(child, bytes) {
                return Some(found);
            }
        }
    }
    find_type(node, bytes)
}

fn find_variable<'a>(node: Node<'a>, bytes: &[u8]) -> Option<Node<'a>> {
    if is_variable_node(node.kind()) && is_variable_name(node_text(node, bytes)) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_variable(child, bytes) {
            return Some(found);
        }
    }
    None
}

fn find_type<'a>(node: Node<'a>, bytes: &[u8]) -> Option<Node<'a>> {
    if is_type_node(node.kind()) && is_type_name(node_text(node, bytes)) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_type(child, bytes) {
            return Some(found);
        }
    }
    None
}

fn first_named_child(node: Node) -> Option<Node> {
    node.named_child(0)
}

fn is_variable_node(kind: &str) -> bool {
    matches!(
        kind,
        "variable"
            | "identifier"
            | "variable_identifier"
            | "varid"
            | "prefix_id"
            | "qvarid"
            | "operator"
    )
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "type"
            | "type_constructor"
            | "constructor"
            | "constructor_identifier"
            | "type_identifier"
            | "conid"
            | "qconid"
            | "name"
    )
}

fn callee_name(func: Node, bytes: &[u8]) -> String {
    if let Some(name_node) = find_variable(func, bytes) {
        let name = clean_name(node_text(name_node, bytes));
        if is_variable_name(&name) {
            return name
                .rsplit('.')
                .find(|part| !part.is_empty())
                .unwrap_or(&name)
                .to_string();
        }
    }
    let text = clean_name(node_text(func, bytes));
    let name = text
        .rsplit('.')
        .find(|part| !part.is_empty())
        .unwrap_or(&text)
        .to_string();
    if is_variable_name(&name) {
        name
    } else {
        String::new()
    }
}

fn clean_name(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .to_string()
}

fn is_variable_name(text: &str) -> bool {
    text.chars()
        .next()
        .map(|c| c == '_' || c.is_ascii_lowercase())
        .unwrap_or(false)
}

fn is_type_name(text: &str) -> bool {
    text.chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = HaskellExtractor::parse(source).expect("parse haskell");
        HaskellExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_bindings_types_and_calls() {
        let src = "add :: Int -> Int -> Int\nadd x y = x + y\nmain = print (add 1 2)\ndata Color = Red | Blue\n";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"add".to_string()));
        assert!(paths.contains(&"main".to_string()));
        assert!(paths.contains(&"Color".to_string()));
        assert_eq!(paths.iter().filter(|p| *p == "add").count(), 1);
        assert!(symbols
            .iter()
            .any(|s| s.name() == "Color" && s.kind == SymbolKind::Struct));
        assert!(refs
            .iter()
            .any(|r| r.name == "add" && r.kind == EdgeKind::Calls));
    }
}
