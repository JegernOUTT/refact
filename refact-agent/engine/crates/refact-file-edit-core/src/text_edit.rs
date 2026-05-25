use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn parse_string_arg(
    args: &HashMap<String, Value>,
    name: &str,
    hint: &str,
) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Err(format!("⚠️ '{}' must be a string, got: {:?}", name, v)),
        None => Err(format!("⚠️ Missing '{}'. 💡 {}", name, hint)),
    }
}

pub fn parse_bool_arg(
    args: &HashMap<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(name) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.to_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("⚠️ '{}' must be true/false, got: {}", name, s)),
        },
        Some(v) => Err(format!("⚠️ '{}' must be a boolean, got: {:?}", name, v)),
        None => Ok(default),
    }
}

pub fn edit_result_summary(before: &str, after: &str, path: &PathBuf) -> String {
    let before_lines = before.lines().count();
    let after_lines = after.lines().count();
    let diff = after_lines as i64 - before_lines as i64;
    let sign = if diff >= 0 { "+" } else { "" };
    format!(
        "✅ Updated {:?}: {} → {} lines ({}{})",
        path.file_name().unwrap_or_default(),
        before_lines,
        after_lines,
        sign,
        diff
    )
}

pub fn normalize_line_endings(content: &str) -> String {
    normalize_all_line_endings(content)
}

fn normalize_all_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn restore_line_endings(content: &str, original_had_crlf: bool) -> String {
    if original_had_crlf {
        normalize_all_line_endings(content).replace('\n', "\r\n")
    } else {
        content.to_string()
    }
}

pub fn strip_line_number_prefixes(s: &str) -> String {
    let re = regex::Regex::new(r"(?m)^\d+[\t|:]\s?").unwrap();
    let non_empty_lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    if non_empty_lines.is_empty() || !non_empty_lines.iter().all(|l| re.is_match(l)) {
        return s.to_string();
    }
    re.replace_all(s, "").to_string()
}

pub fn find_match_lines(content: &str, pattern: &str) -> Vec<usize> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut pos = 0;
    while let Some(idx) = content[pos..].find(pattern) {
        let abs_idx = pos + idx;
        let line_num = content[..abs_idx].lines().count() + 1;
        lines.push(line_num);
        pos = abs_idx + pattern.len();
    }
    lines
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnchorMode {
    ReplaceBetween,
    InsertAfter,
    InsertBefore,
}

pub fn replace_between_anchors(
    content: &str,
    before: &str,
    after: &str,
    replacement: &str,
    multiple: bool,
) -> Result<String, String> {
    if before.is_empty() {
        return Err(
            "⚠️ anchor_before cannot be empty. 💡 Provide unique text before the replacement"
                .to_string(),
        );
    }
    if after.is_empty() {
        return Err(
            "⚠️ anchor_after cannot be empty. 💡 Provide unique text after the replacement"
                .to_string(),
        );
    }

    let before_positions: Vec<usize> = content.match_indices(before).map(|(i, _)| i).collect();
    if before_positions.is_empty() {
        return Err("⚠️ anchor_before not found. 💡 Use cat() to verify text exists".to_string());
    }

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for &b_start in &before_positions {
        let b_end = b_start + before.len();
        if let Some(rel_a) = content[b_end..].find(after) {
            pairs.push((b_start, b_end + rel_a));
        }
    }

    if pairs.is_empty() {
        return Err(
            "⚠️ anchor_after not found after anchor_before. 💡 Check anchor order".to_string(),
        );
    }
    if !multiple && pairs.len() > 1 {
        let lines: Vec<usize> = pairs
            .iter()
            .map(|(i, _)| content[..*i].lines().count() + 1)
            .collect();
        return Err(format!(
            "⚠️ {} anchor pairs at lines {:?}. 💡 Use more specific anchors, or set multiple:true",
            pairs.len(),
            lines
        ));
    }

    pairs.sort_by_key(|(start, _)| *start);
    for i in 1..pairs.len() {
        let prev_end = pairs[i - 1].1 + after.len();
        let curr_start = pairs[i].0;
        if curr_start < prev_end {
            let line1 = content[..pairs[i - 1].0].lines().count() + 1;
            let line2 = content[..curr_start].lines().count() + 1;
            return Err(format!(
                "⚠️ Overlapping anchor regions at lines {} and {}. 💡 Use more specific anchors",
                line1, line2
            ));
        }
    }

    let mut result = content.to_string();
    for (b_start, a_start) in pairs.into_iter().rev() {
        let b_end = b_start + before.len();
        let a_end = a_start + after.len();
        result = format!(
            "{}{}{}{}",
            &result[..b_end],
            replacement,
            after,
            &result[a_end..]
        );
    }
    Ok(result)
}

