use crate::tools::file_edit::auxiliary::normalize_line_endings;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApplyDiffMode {
    Update,
    Create,
}

pub fn apply_v4a_diff(input: &str, diff: &str, mode: ApplyDiffMode) -> Result<String, String> {
    let input_normalized = normalize_line_endings(input);
    let diff_normalized = normalize_line_endings(diff);
    
    let (lines, trailing_newlines) = split_preserving_trailing_newlines(&input_normalized);
    
    let sections = parse_diff_sections(&diff_normalized, mode)?;
    
    let mut result_lines = lines;
    for section in sections {
        result_lines = apply_section(&result_lines, &section)?;
    }
    
    let mut result = result_lines.join("\n");
    for _ in 0..trailing_newlines {
        result.push('\n');
    }
    
    Ok(result)
}

#[derive(Debug, Clone)]
struct DiffSection {
    anchor: Option<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

fn split_preserving_trailing_newlines(s: &str) -> (Vec<String>, usize) {
    if s.is_empty() {
        return (vec![], 0);
    }
    
    let trailing_newlines = s.chars().rev().take_while(|&c| c == '\n').count();
    let content = if trailing_newlines > 0 {
        &s[..s.len() - trailing_newlines]
    } else {
        s
    };
    
    let lines = if content.is_empty() {
        vec![]
    } else {
        content.split('\n').map(|s| s.to_string()).collect()
    };
    
    (lines, trailing_newlines)
}

fn parse_diff_sections(diff: &str, mode: ApplyDiffMode) -> Result<Vec<DiffSection>, String> {
    let lines: Vec<&str> = diff.lines().collect();
    
    if lines.is_empty() {
        return Err("Empty diff provided".to_string());
    }
    
    let mut sections = Vec::new();
    let mut i = 0;
    
    while i < lines.len() && lines[i].is_empty() {
        i += 1;
    }
    
    let has_anchors = lines.iter().any(|l| l.starts_with("@@"));
    
    if mode == ApplyDiffMode::Create && has_anchors {
        return Err("create_file mode does not support @@ anchors".to_string());
    }
    
    if !has_anchors {
        let section = parse_section_body(&lines, 0, lines.len(), None, mode)?;
        sections.push(section);
    } else {
        while i < lines.len() {
            if lines[i].starts_with("@@") {
                let anchor = parse_anchor(lines[i]);
                let anchor_line = lines[i];
                let section_start = i + 1;
                
                let mut section_end = section_start;
                while section_end < lines.len() && !lines[section_end].starts_with("@@") {
                    section_end += 1;
                }
                
                if section_start == section_end {
                    return Err(format!(
                        "Empty section after anchor: {:?}",
                        anchor_line
                    ));
                }
                
                let section = parse_section_body(&lines, section_start, section_end, anchor, mode)?;
                sections.push(section);
                
                i = section_end;
            } else if lines[i].is_empty() {
                i += 1;
            } else {
                return Err(format!(
                    "Invalid diff: stray line outside @@ section: {:?}",
                    lines[i]
                ));
            }
        }
    }
    
    if sections.is_empty() {
        return Err("No valid diff sections found".to_string());
    }
    
    if mode == ApplyDiffMode::Create && sections.len() > 1 {
        return Err("create_file mode does not support multiple sections".to_string());
    }
    
    Ok(sections)
}

fn parse_anchor(line: &str) -> Option<String> {
    let anchor_text = line.trim_start_matches("@@").trim();
    if anchor_text.is_empty() {
        None
    } else {
        Some(anchor_text.to_string())
    }
}

fn parse_section_body(
    lines: &[&str],
    start: usize,
    end: usize,
    anchor: Option<String>,
    mode: ApplyDiffMode,
) -> Result<DiffSection, String> {
    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();
    
    for i in start..end {
        let line = lines[i];
        
        if line.is_empty() && old_lines.is_empty() && new_lines.is_empty() {
            continue;
        }
        
        match mode {
            ApplyDiffMode::Create => {
                if !line.starts_with('+') {
                    return Err(format!(
                        "create_file requires all lines start with '+'. Found: {:?}",
                        line
                    ));
                }
                new_lines.push(line[1..].to_string());
            }
            ApplyDiffMode::Update => {
                if line.starts_with('+') {
                    new_lines.push(line[1..].to_string());
                } else if line.starts_with('-') {
                    old_lines.push(line[1..].to_string());
                } else if line.starts_with(' ') {
                    let content = line[1..].to_string();
                    old_lines.push(content.clone());
                    new_lines.push(content);
                } else if line.is_empty() {
                    old_lines.push(String::new());
                    new_lines.push(String::new());
                } else {
                    return Err(format!(
                        "Invalid diff line (must start with +, -, or space): {:?}",
                        line
                    ));
                }
            }
        }
    }
    
    if mode == ApplyDiffMode::Update {
        if old_lines.is_empty() && !new_lines.is_empty() {
            return Err("Update mode requires context (old lines). Pure insertion not allowed. Use context lines or create_file.".to_string());
        }
        if old_lines.is_empty() && new_lines.is_empty() {
            return Err("Empty diff section".to_string());
        }
    }
    
    Ok(DiffSection {
        anchor,
        old_lines,
        new_lines,
    })
}

fn apply_section(input_lines: &[String], section: &DiffSection) -> Result<Vec<String>, String> {
    if section.old_lines.is_empty() {
        return Ok(section.new_lines.clone());
    }
    
    let (search_start, search_end) = if let Some(anchor) = &section.anchor {
        find_anchor_region(input_lines, anchor)?
    } else {
        (0, input_lines.len())
    };
    
    let search_region = &input_lines[search_start..search_end];
    let matches = find_sequence_matches(search_region, &section.old_lines);
    
    match matches.len() {
        0 => {
            let anchor_text = section.anchor.as_deref().unwrap_or("none");
            Err(format!(
                "Patch conflict: context not found. Anchor: {}. Expected: {:?}",
                anchor_text,
                section.old_lines.iter().take(3).collect::<Vec<_>>()
            ))
        }
        1 => {
            let match_start = search_start + matches[0];
            let match_end = match_start + section.old_lines.len();
            
            let mut result = Vec::new();
            result.extend_from_slice(&input_lines[..match_start]);
            result.extend_from_slice(&section.new_lines);
            result.extend_from_slice(&input_lines[match_end..]);
            
            Ok(result)
        }
        _ => {
            let anchor_text = section.anchor.as_deref().unwrap_or("none");
            Err(format!(
                "Ambiguous patch: {} matches. Anchor: {}. Add more context or use @@ anchor.",
                matches.len(),
                anchor_text
            ))
        }
    }
}

fn find_anchor_region(lines: &[String], anchor: &str) -> Result<(usize, usize), String> {
    let matches: Vec<usize> = lines.iter()
        .enumerate()
        .filter(|(_, line)| line.contains(anchor))
        .map(|(i, _)| i)
        .collect();
    
    match matches.len() {
        0 => Err(format!("Anchor not found: {}", anchor)),
        1 => Ok((matches[0], lines.len())),
        _ => Err(format!("Anchor ambiguous: {} matches for '{}'", matches.len(), anchor)),
    }
}

fn find_sequence_matches(haystack: &[String], needle: &[String]) -> Vec<usize> {
    let mut matches = Vec::new();
    
    if needle.is_empty() {
        return matches;
    }
    
    for start_idx in 0..=haystack.len().saturating_sub(needle.len()) {
        let slice = &haystack[start_idx..start_idx + needle.len()];
        if slice == needle {
            matches.push(start_idx);
        }
    }
    
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_simple() {
        let input = "line1\nold\nline3";
        let diff = " line1\n-old\n+new\n line3";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "line1\nnew\nline3");
    }

