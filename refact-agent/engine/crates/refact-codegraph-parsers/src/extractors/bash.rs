use tree_sitter::{Node, Parser, Tree};

use crate::ir::{EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};

pub struct BashExtractor;

impl BashExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for BashExtractor {
    fn default() -> Self {
        BashExtractor
    }
}

impl LangExtractor for BashExtractor {
    fn language(&self) -> &'static str {
        "bash"
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

fn command_name(node: Node, bytes: &[u8]) -> Option<String> {
    if let Some(name) = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("command_name"))
    {
        return Some(node_text(name, bytes).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "command_name" | "word" | "identifier") {
            return Some(node_text(child, bytes).to_string());
        }
    }
    None
}

fn is_identifier_command(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !is_builtin_command(name)
}

fn is_builtin_command(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | "cd"
            | "exit"
            | "return"
            | "local"
            | "export"
            | "set"
            | "shift"
            | "test"
            | "["
            | "read"
            | "printf"
    )
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
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, bytes).to_string();
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
                    is_override: false,
                });
                if let Some(body) = node.child_by_field_name("body") {
                    walk(body, bytes, &path, symbols, refs);
                }
                return;
            }
        }
        "command" => {
            if let Some(name) = command_name(node, bytes) {
                if is_identifier_command(&name) {
                    let from = if prefix.is_empty() {
                        RawRef::FILE_SCOPE.to_string()
                    } else {
                        prefix.join("::")
                    };
                    refs.push(RawRef {
                        from,
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
        let tree = BashExtractor::parse(source).expect("parse bash");
        BashExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_functions_and_command_calls() {
        let src = "greet() { echo hi; }\ngreet";
        let (symbols, refs) = extract(src);
        assert!(symbols.iter().any(|s| s.name() == "greet"));
        assert!(refs.iter().any(|r| r.name == "greet"), "got {refs:?}");
    }

    #[test]
    fn filters_command_calls_to_identifier_non_builtins() {
        let src = "\
foo() { :; }
foo arg
echo hi
./script.sh
/usr/bin/env bash
$RUNNER
";
        let (_symbols, refs) = extract(src);
        assert!(
            refs.iter()
                .any(|r| r.name == "foo" && r.from == RawRef::FILE_SCOPE)
        );
        assert!(!refs.iter().any(|r| r.name == "echo"), "got {refs:?}");
        assert!(
            !refs.iter().any(|r| r.name == "./script.sh"),
            "got {refs:?}"
        );
        assert!(
            !refs.iter().any(|r| r.name == "/usr/bin/env"),
            "got {refs:?}"
        );
        assert!(!refs.iter().any(|r| r.name == "$RUNNER"), "got {refs:?}");
    }

    #[test]
    fn bash_top_level_call_has_real_caller() {
        let src = "foo() { :; }\nfoo\n";
        let (_symbols, refs) = extract(src);
        let call = refs.iter().find(|r| r.name == "foo").expect("foo call");

        assert_eq!(call.from, RawRef::FILE_SCOPE);
        assert_eq!(call.kind, EdgeKind::Calls);
    }
}
