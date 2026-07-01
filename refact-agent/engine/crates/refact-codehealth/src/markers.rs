use tree_sitter::Node;

const BRANCH_KINDS: &[&str] = &[
    "if_statement",
    "if_expression",
    "elif_clause",
    "else_if_clause",
    "for_statement",
    "for_expression",
    "for_in_statement",
    "enhanced_for_statement",
    "while_statement",
    "while_expression",
    "do_statement",
    "loop_expression",
    "match_arm",
    "when_entry",
    "case",
    "case_statement",
    "switch_section",
    "catch_clause",
    "rescue",
    "except_clause",
    "conditional_expression",
    "ternary_expression",
];

const NESTING_KINDS: &[&str] = &[
    "if_statement",
    "if_expression",
    "for_statement",
    "for_expression",
    "for_in_statement",
    "enhanced_for_statement",
    "while_statement",
    "while_expression",
    "do_statement",
    "loop_expression",
    "match_expression",
    "match_block",
    "switch_statement",
    "switch_expression",
    "try_statement",
    "catch_clause",
];

fn is_short_circuit(node: Node) -> bool {
    matches!(node.kind(), "&&" | "||" | "and" | "or")
}

pub fn cyclomatic_complexity(node: Node) -> u32 {
    let mut count = 1u32;
    count_branches(node, &mut count);
    count
}

fn count_branches(node: Node, count: &mut u32) {
    if BRANCH_KINDS.contains(&node.kind()) || is_short_circuit(node) {
        *count += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_branches(child, count);
    }
}

pub fn max_nesting(node: Node) -> u32 {
    nesting_depth(node, 0)
}

fn nesting_depth(node: Node, current: u32) -> u32 {
    let here = if NESTING_KINDS.contains(&node.kind()) {
        current + 1
    } else {
        current
    };
    let mut deepest = here;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        deepest = deepest.max(nesting_depth(child, here));
    }
    deepest
}

pub fn lines_of_code(node: Node) -> u32 {
    let start = node.start_position().row;
    let end = node.end_position().row;
    (end.saturating_sub(start) + 1) as u32
}