pub fn insert_at_anchor(
    content: &str,
    anchor: &str,
    insert: &str,
    multiple: bool,
    after: bool,
) -> Result<String, String> {
    if anchor.is_empty() {
        return Err(
            "⚠️ Anchor cannot be empty. 💡 Provide unique text to locate edit position".to_string(),
        );
    }

    let positions: Vec<usize> = content.match_indices(anchor).map(|(i, _)| i).collect();
    if positions.is_empty() {
        return Err("⚠️ Anchor not found. 💡 Use cat() to verify text exists".to_string());
    }
    if !multiple && positions.len() > 1 {
        let lines: Vec<usize> = positions
            .iter()
            .map(|i| content[..*i].lines().count() + 1)
            .collect();
        return Err(format!("⚠️ {} anchor occurrences at lines {:?}. 💡 Use more specific anchor, or set multiple:true", positions.len(), lines));
    }

    let mut result = content.to_string();
    for pos in positions.into_iter().rev() {
        let insert_pos = if after { pos + anchor.len() } else { pos };
        result.insert_str(insert_pos, insert);
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

pub fn parse_line_ranges(ranges_str: &str, total_lines: usize) -> Result<Vec<LineRange>, String> {
    let mut ranges = Vec::new();

    for part in ranges_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let range = if part.contains(':') {
            let parts: Vec<&str> = part.splitn(2, ':').collect();
            let start_str = parts[0].trim();
            let end_str = parts[1].trim();

            let start = if start_str.is_empty() {
                1
            } else {
                start_str.parse::<usize>().map_err(|_| {
                    format!(
                        "⚠️ Invalid start '{}' in '{}'. 💡 Use numbers like '10:20'",
                        start_str, part
                    )
                })?
            };

            let end = if end_str.is_empty() {
                total_lines
            } else {
                end_str.parse::<usize>().map_err(|_| {
                    format!(
                        "⚠️ Invalid end '{}' in '{}'. 💡 Use numbers like '10:20'",
                        end_str, part
                    )
                })?
            };

            LineRange { start, end }
        } else {
            let line = part.parse::<usize>().map_err(|_| {
                format!(
                    "⚠️ Invalid line '{}'. 💡 Use number like '10' or range '10:20'",
                    part
                )
            })?;
            LineRange {
                start: line,
                end: line,
            }
        };

        if range.start == 0 {
            return Err("⚠️ Line numbers are 1-based, got 0. 💡 Use 1 for first line".to_string());
        }
        if range.end < range.start {
            return Err(format!(
                "⚠️ Invalid range '{}': end ({}) < start ({}). 💡 Use start:end format",
                part, range.end, range.start
            ));
        }
        if range.start > total_lines && !(total_lines == 0 && range.start == 1) {
            return Err(format!(
                "⚠️ Line {} beyond EOF ({} lines). 💡 Use cat() to check file length",
                range.start, total_lines
            ));
        }

        ranges.push(range);
    }

    if ranges.is_empty() {
        return Err("⚠️ No ranges provided. 💡 Use format '10:20' or '5' or ':10,20:'".to_string());
    }

    let mut sorted: Vec<&LineRange> = ranges.iter().collect();
    sorted.sort_by_key(|r| r.start);

    for i in 1..sorted.len() {
        let prev = sorted[i - 1];
        let curr = sorted[i];
        if curr.start <= prev.end {
            return Err(format!(
                "⚠️ Overlapping ranges {}:{} and {}:{}. 💡 Ranges must not overlap",
                prev.start, prev.end, curr.start, curr.end
            ));
        }
    }

    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    #[test]
    fn parse_string_arg_accepts_strings_and_rejects_missing_or_wrong_type() {
        let args = args(vec![("path", json!("file.txt")), ("count", json!(1))]);
        assert_eq!(parse_string_arg(&args, "path", "hint").unwrap(), "file.txt");
        assert!(parse_string_arg(&args, "missing", "hint")
            .unwrap_err()
            .contains("Missing 'missing'"));
        assert!(parse_string_arg(&args, "count", "hint")
            .unwrap_err()
            .contains("must be a string"));
    }

    #[test]
    fn parse_bool_arg_accepts_booleans_strings_and_default() {
        let args = args(vec![
            ("enabled", json!(true)),
            ("disabled", json!("false")),
            ("bad", json!("maybe")),
        ]);
        assert!(parse_bool_arg(&args, "enabled", false).unwrap());
        assert!(!parse_bool_arg(&args, "disabled", true).unwrap());
        assert!(parse_bool_arg(&args, "missing", true).unwrap());
        assert!(parse_bool_arg(&args, "bad", false)
            .unwrap_err()
            .contains("must be true/false"));
    }

    #[test]
    fn edit_result_summary_reports_line_delta() {
        let path = PathBuf::from("/path/to/file.rs");
        let summary = edit_result_summary("a\nb\nc", "a\nb\nc\nd\ne", &path);
        assert!(summary.contains("file.rs"));
        assert!(summary.contains("3"));
        assert!(summary.contains("5"));
        assert!(summary.contains("+2"));
    }

    #[test]
    fn normalizes_and_restores_line_endings() {
        assert_eq!(normalize_line_endings("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize_line_endings("a\rb\r"), "a\nb\n");
        assert_eq!(normalize_line_endings("a\nb\n"), "a\nb\n");
        assert_eq!(restore_line_endings("a\nb\n", true), "a\r\nb\r\n");
        assert_eq!(restore_line_endings("a\nb\n", false), "a\nb\n");
        assert_eq!(restore_line_endings("a\r\nb\r\n", true), "a\r\nb\r\n");
        assert_eq!(restore_line_endings("a\rb\r", true), "a\r\nb\r\n");
    }

    #[test]
    fn strip_line_number_prefixes_strips_only_fully_numbered_text() {
        assert_eq!(strip_line_number_prefixes("1: foo\n2: bar"), "foo\nbar");
        assert_eq!(strip_line_number_prefixes("1\tfoo\n2\tbar"), "foo\nbar");
        assert_eq!(strip_line_number_prefixes("10|foo\n20|bar"), "foo\nbar");
        let mixed = "8080: service-a\nsome-other-line";
        assert_eq!(strip_line_number_prefixes(mixed), mixed);
        let indented = "    8080: \"http\"\n    9090: \"grpc\"";
        assert_eq!(strip_line_number_prefixes(indented), indented);
        assert_eq!(strip_line_number_prefixes(""), "");
    }

    #[test]
    fn find_match_lines_reports_lines_and_ignores_empty_pattern() {
        let content = "line1\nfoo\nline3\nfoo\nline5";
        assert_eq!(find_match_lines(content, "foo"), vec![2, 4]);
        assert_eq!(find_match_lines("some content", ""), Vec::<usize>::new());
        assert_eq!(find_match_lines("abcabc", "abc").len(), 2);
        assert_eq!(find_match_lines("a\na\na", "a"), vec![1, 2, 3]);
    }

    #[test]
    fn replace_between_anchors_replaces_single_and_multiple_regions() {
        let content = "start\nBEGIN\nold\nEND\nfinish";
        let result = replace_between_anchors(content, "BEGIN\n", "END", "new\n", false).unwrap();
        assert_eq!(result, "start\nBEGIN\nnew\nEND\nfinish");

        let content = "A\nBEGIN\nx\nEND\nB\nBEGIN\ny\nEND\nC";
        let result = replace_between_anchors(content, "BEGIN\n", "END", "z\n", true).unwrap();
        assert_eq!(result, "A\nBEGIN\nz\nEND\nB\nBEGIN\nz\nEND\nC");
    }

    #[test]
    fn replace_between_anchors_reports_empty_missing_repeated_and_overlapping_regions() {
        assert!(
            replace_between_anchors("BEGIN\nold\nEND", "", "END", "x", false)
                .unwrap_err()
                .contains("anchor_before cannot be empty")
        );
        assert!(
            replace_between_anchors("BEGIN\nold\nEND", "BEGIN", "", "x", false)
                .unwrap_err()
                .contains("anchor_after cannot be empty")
        );
        assert!(replace_between_anchors("no anchors here", "BEGIN", "END", "x", false).is_err());
        let repeated = "A\nBEGIN\nx\nEND\nB\nBEGIN\ny\nEND\nC";
        assert!(
            replace_between_anchors(repeated, "BEGIN\n", "END", "z\n", false)
                .unwrap_err()
                .contains("anchor pairs")
        );
        assert!(replace_between_anchors("A{B{C}D}E", "{", "}", "x", true)
            .unwrap_err()
            .contains("Overlapping anchor regions"));
    }

    #[test]
    fn insert_at_anchor_inserts_before_after_and_multiple() {
        let content = "line1\nANCHOR\nline3";
        let result = insert_at_anchor(content, "ANCHOR", "\ninserted", false, true).unwrap();
        assert_eq!(result, "line1\nANCHOR\ninserted\nline3");
        let result = insert_at_anchor(content, "ANCHOR", "inserted\n", false, false).unwrap();
        assert_eq!(result, "line1\ninserted\nANCHOR\nline3");
        let result = insert_at_anchor("A\nA", "A", "x", true, true).unwrap();
        assert_eq!(result, "Ax\nAx");
    }

    #[test]
    fn insert_at_anchor_reports_empty_missing_or_repeated_unique_anchor() {
        assert!(insert_at_anchor("content", "", "x", false, true)
            .unwrap_err()
            .contains("Anchor cannot be empty"));
        assert!(insert_at_anchor("content", "MISSING", "x", false, true).is_err());
        assert!(insert_at_anchor("A\nA\nA", "A", "x", false, true)
            .unwrap_err()
            .contains("anchor occurrences"));
    }

    #[test]
    fn parse_line_ranges_accepts_single_multiple_and_open_ended_ranges() {
        assert!(parse_line_ranges("5", 10).is_ok());
        assert!(parse_line_ranges("1:10", 10).is_ok());
        assert!(parse_line_ranges(":5", 10).is_ok());
        assert!(parse_line_ranges("5:", 10).is_ok());
        let ranges = parse_line_ranges("1:3,7:9", 10).unwrap();
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn parse_line_ranges_preserves_order_and_supports_empty_file_line_one() {
        let ranges = parse_line_ranges("4:4,2:2", 10).unwrap();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].start, 4);
        assert_eq!(ranges[1].start, 2);

        let ranges = parse_line_ranges("1:1", 0).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 1);
        assert_eq!(ranges[0].end, 1);
        assert!(parse_line_ranges("2:2", 0).is_err());
    }

    #[test]
    fn parse_line_ranges_reports_invalid_and_overlapping_ranges() {
        assert!(parse_line_ranges("0", 10).is_err());
        assert!(parse_line_ranges("5:3", 10).is_err());
        assert!(parse_line_ranges("15", 10).is_err());
        assert!(parse_line_ranges("abc", 10).is_err());
        assert!(parse_line_ranges("", 10).is_err());
        assert!(parse_line_ranges("1:5,3:7", 10).is_err());
        assert!(parse_line_ranges("1:5,5:7", 10).is_err());
    }
}
