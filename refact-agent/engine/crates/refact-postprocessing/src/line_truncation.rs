use crate::config::limits;

const HEAD_PERCENT: usize = 70;

pub fn truncate_tool_line(line: &str) -> String {
    truncate_line(line, limits().tool_max_line_chars, "tool_max_line_chars")
}

pub fn truncate_context_line(line: &str) -> String {
    let current = limits();
    if current.tool_max_line_chars <= current.max_line_length_chars {
        truncate_line(line, current.tool_max_line_chars, "tool_max_line_chars")
    } else {
        truncate_line(
            line,
            current.max_line_length_chars,
            "pp_max_line_length_chars",
        )
    }
}

pub fn truncate_tool_text_lines(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(text.len().min(limits().tool_max_line_chars));
    for segment in text.split_inclusive('\n') {
        let (line, newline) = segment
            .strip_suffix('\n')
            .map_or((segment, ""), |line| (line, "\n"));
        output.push_str(&truncate_tool_line(line));
        output.push_str(newline);
    }
    output
}

fn truncate_line(line: &str, max_chars: usize, setting_name: &str) -> String {
    let total_chars = line.chars().count();
    if total_chars <= max_chars {
        return line.to_string();
    }

    let mut omitted_chars = total_chars;
    let mut marker = line_marker(omitted_chars, total_chars, setting_name);
    for _ in 0..3 {
        let marker_chars = marker.chars().count();
        let kept_chars = max_chars.saturating_sub(marker_chars);
        omitted_chars = total_chars.saturating_sub(kept_chars);
        marker = line_marker(omitted_chars, total_chars, setting_name);
    }

    let marker_chars = marker.chars().count();
    if marker_chars >= max_chars {
        return marker.chars().take(max_chars).collect();
    }

    let kept_chars = max_chars - marker_chars;
    let head_chars = kept_chars * HEAD_PERCENT / 100;
    let tail_chars = kept_chars - head_chars;
    let head: String = line.chars().take(head_chars).collect();
    let tail: String = line
        .chars()
        .skip(total_chars.saturating_sub(tail_chars))
        .collect();
    format!("{head}{marker}{tail}")
}

fn line_marker(omitted_chars: usize, total_chars: usize, setting_name: &str) -> String {
    format!(
        " ... [line truncated: {omitted_chars} of {total_chars} chars omitted; raise {setting_name}] ... "
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_are_unchanged() {
        assert_eq!(truncate_line("hello", 80, "limit"), "hello");
    }

    #[test]
    fn long_lines_keep_head_tail_and_quantify_omission() {
        let line = format!("HEAD{}TAIL", "x".repeat(200));
        let truncated = truncate_line(&line, 100, "tool_max_line_chars");

        assert_eq!(truncated.chars().count(), 100);
        assert!(truncated.starts_with("HEAD"));
        assert!(truncated.ends_with("TAIL"));
        assert!(truncated.contains("chars omitted"));
        assert!(truncated.contains("tool_max_line_chars"));
    }

    #[test]
    fn truncation_preserves_unicode_boundaries() {
        let line = format!("α{}ω", "🦀".repeat(100));
        let truncated = truncate_line(&line, 90, "tool_max_line_chars");

        assert_eq!(truncated.chars().count(), 90);
        assert!(truncated.starts_with('α'));
        assert!(truncated.ends_with('ω'));
    }

    #[test]
    fn text_truncation_preserves_newlines() {
        let text = format!("short\n{}\nlast\n", "x".repeat(200));
        let truncated = truncate_tool_text_lines_with_limit(&text, 80);

        assert!(truncated.starts_with("short\n"));
        assert!(truncated.ends_with("\nlast\n"));
        assert_eq!(truncated.lines().count(), 3);
    }

    fn truncate_tool_text_lines_with_limit(text: &str, max_chars: usize) -> String {
        text.split_inclusive('\n')
            .map(|segment| {
                let (line, newline) = segment
                    .strip_suffix('\n')
                    .map_or((segment, ""), |line| (line, "\n"));
                format!(
                    "{}{}",
                    truncate_line(line, max_chars, "tool_max_line_chars"),
                    newline
                )
            })
            .collect()
    }
}