    #[test]
    fn test_create() {
        let diff = "+line1\n+line2";
        let result = apply_v4a_diff("", diff, ApplyDiffMode::Create).unwrap();
        assert_eq!(result, "line1\nline2");
    }

    #[test]
    fn test_ambiguous() {
        let input = "old\nold";
        let diff = "-old\n+new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Ambiguous"));
    }

    #[test]
    fn test_trailing_newline() {
        let input = "line1\n";
        let diff = "-line1\n+line2";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "line2\n");
    }

    #[test]
    fn test_update_insertion_only_error() {
        let input = "a\nb\n";
        let diff = "+x";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires context"));
    }

    #[test]
    fn test_multiple_trailing_blanks() {
        let input = "a\n\n\n";
        let diff = "-a\n+A";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "A\n\n\n");
    }

    #[test]
    fn test_anchor_disambiguation() {
        let input = "fn foo() {\n    x\n}\nfn bar() {\n    x\n}";
        let diff = "@@ fn bar()\n-    x\n+    y";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert!(result.contains("fn bar() {\n    y"));
        assert!(result.contains("fn foo() {\n    x"));
    }

    #[test]
    fn test_deletion_with_context() {
        let input = "a\nb\nc";
        let diff = " a\n-b\n c";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\nc");
    }

    #[test]
    fn test_replacement_at_start() {
        let input = "old\nkeep";
        let diff = "-old\n+new\n keep";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "new\nkeep");
    }

    #[test]
    fn test_replacement_at_end() {
        let input = "keep\nold";
        let diff = " keep\n-old\n+new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "keep\nnew");
    }

    #[test]
    fn test_context_not_found() {
        let input = "a\nb";
        let diff = " a\n-c\n+d";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_multiple_sections() {
        let input = "# section1\na\nb\nc\n# section2\nd\ne";
        let diff = "@@ section1\n a\n-b\n+B\n c\n@@ section2\n d\n-e\n+E";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "# section1\na\nB\nc\n# section2\nd\nE");
    }

    #[test]
    fn test_create_reject_non_plus() {
        let diff = "+a\n b";
        let result = apply_v4a_diff("", diff, ApplyDiffMode::Create);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("start with '+'"));
    }

    #[test]
    fn test_create_reject_anchors() {
        let diff = "@@ something\n+a";
        let result = apply_v4a_diff("", diff, ApplyDiffMode::Create);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not support @@ anchors"));
    }

    #[test]
    fn test_anchor_not_found() {
        let input = "fn foo() {\n    x\n}";
        let diff = "@@ fn bar()\n-x\n+y";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Anchor not found"));
    }

    #[test]
    fn test_anchor_ambiguous() {
        let input = "fn foo() {\n    x\n}\nfn foo() {\n    y\n}";
        let diff = "@@ fn foo()\n-x\n+z";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Anchor ambiguous"));
    }

    #[test]
    fn test_empty_line_handling() {
        let input = "a\n\nb";
        let diff = " a\n \n-b\n+c";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\n\nc");
    }

    #[test]
    fn test_no_trailing_newline() {
        let input = "line1";
        let diff = "-line1\n+line2";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "line2");
        assert!(!result.ends_with('\n'));
    }

    #[test]
    fn test_split_empty_string() {
        let (lines, trailing) = split_preserving_trailing_newlines("");
        assert_eq!(lines, Vec::<String>::new());
        assert_eq!(trailing, 0);
    }

    #[test]
    fn test_split_only_newlines() {
        let (lines, trailing) = split_preserving_trailing_newlines("\n");
        assert_eq!(lines, Vec::<String>::new());
        assert_eq!(trailing, 1);
        
        let (lines, trailing) = split_preserving_trailing_newlines("\n\n\n");
        assert_eq!(lines, Vec::<String>::new());
        assert_eq!(trailing, 3);
    }

    #[test]
    fn test_split_no_trailing() {
        let (lines, trailing) = split_preserving_trailing_newlines("a\nb");
        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(trailing, 0);
    }

    #[test]
    fn test_split_with_trailing() {
        let (lines, trailing) = split_preserving_trailing_newlines("a\nb\n");
        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(trailing, 1);
        
        let (lines, trailing) = split_preserving_trailing_newlines("x\n\n");
        assert_eq!(lines, vec!["x"]);
        assert_eq!(trailing, 2);
    }

    #[test]
    fn test_split_single_line() {
        let (lines, trailing) = split_preserving_trailing_newlines("single");
        assert_eq!(lines, vec!["single"]);
        assert_eq!(trailing, 0);
        
        let (lines, trailing) = split_preserving_trailing_newlines("single\n");
        assert_eq!(lines, vec!["single"]);
        assert_eq!(trailing, 1);
    }

    #[test]
    fn test_whitespace_only_context() {
        let input = "a\n\n\nb";
        let diff = " a\n \n \n-b\n+B";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\n\n\nB");
    }

    #[test]
    fn test_deletion_only_succeeds_unique() {
        let input = "a\nb\nc";
        let diff = "-b";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\nc");
    }

    #[test]
    fn test_sequential_sections() {
        let input = "# first\na\nb\nc\n# second\nd";
        let diff = "@@ first\n-a\n+A\n b\n@@ second\n-d\n+D";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "# first\nA\nb\nc\n# second\nD");
    }

    #[test]
    fn test_anchor_restricts_search_region() {
        let input = "class A:\n    def foo():\n        x = 1\nclass B:\n    def foo():\n        x = 1";
        let diff = "@@ class B\n-        x = 1\n+        x = 2";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert!(result.contains("class A:\n    def foo():\n        x = 1"));
        assert!(result.contains("class B:\n    def foo():\n        x = 2"));
    }

    #[test]
    fn test_create_with_trailing_newline() {
        let diff = "+line1\n+line2\n+";
        let result = apply_v4a_diff("", diff, ApplyDiffMode::Create).unwrap();
        assert_eq!(result, "line1\nline2\n");
    }

    #[test]
    fn test_create_single_empty_line() {
        let diff = "+";
        let result = apply_v4a_diff("", diff, ApplyDiffMode::Create).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_exact_match_succeeds() {
        let input = "function() {\n    code\n}";
        let diff = "-    code\n+    CODE";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "function() {\n    CODE\n}");
    }

    #[test]
    fn test_tabs_vs_spaces() {
        let input = "\tindented";
        let diff = "-\tindented\n+    indented";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "    indented");
    }

    #[test]
    fn test_create_multiple_sections_error() {
        let diff = "@@ sec1\n+a\n@@ sec2\n+b";
        let result = apply_v4a_diff("", diff, ApplyDiffMode::Create);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not support"));
    }

    #[test]
    fn test_delete_all_content() {
        let input = "line1\nline2\nline3";
        let diff = "-line1\n-line2\n-line3";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_anchor_includes_self() {
        let input = "fn foo() {\n    old\n}";
        let diff = "@@ fn foo()\n-fn foo() {\n-    old\n+fn bar() {\n+    new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "fn bar() {\n    new\n}");
    }

    #[test]
    fn test_crlf_input() {
        let input = "line1\r\nold\r\nline3";
        let diff = " line1\n-old\n+new\n line3";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "line1\nnew\nline3");
    }

    #[test]
    fn test_crlf_diff() {
        let input = "line1\nold\nline3";
        let diff = " line1\r\n-old\r\n+new\r\n line3";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "line1\nnew\nline3");
    }

    #[test]
    fn test_unicode_content() {
        let input = "hello 世界\nold 🎉\nend";
        let diff = " hello 世界\n-old 🎉\n+new 🚀\n end";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "hello 世界\nnew 🚀\nend");
    }

    #[test]
    fn test_unicode_anchor() {
        let input = "# 日本語セクション\nold\n# other";
        let diff = "@@ 日本語セクション\n-old\n+new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "# 日本語セクション\nnew\n# other");
    }

    #[test]
    fn test_empty_anchor_marker() {
        let input = "a\nb\nc";
        let diff = "@@\n-b\n+B";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\nB\nc");
    }

    #[test]
    fn test_anchor_with_special_regex_chars() {
        let input = "func foo() { return x * (y + z); }\nold";
        let diff = "@@ (y + z)\n-old\n+new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "func foo() { return x * (y + z); }\nnew");
    }

    #[test]
    fn test_section_removes_next_anchor() {
        let input = "# first\na\n# second\nb";
        let diff = "@@ first\n-a\n-# second\n+replaced\n@@ second\n-b\n+B";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Anchor not found"));
    }

    #[test]
    fn test_duplicate_lines_in_context() {
        let input = "repeat\nrepeat\nrepeat\nunique";
        let diff = " repeat\n repeat\n repeat\n-unique\n+UNIQUE";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "repeat\nrepeat\nrepeat\nUNIQUE");
    }

    #[test]
    fn test_content_starting_with_plus() {
        let input = "normal\n+prefixed\nend";
        let diff = " normal\n-+prefixed\n++modified\n end";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "normal\n+modified\nend");
    }

    #[test]
    fn test_content_starting_with_minus() {
        let input = "normal\n-prefixed\nend";
        let diff = " normal\n--prefixed\n+-modified\n end";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "normal\n-modified\nend");
    }

    #[test]
    fn test_diff_begins_with_blank_context() {
        let input = "\nA\nB";
        let diff = " \n-A\n+X\n B";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "\nX\nB");
    }

    #[test]
    fn test_anchored_section_begins_with_blank_context() {
        let input = "fn f() {\n\n    A\n}";
        let diff = "@@ fn f()\n \n-    A\n+    X\n }";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "fn f() {\n\n    X\n}");
    }

    #[test]
    fn test_delete_from_newline_terminated_file() {
        let input = "a\n";
        let diff = "-a";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "\n");
    }

    #[test]
    fn test_add_empty_line() {
        let input = "a\nb";
        let diff = " a\n+\n b";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\n\nb");
    }

    #[test]
    fn test_delete_empty_line() {
        let input = "a\n\nb";
        let diff = " a\n-\n b";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "a\nb");
    }

    #[test]
    fn test_anchor_substring_unique() {
        let input = "fn foobar() {}\n// foo comment\nfn foo() {\n    old\n}";
        let diff = "@@ fn foo()\n-    old\n+    new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert!(result.contains("fn foo() {\n    new\n}"));
        assert!(result.contains("fn foobar() {}"));
    }

    #[test]
    fn test_reject_stray_preamble_line_before_first_anchor() {
        let input = "a\nold\nc";
        let diff = "oops\n@@ a\n-old\n+new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("stray line"));
    }

    #[test]
    fn test_reject_invalid_line_in_section() {
        let input = "# a\nold1\n# b\nold2";
        let diff = "@@ a\n-old1\n+new1\noops\n@@ b\n-old2\n+new2";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid diff line"));
    }

    #[test]
    fn test_reject_invalid_trailing_line_in_section() {
        let input = "# a\nold";
        let diff = "@@ a\n-old\n+new\noops";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid diff line"));
    }

    #[test]
    fn test_empty_section_after_anchor_errors() {
        let input = "fn foo() {}\nfn bar() {\n    x\n}";
        let diff = "@@ fn foo()\n@@ fn bar()\n-    x\n+    y";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Empty section"));
        assert!(err.contains("fn foo()"));
    }

    #[test]
    fn test_update_cannot_add_final_newline() {
        let input = "a";
        let diff = "-a\n+b";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "b");
        assert!(!result.ends_with('\n'));
    }

    #[test]
    fn test_update_preserves_multiple_trailing_newlines() {
        let input = "a\n\n";
        let diff = "-a\n+b";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update).unwrap();
        assert_eq!(result, "b\n\n");
    }

    #[test]
    fn test_anchored_insertion_only_rejected() {
        let input = "fn foo() {\n    code\n}";
        let diff = "@@ fn foo()\n+    inserted";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires context"));
    }

    #[test]
    fn test_anchor_ambiguous_comment_and_code() {
        let input = "// fn foo() comment\nfn foo() {\n    old\n}";
        let diff = "@@ fn foo()\n-    old\n+    new";
        let result = apply_v4a_diff(input, diff, ApplyDiffMode::Update);
        assert!(result.is_err());
        let err = result.unwrap_err().to_lowercase();
        assert!(err.contains("ambiguous"));
    }
}
