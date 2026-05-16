pub fn remove_fencing(message: &String) -> Vec<String> {
    let trimmed_message = message.trim();
    if !trimmed_message.contains("```") {
        return Vec::new();
    }
    if trimmed_message.contains("``````") {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut in_code_block = false;

    for (_i, part) in trimmed_message.split("```").enumerate() {
        if in_code_block {
            let part_lines: Vec<&str> = part.lines().collect();
            if !part_lines.is_empty() {
                let start_idx = if part_lines[0].trim().split_whitespace().count() <= 1
                    && part_lines.len() > 1
                {
                    1
                } else {
                    0
                };
                if start_idx < part_lines.len() {
                    let code_block = part_lines[start_idx..].join("\n");
                    if !code_block.is_empty() {
                        results.push(code_block.trim().to_string());
                    }
                }
            }
        }

        in_code_block = !in_code_block;
    }
    if !results.is_empty() {
        results
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_fencing() {
        let input = "Simple text without fencing".to_string();
        assert_eq!(remove_fencing(&input), Vec::<String>::new());
    }

    #[test]
    fn test_simple_fencing() {
        let input = "```\nCode block\n```".to_string();
        assert_eq!(remove_fencing(&input), vec!["Code block".to_string()]);
    }

    #[test]
    fn test_language_tag() {
        let input = "```rust\nfn main() {\n    println!(\"Hello\");\n}\n```".to_string();
        assert_eq!(
            remove_fencing(&input),
            vec!["fn main() {\n    println!(\"Hello\");\n}".to_string()]
        );
    }

    #[test]
    fn test_text_before_and_after() {
        let input = "Text before\nText before\n```\nCode block\n```\nText after".to_string();
        assert_eq!(remove_fencing(&input), vec!["Code block".to_string()]);
    }

    #[test]
    fn test_multiple_code_blocks() {
        let input = "First paragraph\n```\nFirst code\n```\nMiddle text\n```python\ndef hello():\n    print('world')\n```\nLast paragraph".to_string();
        assert_eq!(
            remove_fencing(&input),
            vec![
                "First code".to_string(),
                "def hello():\n    print('world')".to_string()
            ]
        );
    }

    #[test]
    fn test_empty_code_block() {
        let input = "Text with `````` empty block".to_string();
        assert_eq!(remove_fencing(&input), Vec::<String>::new());
    }
}
