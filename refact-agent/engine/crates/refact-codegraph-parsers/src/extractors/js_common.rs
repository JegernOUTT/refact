use tree_sitter::Node;

use crate::ir::{EdgeKind, RawRef, SymbolKind, SymbolNode};

pub fn node_text<'a>(node: Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        if matches!(bytes[0], b'\'' | b'"' | b'`') && bytes[0] == bytes[text.len() - 1] {
            return text[1..text.len() - 1].to_string();
        }
    }
    text.to_string()
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
        is_override: false,
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
                        "extends_clause" | "implements_clause" => {
                            extract_type_names(h, bytes, &mut out)
                        }
                        "identifier" | "member_expression" => {
                            out.push(node_text(h, bytes).to_string())
                        }
                        _ => {}
                    }
                }
            }
            "extends_clause" | "implements_clause" => extract_type_names(child, bytes, &mut out),
            _ => {}
        }
    }
    out
}

fn source_name(node: Node, bytes: &[u8]) -> Option<String> {
    node.child_by_field_name("source")
        .map(|source| unquote(node_text(source, bytes)))
}

fn child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let child = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == kind);
    child
}

fn specifier_name(node: Node, bytes: &[u8]) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(unquote(node_text(name, bytes)));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "string") {
            return Some(unquote(node_text(child, bytes)));
        }
    }
    None
}

fn push_import_ref(refs: &mut Vec<RawRef>, prefix: &[String], name: String, line: usize) {
    if !name.is_empty() {
        refs.push(RawRef {
            from: prefix.join("::"),
            name,
            kind: EdgeKind::Imports,
            line,
        });
    }
}

fn extract_import_statement(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    refs: &mut Vec<RawRef>,
    include_module_refs: bool,
) {
    if include_module_refs {
        if let Some(source) = source_name(node, bytes) {
            push_import_ref(refs, prefix, source, line1(node));
        }
    }
    let Some(import_clause) = child_by_kind(node, "import_clause") else {
        return;
    };
    let Some(named_imports) = child_by_kind(import_clause, "named_imports") else {
        return;
    };
    let mut cursor = named_imports.walk();
    for child in named_imports.named_children(&mut cursor) {
        if child.kind() == "import_specifier" {
            if let Some(name) = specifier_name(child, bytes) {
                push_import_ref(refs, prefix, name, line1(child));
            }
        }
    }
}

fn extract_export_statement(node: Node, bytes: &[u8], prefix: &[String], refs: &mut Vec<RawRef>) {
    let Some(source) = source_name(node, bytes) else {
        return;
    };
    if let Some(export_clause) = child_by_kind(node, "export_clause") {
        let mut cursor = export_clause.walk();
        for child in export_clause.named_children(&mut cursor) {
            if child.kind() == "export_specifier" {
                if let Some(name) = specifier_name(child, bytes) {
                    push_import_ref(refs, prefix, name, line1(child));
                }
            }
        }
    } else {
        push_import_ref(refs, prefix, source, line1(node));
    }
}

pub fn walk(
    node: Node,
    bytes: &[u8],
    prefix: &[String],
    symbols: &mut Vec<SymbolNode>,
    refs: &mut Vec<RawRef>,
    include_module_refs: bool,
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
                    walk(
                        body,
                        bytes,
                        &child_prefix,
                        symbols,
                        refs,
                        include_module_refs,
                    );
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
                    walk(
                        body,
                        bytes,
                        &child_prefix,
                        symbols,
                        refs,
                        include_module_refs,
                    );
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
                    walk(
                        body,
                        bytes,
                        &child_prefix,
                        symbols,
                        refs,
                        include_module_refs,
                    );
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
                        walk(
                            body,
                            bytes,
                            &child_prefix,
                            symbols,
                            refs,
                            include_module_refs,
                        );
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
        "import_statement" if include_module_refs => {
            extract_import_statement(node, bytes, prefix, refs, include_module_refs)
        }
        "export_statement" if include_module_refs => {
            extract_export_statement(node, bytes, prefix, refs)
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, bytes, prefix, symbols, refs, include_module_refs);
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
