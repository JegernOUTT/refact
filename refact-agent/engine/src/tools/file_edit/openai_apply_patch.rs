use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ParsedPatch {
    pub operations: Vec<FileOperation>,
}

#[derive(Debug, Clone)]
pub enum FileOperation {
    Add {
        path: String,
        contents: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<UpdateChunk>,
    },
}

#[derive(Debug, Clone)]
pub struct UpdateChunk {
    pub change_context: Vec<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub is_end_of_file: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    InvalidPatch(String),
    InvalidHunk { message: String, line_number: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::InvalidPatch(msg) => write!(f, "Invalid patch: {}", msg),
            ParseError::InvalidHunk {
                message,
                line_number,
            } => {
                write!(f, "Invalid hunk at line {}: {}", line_number, message)
            }
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse_patch(patch: &str) -> Result<ParsedPatch, ParseError> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut operations = Vec::new();
    let mut i = 0;
    let mut found_end_patch = false;

    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }

    if i >= lines.len() || !lines[i].trim().starts_with("*** Begin Patch") {
        return Err(ParseError::InvalidPatch(
            "Patch must start with '*** Begin Patch'".to_string(),
        ));
    }
    i += 1;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("*** End Patch") {
            found_end_patch = true;
            break;
        }

        if line.is_empty() {
            i += 1;
            continue;
        }

        if line.starts_with("*** Add File:") {
            let (op, next_i) = parse_add_file(&lines, i)?;
            operations.push(op);
            i = next_i;
        } else if line.starts_with("*** Delete File:") {
            let (op, next_i) = parse_delete_file(&lines, i)?;
            operations.push(op);
            i = next_i;
        } else if line.starts_with("*** Update File:") {
            let (op, next_i) = parse_update_file(&lines, i)?;
            operations.push(op);
            i = next_i;
        } else {
            return Err(ParseError::InvalidHunk {
                message: format!("Unexpected line: '{}'", line),
                line_number: i + 1,
            });
        }
    }

    if !found_end_patch {
        return Err(ParseError::InvalidPatch(
            "Patch must end with '*** End Patch'".to_string(),
        ));
    }

    if operations.is_empty() {
        return Err(ParseError::InvalidPatch(
            "No file operations found".to_string(),
        ));
    }

    Ok(ParsedPatch { operations })
}

fn parse_add_file(lines: &[&str], start: usize) -> Result<(FileOperation, usize), ParseError> {
    let header = lines[start].trim();
    let path = header
        .strip_prefix("*** Add File:")
        .ok_or_else(|| ParseError::InvalidPatch("Invalid Add File header".to_string()))?
        .trim()
        .to_string();

    if path.is_empty() {
        return Err(ParseError::InvalidHunk {
            message: "Add File path cannot be empty".to_string(),
            line_number: start + 1,
        });
    }

    let mut i = start + 1;
    let mut content_lines = Vec::new();

    while i < lines.len() {
        let line = lines[i];

        if line.trim().starts_with("*** ") {
            break;
        }

        if line.starts_with('+') {
            content_lines.push(&line[1..]);
        } else {
            return Err(ParseError::InvalidHunk {
                message: format!("Add File lines must start with '+', found: '{}'", line),
                line_number: i + 1,
            });
        }
        i += 1;
    }

    let contents = if content_lines.is_empty() {
        String::new()
    } else {
        let mut result = content_lines.join("\n");
        result.push('\n');
        result
    };

    Ok((FileOperation::Add { path, contents }, i))
}

fn parse_delete_file(lines: &[&str], start: usize) -> Result<(FileOperation, usize), ParseError> {
    let header = lines[start].trim();
    let path = header
        .strip_prefix("*** Delete File:")
        .ok_or_else(|| ParseError::InvalidPatch("Invalid Delete File header".to_string()))?
        .trim()
        .to_string();

    if path.is_empty() {
        return Err(ParseError::InvalidHunk {
            message: "Delete File path cannot be empty".to_string(),
            line_number: start + 1,
        });
    }

    Ok((FileOperation::Delete { path }, start + 1))
}

