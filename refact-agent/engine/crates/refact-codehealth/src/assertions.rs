use tree_sitter::Node;

const FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "method_declaration",
    "method_definition",
    "method",
    "generator_function_declaration",
    "constructor_declaration",
    "singleton_method",
    "local_function_statement",
];

pub fn assertion_blocks(lang: &str, text: &str) -> Vec<crate::test_smells::AssertionBlock> {
    let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) else {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    collect_functions(tree.root_node(), bytes, &mut out);
    out.sort_by(|a, b| (a.start_line, &a.function).cmp(&(b.start_line, &b.function)));
    out
}

fn collect_functions(
    node: Node<'_>,
    bytes: &[u8],
    out: &mut Vec<crate::test_smells::AssertionBlock>,
) {
    if FUNCTION_KINDS.contains(&node.kind()) {
        collect_assertion_blocks(node, bytes, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, bytes, out);
    }
}

fn collect_assertion_blocks(
    function: Node<'_>,
    bytes: &[u8],
    out: &mut Vec<crate::test_smells::AssertionBlock>,
) {
    let Some(body) = function_body(function) else {
        return;
    };
    let name = function_name(function, bytes);
    collect_assertion_blocks_in_block(body, &name, bytes, out);
}

fn collect_assertion_blocks_in_block(
    block: Node<'_>,
    name: &str,
    bytes: &[u8],
    out: &mut Vec<crate::test_smells::AssertionBlock>,
) {
    let mut run_start = 0;
    let mut run_end = 0;
    let mut run_count = 0;
    let mut cursor = block.walk();
    for child in block.named_children(&mut cursor) {
        if is_comment(child) {
            continue;
        }
        if is_assertion(child, bytes) {
            if run_count == 0 {
                run_start = child.start_position().row + 1;
            }
            run_end = child.end_position().row + 1;
            run_count += 1;
        } else {
            push_run(name, run_start, run_end, run_count, out);
            run_start = 0;
            run_end = 0;
            run_count = 0;
        }
    }
    push_run(name, run_start, run_end, run_count, out);

    let mut cursor = block.walk();
    for child in block.named_children(&mut cursor) {
        collect_nested_assertion_blocks(child, name, bytes, out);
    }
}

fn collect_nested_assertion_blocks(
    node: Node<'_>,
    name: &str,
    bytes: &[u8],
    out: &mut Vec<crate::test_smells::AssertionBlock>,
) {
    if FUNCTION_KINDS.contains(&node.kind()) {
        return;
    }
    if is_block(node) {
        collect_assertion_blocks_in_block(node, name, bytes, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_nested_assertion_blocks(child, name, bytes, out);
    }
}

fn push_run(
    function: &str,
    start_line: usize,
    end_line: usize,
    count: usize,
    out: &mut Vec<crate::test_smells::AssertionBlock>,
) {
    if count >= 2 {
        out.push(crate::test_smells::AssertionBlock {
            function: function.to_string(),
            start_line,
            end_line,
            count,
        });
    }
}

fn function_body<'a>(node: Node<'a>) -> Option<Node<'a>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|child| is_block(*child));
    found
}

fn is_block(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "block" | "body" | "statement_block" | "compound_statement" | "declaration_list" | "suite"
    )
}

fn function_name(node: Node<'_>, bytes: &[u8]) -> String {
    if let Some(name) = node.child_by_field_name("name") {
        return text_of(name, bytes).to_string();
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "simple_identifier" | "field_identifier"
        ) {
            return text_of(child, bytes).to_string();
        }
    }
    String::new()
}

fn is_comment(node: Node<'_>) -> bool {
    node.kind().contains("comment")
}

fn is_assertion(node: Node<'_>, bytes: &[u8]) -> bool {
    if matches!(
        node.kind(),
        "assert_statement" | "assertion" | "static_assert_declaration"
    ) {
        return true;
    }
    let trimmed = trim_statement(text_of(node, bytes).trim());
    if trimmed.is_empty() {
        return false;
    }
    if is_known_assertion_start(trimmed) || is_expect_assertion(trimmed) {
        return true;
    }
    if is_conversion_call(trimmed) {
        return false;
    }
    has_method_segment(trimmed, "should") || has_matcher_chain(trimmed)
}

fn trim_statement(text: &str) -> &str {
    text.trim_end_matches(';').trim()
}

