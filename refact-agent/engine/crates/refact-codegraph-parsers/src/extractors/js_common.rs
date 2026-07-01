use tree_sitter::Node;

use crate::ir::{EdgeKind, RawRef, SymbolKind, SymbolNode};

pub fn node_text<'a>(node: Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn line1(node: Node) -> usize {
    node.start_position().row + 1
}

fn line2(node: Node) -> usize {
    node.end_position().row + 1
}

fn push_symbol(
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

fn extract_type_names(clause: Node, bytes: &[u8], out: &mut Vec<String>) {
    let mut c = clause.walk();
    for ch in clause.named_children(&mut c) {
        match ch.kind() {
            "identifier" | "type_identifier" | "member_expression" | "generic_type" => {
                out.push(node_text(ch, bytes).to_string())
            }
            _ => {}
        }
    }
}

fn class_heritage(node: Node, bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" => {
                let mut hc = child.walk();
                for h in child.named_children(&mut hc) {
                    match h.kind() {
                        "extends_clause" => extract_type_names(h, bytes, &mut out),
                        "identifier" | "member_expression" => {
                            out.push(node_text(h, bytes).to_string())
                        }
                        _ => {}
                    }
                }
            }
            "extends_clause" => extract_type_names(child, bytes, &mut out),
            _ => {}
        }
    }
    out
}

pub fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let child_prefix = push_symbol(
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
        "class_declaration" | "class" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let derived = class_heritage(node, bytes);
                let child_prefix = push_symbol(
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
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let child_prefix = push_symbol(
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
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                let derived = class_heritage(node, bytes);
                push_symbol(
                    symbols,
                    prefix,
                    name,
                    SymbolKind::Struct,
                    true,
                    derived,
                    node,
                );
            }
            return;
        }
        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
                push_symbol(
                    symbols,
                    prefix,
                    name,
                    SymbolKind::TypeAlias,
                    false,
                    vec![],
                    node,
                );
            }
            return;
        }
        "variable_declarator" => {
            if let (Some(name_node), Some(value)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) {
                if matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "function"
                ) {
                    let name = node_text(name_node, bytes).to_string();
                    let child_prefix = push_symbol(
                        symbols,
                        prefix,
                        name,
                        SymbolKind::Function,
                        false,
                        vec![],
                        node,
                    );
                    if let Some(body) = value.child_by_field_name("body") {
                        walk(body, bytes, &child_prefix, symbols, refs);
                    }
                    return;
                }
            }
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
        "member_expression" => func
            .child_by_field_name("property")
            .map(|n| node_text(n, bytes).to_string())
            .unwrap_or_default(),
        _ => node_text(func, bytes).to_string(),
    }
}
