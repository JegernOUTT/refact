pub fn remove_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 {
            if let Some(end_idx) = lines.iter().rposition(|l| l.trim() == "```") {
                if end_idx > 0 {
                    let start_idx = 1;
                    if start_idx < end_idx {
                        return lines[start_idx..end_idx].join("\n");
                    }
                }
            }
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_markdown_fences_with_language() {
        let input = "```python\ndef hello():\n    print('world')\n```";
        assert_eq!(
            remove_markdown_fences(input),
            "def hello():\n    print('world')"
        );
    }

    #[test]
    fn test_remove_markdown_fences_without_language() {
        let input = "```\nsome code\n```";
        assert_eq!(remove_markdown_fences(input), "some code");
    }

    #[test]
    fn test_remove_markdown_fences_no_fences() {
        let input = "plain code without fences";
        assert_eq!(remove_markdown_fences(input), "plain code without fences");
    }

    #[test]
    fn test_remove_markdown_fences_with_whitespace() {
        let input = "  ```rust\nfn main() {}\n```  ";
        assert_eq!(remove_markdown_fences(input), "fn main() {}");
    }
}