fn is_known_assertion_start(text: &str) -> bool {
    let methods = [
        "assertEquals",
        "assertNotEquals",
        "assertTrue",
        "assertFalse",
        "assertNull",
        "assertNotNull",
        "assertSame",
        "assertNotSame",
        "assertThrows",
        "assertThat",
        "assertArrayEquals",
        "assertIterableEquals",
        "assertLinesMatch",
        "assertAll",
        "assertDoesNotThrow",
    ];
    methods
        .iter()
        .any(|method| text.starts_with(method) || has_method_segment(text, method))
        || [
            "assert!",
            "assert_eq!",
            "assert_ne!",
            "debug_assert!",
            "debug_assert_eq!",
            "debug_assert_ne!",
            "assert(",
            "self.assert",
            "assert.",
            "require.",
            "XCTAssert",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
        || text.starts_with("EXPECT_")
        || text.starts_with("ASSERT_")
}

fn is_expect_assertion(text: &str) -> bool {
    text.starts_with("expect(") || text.starts_with("expect.")
}

fn has_matcher_chain(text: &str) -> bool {
    [
        "toBe",
        "toEqual",
        "toStrictEqual",
        "toThrow",
        "toContain",
        "toMatch",
        "toHave",
        "toHaveBeen",
        "toBeTruthy",
        "toBeFalsy",
        "toBeNull",
        "toBeUndefined",
        "toBeDefined",
        "toBeInstanceOf",
        "toBeGreaterThan",
        "toBeLessThan",
        "toBeCloseTo",
        "isEqualTo",
        "isNotEqualTo",
        "isTrue",
        "isFalse",
        "isNull",
        "isNotNull",
    ]
    .iter()
    .any(|method| has_method_segment(text, method))
}

fn has_method_segment(text: &str, method: &str) -> bool {
    let mut rest = text;
    while let Some(index) = rest.find('.') {
        let after_dot = &rest[index + 1..];
        if after_dot.starts_with(method) {
            let tail = &after_dot[method.len()..];
            if tail.starts_with('(') || tail.starts_with('.') || tail.is_empty() {
                return true;
            }
        }
        rest = after_dot;
    }
    false
}

fn is_conversion_call(text: &str) -> bool {
    [
        "to_string",
        "to_owned",
        "to_vec",
        "to_str",
        "to_lowercase",
        "to_uppercase",
        "toString",
        "toLocaleString",
        "valueOf",
    ]
    .iter()
    .any(|method| has_method_segment(text, method))
}

fn text_of<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_consecutive_asserts_form_one_block() {
        let src = "\
def test_values():\n    assert a\n    assert b\n    assert c\n";
        let blocks = assertion_blocks("python", src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].function, "test_values");
        assert_eq!(blocks[0].start_line, 2);
        assert_eq!(blocks[0].end_line, 4);
        assert_eq!(blocks[0].count, 3);
    }

    #[test]
    fn rust_consecutive_assert_macros_form_one_block() {
        let src = "\
#[test]\nfn checks() {\n    assert_eq!(one(), 1);\n    assert_eq!(two(), 2);\n}\n";
        let blocks = assertion_blocks("rust", src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].function, "checks");
        assert_eq!(blocks[0].start_line, 3);
        assert_eq!(blocks[0].end_line, 4);
        assert_eq!(blocks[0].count, 2);
    }

    #[test]
    fn single_assert_is_not_emitted() {
        let src = "\
def test_one():\n    assert value\n";
        let blocks = assertion_blocks("python", src);
        assert!(blocks.is_empty());
    }

    #[test]
    fn non_assertion_statements_split_runs() {
        let src = "\
def test_split():\n    assert a\n    assert b\n    x = 1\n    assert c\n    assert d\n";
        let blocks = assertion_blocks("python", src);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].start_line, 2);
        assert_eq!(blocks[0].end_line, 3);
        assert_eq!(blocks[0].count, 2);
        assert_eq!(blocks[1].start_line, 5);
        assert_eq!(blocks[1].end_line, 6);
        assert_eq!(blocks[1].count, 2);
    }

    #[test]
    fn python_nested_if_asserts_form_one_block() {
        let src = "\
def test_nested():\n    if ready:\n        assert a\n        assert b\n";
        let blocks = assertion_blocks("python", src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].function, "test_nested");
        assert_eq!(blocks[0].start_line, 3);
        assert_eq!(blocks[0].end_line, 4);
        assert_eq!(blocks[0].count, 2);
    }

    #[test]
    fn python_nested_if_and_for_asserts_form_separate_blocks() {
        let src = "\
def test_nested_runs():\n    if ready:\n        assert a\n        assert b\n    for item in items:\n        assert item\n        assert len(item)\n";
        let blocks = assertion_blocks("python", src);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].function, "test_nested_runs");
        assert_eq!(blocks[0].start_line, 3);
        assert_eq!(blocks[0].end_line, 4);
        assert_eq!(blocks[0].count, 2);
        assert_eq!(blocks[1].function, "test_nested_runs");
        assert_eq!(blocks[1].start_line, 6);
        assert_eq!(blocks[1].end_line, 7);
        assert_eq!(blocks[1].count, 2);
    }

    #[test]
    fn python_comments_and_blank_lines_do_not_split_runs() {
        let src = "\
def test_spaced():\n    assert a\n\n    # still same run\n    assert b\n";
        let blocks = assertion_blocks("python", src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].function, "test_spaced");
        assert_eq!(blocks[0].start_line, 2);
        assert_eq!(blocks[0].end_line, 5);
        assert_eq!(blocks[0].count, 2);
    }

    #[test]
    fn to_conversion_calls_are_not_assertions_and_split_runs() {
        let src = "\
#[test]\nfn conversions_split() {\n    assert_eq!(one(), 1);\n    assert_eq!(two(), 2);\n    x.to_string();\n    y.to_owned();\n    assert_eq!(three(), 3);\n    assert_eq!(four(), 4);\n}\n";
        let blocks = assertion_blocks("rust", src);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].start_line, 3);
        assert_eq!(blocks[0].end_line, 4);
        assert_eq!(blocks[0].count, 2);
        assert_eq!(blocks[1].start_line, 7);
        assert_eq!(blocks[1].end_line, 8);
        assert_eq!(blocks[1].count, 2);
    }

    #[test]
    fn rust_nested_block_asserts_form_one_block() {
        let src = "\
#[test]\nfn nested_checks() {\n    {\n        assert_eq!(one(), 1);\n        assert_eq!(two(), 2);\n    }\n}\n";
        let blocks = assertion_blocks("rust", src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].function, "nested_checks");
        assert_eq!(blocks[0].start_line, 4);
        assert_eq!(blocks[0].end_line, 5);
        assert_eq!(blocks[0].count, 2);
    }

    #[test]
    fn rust_conversion_and_ordinary_to_calls_are_not_assertions() {
        let src = "\
#[test]\nfn no_false_positive_conversions() {\n    foo().to_string();\n    x.to_owned();\n    value.to_vec();\n    object.to_custom();\n}\n";
        let blocks = assertion_blocks("rust", src);
        assert!(blocks.is_empty());
    }
}