fn parse_update_file(lines: &[&str], start: usize) -> Result<(FileOperation, usize), ParseError> {
    let header = lines[start].trim();
    let path = header
        .strip_prefix("*** Update File:")
        .ok_or_else(|| ParseError::InvalidPatch("Invalid Update File header".to_string()))?
        .trim()
        .to_string();

    if path.is_empty() {
        return Err(ParseError::InvalidHunk {
            message: "Update File path cannot be empty".to_string(),
            line_number: start + 1,
        });
    }

    let mut i = start + 1;
    let mut move_to = None;
    let mut chunks = Vec::new();

    if i < lines.len() && lines[i].trim().starts_with("*** Move to:") {
        let move_path = lines[i]
            .trim()
            .strip_prefix("*** Move to:")
            .unwrap()
            .trim()
            .to_string();
        if move_path.is_empty() {
            return Err(ParseError::InvalidHunk {
                message: "Move to path cannot be empty".to_string(),
                line_number: i + 1,
            });
        }
        move_to = Some(move_path);
        i += 1;
    }

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("*** Add File:")
            || line.starts_with("*** Delete File:")
            || line.starts_with("*** Update File:")
            || line.starts_with("*** End Patch")
        {
            break;
        }

        if line.starts_with("@@")
            || line.starts_with('+')
            || line.starts_with('-')
            || line.starts_with(' ')
        {
            let (chunk, next_i) = parse_hunk(lines, i)?;
            chunks.push(chunk);
            i = next_i;
        } else if line.is_empty() {
            i += 1;
        } else {
            return Err(ParseError::InvalidHunk {
                message: format!("Unexpected line in Update File: '{}'", line),
                line_number: i + 1,
            });
        }
    }

    if chunks.is_empty() {
        return Err(ParseError::InvalidHunk {
            message: "Update File requires at least one hunk".to_string(),
            line_number: start + 1,
        });
    }

    Ok((
        FileOperation::Update {
            path,
            move_to,
            chunks,
        },
        i,
    ))
}

pub fn validate_relative_path(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();

    if path.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    if path.starts_with('/') {
        return Err(format!("Absolute paths not allowed: '{}'", path));
    }

    if path.len() >= 2 {
        let bytes = path.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err(format!("Absolute paths not allowed: '{}'", path));
        }
    }
    if path.starts_with("\\\\") {
        return Err(format!("UNC paths not allowed: '{}'", path));
    }

    if path.contains('\\') {
        return Err(format!("Backslashes not allowed in paths: '{}'", path));
    }

    let path_buf = PathBuf::from(path);

    let mut depth: i32 = 0;
    for component in path_buf.components() {
        match component {
            std::path::Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("Path escapes workspace: '{}'", path));
                }
            }
            std::path::Component::Normal(_) => {
                depth += 1;
            }
            std::path::Component::CurDir => {}
            _ => {
                return Err(format!("Invalid path component in: '{}'", path));
            }
        }
    }

    Ok(path_buf)
}

pub fn apply_update_chunks(original: &str, chunks: &[UpdateChunk]) -> Result<String, String> {
    let mut lines: Vec<String> = original.lines().map(String::from).collect();
    let had_trailing_newline = original.ends_with('\n');

    let replacements = compute_replacements(&lines, chunks)?;

    for (start_idx, old_len, new_segment) in replacements.into_iter().rev() {
        let end_idx = (start_idx + old_len).min(lines.len());
        lines.splice(start_idx..end_idx, new_segment);
    }

    let mut result = lines.join("\n");
    if !result.is_empty() || had_trailing_newline {
        result.push('\n');
    }

    Ok(result)
}

fn compute_replacements(
    original_lines: &[String],
    chunks: &[UpdateChunk],
) -> Result<Vec<(usize, usize, Vec<String>)>, String> {
    let mut replacements: Vec<(usize, usize, Vec<String>, usize)> = Vec::new();
    let mut line_index: usize = 0;

    for (seq, chunk) in chunks.iter().enumerate() {
        for ctx in &chunk.change_context {
            if let Some(idx) = seek_sequence(original_lines, &[ctx.clone()], line_index, false) {
                line_index = idx + 1;
            } else {
                return Err(format!("Failed to find context '{}' in file", ctx));
            }
        }

        if chunk.old_lines.is_empty() {
            let insertion_idx = if chunk.is_end_of_file {
                if original_lines.last().is_some_and(String::is_empty) {
                    original_lines.len() - 1
                } else {
                    original_lines.len()
                }
            } else {
                line_index
            };
            if !chunk.new_lines.is_empty() {
                replacements.push((insertion_idx, 0, chunk.new_lines.clone(), seq));
                line_index = insertion_idx + chunk.new_lines.len();
            }
            continue;
        }

        let pattern = &chunk.old_lines;
        let found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);

        let (found, pattern_len, new_slice) = match found {
            Some(idx) => (Some(idx), pattern.len(), chunk.new_lines.as_slice()),
            None if pattern.last().is_some_and(String::is_empty) => {
                let trimmed_pattern = &pattern[..pattern.len() - 1];
                let trimmed_new = if chunk.new_lines.last().is_some_and(String::is_empty) {
                    &chunk.new_lines[..chunk.new_lines.len() - 1]
                } else {
                    &chunk.new_lines[..]
                };
                let retry = seek_sequence(
                    original_lines,
                    trimmed_pattern,
                    line_index,
                    chunk.is_end_of_file,
                );
                (retry, trimmed_pattern.len(), trimmed_new)
            }
            None => (None, pattern.len(), chunk.new_lines.as_slice()),
        };

        if let Some(start_idx) = found {
            replacements.push((start_idx, pattern_len, new_slice.to_vec(), seq));
            line_index = start_idx + pattern_len;
        } else {
            return Err(format!(
                "Failed to find expected lines in file:\n{}",
                chunk
                    .old_lines
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }

    replacements.sort_by(|a, b| a.0.cmp(&b.0).then(a.3.cmp(&b.3)));

    Ok(replacements
        .into_iter()
        .map(|(idx, len, lines, _)| (idx, len, lines))
        .collect())
}

fn seek_sequence(
    haystack: &[String],
    needle: &[String],
    start_from: usize,
    is_end_of_file: bool,
) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }

    let max_idx = haystack.len().saturating_sub(needle.len());

    if is_end_of_file {
        for i in (start_from..=max_idx).rev() {
            if matches_sequence(&haystack[i..], needle) {
                return Some(i);
            }
        }
        for i in (start_from..=max_idx).rev() {
            if matches_sequence_normalized(&haystack[i..], needle) {
                return Some(i);
            }
        }
    } else {
        for i in start_from..=max_idx {
            if matches_sequence(&haystack[i..], needle) {
                return Some(i);
            }
        }
        for i in start_from..=max_idx {
            if matches_sequence_normalized(&haystack[i..], needle) {
                return Some(i);
            }
        }
    }

    None
}

fn matches_sequence(haystack: &[String], needle: &[String]) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    for (h, n) in haystack.iter().zip(needle.iter()) {
        if h != n && h.trim_end() != n.trim_end() {
            return false;
        }
    }
    true
}

fn matches_sequence_normalized(haystack: &[String], needle: &[String]) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    for (h, n) in haystack.iter().zip(needle.iter()) {
        if !lines_match_normalized(h, n) {
            return false;
        }
    }
    true
}

fn lines_match_normalized(a: &str, b: &str) -> bool {
    let norm_a = normalize_line(a);
    let norm_b = normalize_line(b);
    norm_a == norm_b
}

fn normalize_line(s: &str) -> String {
    s.trim_end()
        .chars()
        .map(|c| match c {
            '\u{2013}' | '\u{2014}' | '\u{2212}' | '\u{2011}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            _ => c,
        })
        .collect()
}

fn parse_hunk(lines: &[&str], start: usize) -> Result<(UpdateChunk, usize), ParseError> {
    let mut i = start;
    let mut change_context = Vec::new();
    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();
    let mut is_end_of_file = false;

    while i < lines.len() && lines[i].trim().starts_with("@@") {
        let ctx = lines[i].trim().strip_prefix("@@").unwrap().trim();
        if !ctx.is_empty() {
            change_context.push(ctx.to_string());
        }
        i += 1;
    }

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.starts_with("*** End of File") {
            is_end_of_file = true;
            i += 1;
            break;
        }

        if trimmed.starts_with("*** ") || trimmed.starts_with("@@") {
            break;
        }

        if line.starts_with('+') {
            new_lines.push(line[1..].to_string());
        } else if line.starts_with('-') {
            old_lines.push(line[1..].to_string());
        } else if line.starts_with(' ') || line.is_empty() {
            let content = if line.is_empty() {
                String::new()
            } else {
                line[1..].to_string()
            };
            old_lines.push(content.clone());
            new_lines.push(content);
        } else {
            return Err(ParseError::InvalidHunk {
                message: format!(
                    "Invalid diff line (must start with +, -, or space): '{}'",
                    line
                ),
                line_number: i + 1,
            });
        }
        i += 1;
    }

    Ok((
        UpdateChunk {
            change_context,
            old_lines,
            new_lines,
            is_end_of_file,
        },
        i,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap_patch(body: &str) -> String {
        format!("*** Begin Patch\n{}\n*** End Patch", body)
    }

    #[test]
    fn test_parse_add_file() {
        let patch = wrap_patch("*** Add File: hello.txt\n+Hello world\n+Line 2");
        let parsed = parse_patch(&patch).unwrap();
        assert_eq!(parsed.operations.len(), 1);
        match &parsed.operations[0] {
            FileOperation::Add { path, contents } => {
                assert_eq!(path, "hello.txt");
                assert_eq!(contents, "Hello world\nLine 2\n");
            }
            _ => panic!("Expected Add operation"),
        }
    }

    #[test]
    fn test_parse_delete_file() {
        let patch = wrap_patch("*** Delete File: obsolete.txt");
        let parsed = parse_patch(&patch).unwrap();
        assert_eq!(parsed.operations.len(), 1);
        match &parsed.operations[0] {
            FileOperation::Delete { path } => {
                assert_eq!(path, "obsolete.txt");
            }
            _ => panic!("Expected Delete operation"),
        }
    }

    #[test]
    fn test_parse_update_file() {
        let patch = wrap_patch(
            "*** Update File: src/app.py\n@@ def greet():\n-print(\"Hi\")\n+print(\"Hello, world!\")"
        );
        let parsed = parse_patch(&patch).unwrap();
        assert_eq!(parsed.operations.len(), 1);
        match &parsed.operations[0] {
            FileOperation::Update {
                path,
                move_to,
                chunks,
            } => {
                assert_eq!(path, "src/app.py");
                assert!(move_to.is_none());
                assert_eq!(chunks.len(), 1);
                assert_eq!(chunks[0].change_context, vec!["def greet():"]);
                assert_eq!(chunks[0].old_lines, vec!["print(\"Hi\")"]);
                assert_eq!(chunks[0].new_lines, vec!["print(\"Hello, world!\")"]);
            }
            _ => panic!("Expected Update operation"),
        }
    }

    #[test]
    fn test_parse_update_with_move() {
        let patch = wrap_patch("*** Update File: old.py\n*** Move to: new.py\n@@ \n-old\n+new");
        let parsed = parse_patch(&patch).unwrap();
        match &parsed.operations[0] {
            FileOperation::Update { path, move_to, .. } => {
                assert_eq!(path, "old.py");
                assert_eq!(move_to.as_deref(), Some("new.py"));
            }
            _ => panic!("Expected Update operation"),
        }
    }

    #[test]
    fn test_parse_multi_file() {
        let patch = wrap_patch(
            "*** Add File: new.txt\n+content\n*** Update File: existing.txt\n@@ \n-old\n+new\n*** Delete File: old.txt"
        );
        let parsed = parse_patch(&patch).unwrap();
        assert_eq!(parsed.operations.len(), 3);
        assert!(matches!(&parsed.operations[0], FileOperation::Add { .. }));
        assert!(matches!(
            &parsed.operations[1],
            FileOperation::Update { .. }
        ));
        assert!(matches!(
            &parsed.operations[2],
            FileOperation::Delete { .. }
        ));
    }

    #[test]
    fn test_parse_eof_marker() {
        let patch = wrap_patch("*** Update File: file.txt\n@@\n+appended line\n*** End of File");
        let parsed = parse_patch(&patch).unwrap();
        match &parsed.operations[0] {
            FileOperation::Update { chunks, .. } => {
                assert!(chunks[0].is_end_of_file);
            }
            _ => panic!("Expected Update operation"),
        }
    }

    #[test]
    fn test_validate_relative_path() {
        assert!(validate_relative_path("src/file.rs").is_ok());
        assert!(validate_relative_path("file.txt").is_ok());
        assert!(validate_relative_path("a/b/c/d.txt").is_ok());

        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("C:\\Windows\\file.txt").is_err());
        assert!(validate_relative_path("\\\\server\\share").is_err());

        assert!(validate_relative_path("../escape.txt").is_err());
        assert!(validate_relative_path("a/../../escape.txt").is_err());

        assert!(validate_relative_path("a/b/../c.txt").is_ok());
    }

    #[test]
    fn test_apply_simple_update() {
        let original = "foo\nbar\nbaz\n";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec!["bar".to_string()],
            new_lines: vec!["BAR".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "foo\nBAR\nbaz\n");
    }

    #[test]
    fn test_apply_with_context() {
        let original = "foo\nbar\nbaz\nqux\n";
        let chunks = vec![UpdateChunk {
            change_context: vec!["foo".to_string()],
            old_lines: vec!["bar".to_string()],
            new_lines: vec!["BAR".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "foo\nBAR\nbaz\nqux\n");
    }

    #[test]
    fn test_apply_multiple_chunks() {
        let original = "a\nb\nc\nd\ne\nf\n";
        let chunks = vec![
            UpdateChunk {
                change_context: vec!["a".to_string()],
                old_lines: vec!["b".to_string()],
                new_lines: vec!["B".to_string()],
                is_end_of_file: false,
            },
            UpdateChunk {
                change_context: vec!["d".to_string()],
                old_lines: vec!["e".to_string()],
                new_lines: vec!["E".to_string()],
                is_end_of_file: false,
            },
        ];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "a\nB\nc\nd\nE\nf\n");
    }

    #[test]
    fn test_apply_eof_insertion() {
        let original = "foo\nbar\n";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec![],
            new_lines: vec!["baz".to_string()],
            is_end_of_file: true,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "foo\nbar\nbaz\n");
    }

    #[test]
    fn test_apply_ensures_trailing_newline() {
        let original = "foo";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec!["foo".to_string()],
            new_lines: vec!["bar".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert!(result.ends_with('\n'));
        assert_eq!(result, "bar\n");
    }

    #[test]
    fn test_unicode_normalization() {
        let original = "import foo \u{2013} comment\n";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec!["import foo - comment".to_string()],
            new_lines: vec!["import bar".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "import bar\n");
    }

    #[test]
    fn test_no_begin_patch() {
        let patch = "*** Add File: test.txt\n+content";
        let result = parse_patch(patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_patch() {
        let patch = "*** Begin Patch\n*** End Patch";
        let result = parse_patch(&patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_backslash_traversal_rejected() {
        assert!(validate_relative_path("a\\..\\..\\secret").is_err());
        assert!(validate_relative_path("path\\to\\file.txt").is_err());
    }

    #[test]
    fn test_update_requires_hunks() {
        let patch = wrap_patch("*** Update File: file.txt");
        let result = parse_patch(&patch);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("at least one hunk"));
    }

    #[test]
    fn test_empty_move_to_rejected() {
        let patch = wrap_patch("*** Update File: file.txt\n*** Move to:\n@@ \n-old\n+new");
        let result = parse_patch(&patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_end_patch() {
        let patch = "*** Begin Patch\n*** Add File: test.txt\n+content";
        let result = parse_patch(patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("End Patch"));
    }

    #[test]
    fn test_add_file_strict_plus_prefix() {
        let patch = wrap_patch("*** Add File: test.txt\n+line1\nunprefixed line\n+line2");
        let result = parse_patch(&patch);
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_insertions_same_location() {
        let original = "a\nb\nc\n";
        let chunks = vec![
            UpdateChunk {
                change_context: vec![],
                old_lines: vec![],
                new_lines: vec!["X".to_string()],
                is_end_of_file: true,
            },
            UpdateChunk {
                change_context: vec![],
                old_lines: vec![],
                new_lines: vec!["Y".to_string()],
                is_end_of_file: true,
            },
        ];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert!(result.contains("X"));
        assert!(result.contains("Y"));
    }

    #[test]
    fn test_interleaved_changes() {
        let original = "a\nb\nc\nd\ne\nf\n";
        let chunks = vec![
            UpdateChunk {
                change_context: vec!["a".to_string()],
                old_lines: vec!["b".to_string()],
                new_lines: vec!["B".to_string()],
                is_end_of_file: false,
            },
            UpdateChunk {
                change_context: vec!["d".to_string()],
                old_lines: vec!["e".to_string()],
                new_lines: vec!["E".to_string()],
                is_end_of_file: false,
            },
            UpdateChunk {
                change_context: vec!["f".to_string()],
                old_lines: vec![],
                new_lines: vec!["g".to_string()],
                is_end_of_file: true,
            },
        ];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "a\nB\nc\nd\nE\nf\ng\n");
    }

    #[test]
    fn test_context_based_insertion_not_eof() {
        let original = "fn a() {}\n\nfn b() {}\n";
        let chunks = vec![UpdateChunk {
            change_context: vec!["fn a() {}".to_string()],
            old_lines: vec![],
            new_lines: vec!["".to_string(), "fn inserted() {}".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert!(result.contains("fn a() {}\n\nfn inserted() {}"));
        assert!(result.find("fn inserted()").unwrap() < result.find("fn b()").unwrap());
    }

    #[test]
    fn test_eof_hunk_matches_last_occurrence() {
        let original = "block\nend\n\nblock\nend\n";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec!["block".to_string(), "end".to_string()],
            new_lines: vec!["BLOCK".to_string(), "END".to_string()],
            is_end_of_file: true,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "block\nend\n\nBLOCK\nEND\n");
    }

    #[test]
    fn test_whitespace_tolerant_matching() {
        let original = "foo  \nbar\t\nbaz\n";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec!["foo".to_string(), "bar".to_string()],
            new_lines: vec!["FOO".to_string(), "BAR".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "FOO\nBAR\nbaz\n");
    }

    #[test]
    fn test_trailing_whitespace_in_patch() {
        let original = "line1\nline2\nline3\n";
        let chunks = vec![UpdateChunk {
            change_context: vec![],
            old_lines: vec!["line2  ".to_string()],
            new_lines: vec!["LINE2".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "line1\nLINE2\nline3\n");
    }

    #[test]
    fn test_sequential_insertions_at_cursor() {
        let original = "header\n\nfooter\n";
        let chunks = vec![
            UpdateChunk {
                change_context: vec!["header".to_string()],
                old_lines: vec![],
                new_lines: vec!["insert1".to_string()],
                is_end_of_file: false,
            },
            UpdateChunk {
                change_context: vec![],
                old_lines: vec![],
                new_lines: vec!["insert2".to_string()],
                is_end_of_file: false,
            },
        ];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert!(result.contains("insert1"));
        assert!(result.contains("insert2"));
        let pos1 = result.find("insert1").unwrap();
        let pos2 = result.find("insert2").unwrap();
        assert!(pos1 < pos2);
    }

    #[test]
    fn test_non_eof_insertion_uses_cursor() {
        let original = "a\nb\nc\nd\n";
        let chunks = vec![UpdateChunk {
            change_context: vec!["b".to_string()],
            old_lines: vec![],
            new_lines: vec!["X".to_string()],
            is_end_of_file: false,
        }];
        let result = apply_update_chunks(original, &chunks).unwrap();
        assert_eq!(result, "a\nb\nX\nc\nd\n");
    }
}
