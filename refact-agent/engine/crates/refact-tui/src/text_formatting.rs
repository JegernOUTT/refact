// Adapted from openai/codex codex-rs/tui/src/text_formatting.rs, Apache-2.0.
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn capitalize_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => {
            let mut capitalized = first.to_uppercase().collect::<String>();
            capitalized.push_str(chars.as_str());
            capitalized
        }
        None => String::new(),
    }
}

pub fn format_and_truncate_tool_result(text: &str, max_lines: usize, line_width: usize) -> String {
    let max_graphemes = max_lines
        .saturating_mul(line_width)
        .saturating_sub(max_lines);
    let display_text = format_json_compact(text).unwrap_or_else(|| text.to_string());
    truncate_text(&display_text, max_graphemes)
}

pub fn format_json_compact(text: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let json_pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());
    let mut result = String::new();
    let mut chars = json_pretty.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if !escape_next => {
                in_string = !in_string;
                result.push(ch);
            }
            '\\' if in_string => {
                escape_next = !escape_next;
                result.push(ch);
            }
            '\n' | '\r' if !in_string => {}
            ' ' | '\t' if !in_string => {
                let next_ch = chars.peek().copied();
                let last_ch = result.chars().last();
                if matches!(last_ch, Some(':') | Some(',')) && !matches!(next_ch, Some('}' | ']')) {
                    result.push(' ');
                }
            }
            _ => {
                if escape_next && in_string {
                    escape_next = false;
                }
                result.push(ch);
            }
        }
    }

    Some(result)
}

pub fn truncate_text(text: &str, max_graphemes: usize) -> String {
    if max_graphemes == 0 {
        return String::new();
    }

    if text.graphemes(true).nth(max_graphemes).is_none() {
        return text.to_string();
    }

    let mut truncated = text
        .graphemes(true)
        .take(max_graphemes.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

struct PathSegment<'a> {
    original: &'a str,
    text: String,
    truncatable: bool,
    is_suffix: bool,
}

struct PathMetrics {
    prefix_widths: Vec<usize>,
    suffix_widths: Vec<usize>,
    prefix_nonempty: Vec<usize>,
    suffix_nonempty: Vec<usize>,
    separator_width: usize,
}

impl PathMetrics {
    fn new(segments: &[&str], max_width: usize, separator_width: usize) -> Self {
        let segment_widths = segments
            .iter()
            .map(|segment| {
                let width = UnicodeWidthStr::width(*segment);
                if width > max_width {
                    UnicodeWidthStr::width("…")
                } else {
                    width
                }
            })
            .collect::<Vec<_>>();

        let mut prefix_widths: Vec<usize> = Vec::with_capacity(segments.len() + 1);
        let mut prefix_nonempty: Vec<usize> = Vec::with_capacity(segments.len() + 1);
        prefix_widths.push(0usize);
        prefix_nonempty.push(0usize);
        for (segment, width) in segments.iter().zip(&segment_widths) {
            prefix_widths.push(
                prefix_widths
                    .last()
                    .copied()
                    .unwrap_or(0usize)
                    .saturating_add(*width),
            );
            prefix_nonempty.push(
                prefix_nonempty
                    .last()
                    .copied()
                    .unwrap_or(0usize)
                    .saturating_add(usize::from(!segment.is_empty())),
            );
        }

        let mut suffix_widths = vec![0; segments.len() + 1];
        let mut suffix_nonempty = vec![0; segments.len() + 1];
        for index in (0..segments.len()).rev() {
            suffix_widths[index] = segment_widths[index].saturating_add(suffix_widths[index + 1]);
            suffix_nonempty[index] =
                usize::from(!segments[index].is_empty()).saturating_add(suffix_nonempty[index + 1]);
        }

        Self {
            prefix_widths,
            suffix_widths,
            prefix_nonempty,
            suffix_nonempty,
            separator_width,
        }
    }

    fn candidate_width(
        &self,
        segments: &[&str],
        has_leading_sep: bool,
        left_count: usize,
        right_count: usize,
    ) -> usize {
        let suffix_start = segments.len().saturating_sub(right_count);
        let text_width = self.prefix_widths[left_count]
            .saturating_add(UnicodeWidthStr::width("…"))
            .saturating_add(self.suffix_widths[suffix_start]);
        let nonempty_count = self.prefix_nonempty[left_count]
            .saturating_add(1)
            .saturating_add(self.suffix_nonempty[suffix_start]);
        let separator_count = nonempty_count
            .saturating_sub(1)
            .saturating_add(usize::from(has_leading_sep));
        text_width.saturating_add(self.separator_width.saturating_mul(separator_count))
    }
}

fn front_truncate(original: &str, allowed_width: usize) -> String {
    if allowed_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(original) <= allowed_width {
        return original.to_string();
    }
    if allowed_width <= UnicodeWidthStr::width("…") {
        return "…".to_string();
    }

    let mut kept = Vec::new();
    let mut used_width = UnicodeWidthStr::width("…");
    for grapheme in original.graphemes(true).rev() {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used_width.saturating_add(grapheme_width) > allowed_width {
            break;
        }
        used_width = used_width.saturating_add(grapheme_width);
        kept.push(grapheme);
    }
    kept.reverse();

    let mut truncated = String::from("…");
    for grapheme in kept {
        truncated.push_str(grapheme);
    }
    truncated
}

fn assemble_path(has_leading_sep: bool, sep: char, segments: &[PathSegment<'_>]) -> String {
    let mut result = String::new();
    if has_leading_sep {
        result.push(sep);
    }
    for segment in segments {
        if !result.is_empty() && !result.ends_with(sep) {
            result.push(sep);
        }
        result.push_str(segment.text.as_str());
    }
    result
}

fn fit_path_segments(
    has_leading_sep: bool,
    sep: char,
    segments: &mut [PathSegment<'_>],
    max_width: usize,
    segment_count: usize,
    allow_front_truncate: bool,
) -> Option<String> {
    loop {
        let candidate = assemble_path(has_leading_sep, sep, segments);
        let width = UnicodeWidthStr::width(candidate.as_str());
        if width <= max_width {
            return Some(candidate);
        }
        if !allow_front_truncate {
            return None;
        }

        let mut changed = false;
        'segments: for is_suffix in [true, false] {
            for index in (0..segments.len()).rev() {
                let segment = &mut segments[index];
                if !segment.truncatable || segment.is_suffix != is_suffix {
                    continue;
                }

                let original_width = UnicodeWidthStr::width(segment.original);
                if original_width <= max_width && segment_count > 2 {
                    continue;
                }

                let segment_width = UnicodeWidthStr::width(segment.text.as_str());
                let other_width = width.saturating_sub(segment_width);
                let allowed_width = max_width.saturating_sub(other_width).max(1);
                let new_text = front_truncate(segment.original, allowed_width);
                if new_text != segment.text {
                    segment.text = new_text;
                    changed = true;
                    break 'segments;
                }
            }
        }

        if !changed {
            return None;
        }
    }
}

fn build_path_candidate<'a>(
    segments: &[&'a str],
    left_count: usize,
    right_count: usize,
) -> Vec<PathSegment<'a>> {
    let mut candidate = segments[..left_count]
        .iter()
        .map(|segment| PathSegment {
            original: segment,
            text: (*segment).to_string(),
            truncatable: true,
            is_suffix: false,
        })
        .collect::<Vec<_>>();

    candidate.push(PathSegment {
        original: "…",
        text: "…".to_string(),
        truncatable: false,
        is_suffix: false,
    });
    candidate.extend(
        segments[segments.len() - right_count..]
            .iter()
            .map(|segment| PathSegment {
                original: segment,
                text: (*segment).to_string(),
                truncatable: true,
                is_suffix: true,
            }),
    );
    candidate
}

fn center_truncate_path_with_candidate_count(path: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 {
        return (String::new(), 0);
    }
    if UnicodeWidthStr::width(path) <= max_width {
        return (path.to_string(), 0);
    }

    let sep = std::path::MAIN_SEPARATOR;
    let has_leading_sep = path.starts_with(sep);
    let has_trailing_sep = path.ends_with(sep);
    let mut raw_segments: Vec<&str> = path.split(sep).collect();
    if has_leading_sep && !raw_segments.is_empty() && raw_segments[0].is_empty() {
        raw_segments.remove(0);
    }
    if has_trailing_sep
        && !raw_segments.is_empty()
        && raw_segments.last().is_some_and(|last| last.is_empty())
    {
        raw_segments.pop();
    }

    if raw_segments.is_empty() {
        if has_leading_sep {
            let root = sep.to_string();
            if UnicodeWidthStr::width(root.as_str()) <= max_width {
                return (root, 0);
            }
        }
        return ("…".to_string(), 0);
    }

    let segment_count = raw_segments.len();
    if segment_count <= 2 {
        let (left_count, right_count) = if segment_count == 1 { (1, 0) } else { (1, 1) };
        let mut candidate =
            if right_count == 0 {
                raw_segments[..left_count]
                    .iter()
                    .map(|segment| PathSegment {
                        original: segment,
                        text: (*segment).to_string(),
                        truncatable: true,
                        is_suffix: false,
                    })
                    .collect()
            } else {
                let mut candidate = raw_segments[..left_count]
                    .iter()
                    .map(|segment| PathSegment {
                        original: segment,
                        text: (*segment).to_string(),
                        truncatable: true,
                        is_suffix: false,
                    })
                    .collect::<Vec<_>>();
                candidate.extend(raw_segments[segment_count - right_count..].iter().map(
                    |segment| PathSegment {
                        original: segment,
                        text: (*segment).to_string(),
                        truncatable: true,
                        is_suffix: true,
                    },
                ));
                candidate
            };
        if let Some(candidate) = fit_path_segments(
            has_leading_sep,
            sep,
            &mut candidate,
            max_width,
            segment_count,
            true,
        ) {
            return (candidate, 1);
        }
        return (front_truncate(path, max_width), 1);
    }

    let metrics = PathMetrics::new(
        &raw_segments,
        max_width,
        UnicodeWidthStr::width(sep.to_string().as_str()),
    );
    let mut candidate_count = 0;
    let desired_suffix = 2;
    let mut right_count = desired_suffix;

    for left_count in (1..segment_count.saturating_sub(1)).rev() {
        let max_right_count = segment_count.saturating_sub(left_count + 1);
        if max_right_count < desired_suffix {
            continue;
        }

        candidate_count += 1;
        if metrics.candidate_width(&raw_segments, has_leading_sep, left_count, right_count)
            > max_width
        {
            continue;
        }
        while right_count < max_right_count {
            candidate_count += 1;
            if metrics.candidate_width(&raw_segments, has_leading_sep, left_count, right_count + 1)
                > max_width
            {
                break;
            }
            right_count += 1;
        }

        let mut candidate = build_path_candidate(&raw_segments, left_count, right_count);
        if let Some(candidate) = fit_path_segments(
            has_leading_sep,
            sep,
            &mut candidate,
            max_width,
            segment_count,
            true,
        ) {
            return (candidate, candidate_count);
        }
    }

    for left_count in (1..segment_count.saturating_sub(1)).rev() {
        candidate_count += 1;
        if metrics.candidate_width(&raw_segments, has_leading_sep, left_count, 1) > max_width {
            continue;
        }

        let mut candidate = build_path_candidate(&raw_segments, left_count, 1);
        if let Some(candidate) = fit_path_segments(
            has_leading_sep,
            sep,
            &mut candidate,
            max_width,
            segment_count,
            true,
        ) {
            return (candidate, candidate_count);
        }
    }

    (front_truncate(path, max_width), candidate_count)
}

pub fn center_truncate_path(path: &str, max_width: usize) -> String {
    center_truncate_path_with_candidate_count(path, max_width).0
}

pub fn format_tokens_compact(value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    if value < 1_000 {
        return value.to_string();
    }

    if value < 1_000_000 && value >= 999_950 {
        return "1.0M".to_string();
    }
    if value < 1_000_000_000 && value >= 999_950_000 {
        return "1.0B".to_string();
    }
    if value < 1_000_000_000_000 && value >= 999_950_000_000 {
        return "1.0T".to_string();
    }

    let value_f64 = value as f64;
    let (scaled, suffix) = if value >= 1_000_000_000_000 {
        (value_f64 / 1_000_000_000_000.0, "T")
    } else if value >= 1_000_000_000 {
        (value_f64 / 1_000_000_000.0, "B")
    } else if value >= 1_000_000 {
        (value_f64 / 1_000_000.0, "M")
    } else {
        (value_f64 / 1_000.0, "K")
    };

    let decimals = if scaled < 10.0 {
        2
    } else if scaled < 100.0 {
        1
    } else {
        0
    };
    let mut formatted = format!("{scaled:.decimals$}");
    if formatted == "1000" {
        formatted = "999".to_string();
    }
    if formatted.contains('.') {
        while formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }
    format!("{formatted}{suffix}")
}

pub fn proper_join<T: AsRef<str>>(items: &[T]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].as_ref().to_string(),
        2 => format!("{} and {}", items[0].as_ref(), items[1].as_ref()),
        _ => {
            let last = items[items.len() - 1].as_ref();
            let mut result = String::new();

            for (idx, item) in items.iter().take(items.len() - 1).enumerate() {
                if idx > 0 {
                    result.push_str(", ");
                }
                result.push_str(item.as_ref());
            }

            format!("{result} and {last}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_text_uses_unicode_ellipsis_and_grapheme_boundaries() {
        let family = "👨‍👩‍👧‍👦";
        assert_eq!(
            truncate_text(&format!("ab{family}cd"), 4),
            format!("ab{family}…")
        );
        assert_eq!(truncate_text("Hello", 0), "");
        assert_eq!(truncate_text("Hello", 1), "…");
        assert_eq!(truncate_text("Hi", 10), "Hi");
        assert_eq!(truncate_text("Hello", 5), "Hello");
    }

    #[test]
    fn format_json_compact_formats_single_line_with_spaces() {
        let json = r#"{ "user": { "name": "John", "details": { "age": 30, "city": "NYC" } } }"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(
            result,
            r#"{"user": {"name": "John", "details": {"age": 30, "city": "NYC"}}}"#
        );
        assert_eq!(
            format_json_compact(r#"[ 1, 2, { "key": "value" }, "string" ]"#).unwrap(),
            r#"[1, 2, {"key": "value"}, "string"]"#
        );
        assert!(format_json_compact(r#"{"invalid": json syntax}"#).is_none());
    }

    #[test]
    fn format_and_truncate_tool_result_compacts_json_before_truncating() {
        let result = format_and_truncate_tool_result(r#"{"compact":true,"items":[1,2,3]}"#, 2, 16);
        assert_eq!(result, r#"{"compact": true, "items": [1…"#);
    }

    #[test]
    fn format_and_truncate_tool_result_saturates_capacity_math() {
        assert_eq!(
            format_and_truncate_tool_result("x", usize::MAX, usize::MAX),
            ""
        );
    }

    #[test]
    fn center_truncate_path_preserves_status_card_output() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("~{sep}hello{sep}the{sep}fox{sep}is{sep}very{sep}fast");
        let truncated = center_truncate_path(&path, 24);
        assert_eq!(
            truncated,
            format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast")
        );
    }

    #[test]
    fn center_truncate_path_front_truncates_long_segment() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("~{sep}supercalifragilisticexpialidocious");
        let truncated = center_truncate_path(&path, 18);
        assert_eq!(truncated, format!("~{sep}…cexpialidocious"));
    }

    #[test]
    fn front_truncate_preserves_combining_and_zwj_graphemes() {
        assert_eq!(front_truncate("prefixe\u{301}", 2), "…e\u{301}");
        let family = "👨‍👩‍👧‍👦";
        assert_eq!(
            front_truncate(&format!("prefix{family}"), 3),
            format!("…{family}")
        );
    }

    #[test]
    fn center_truncate_path_scans_a_linear_number_of_candidates() {
        let sep = std::path::MAIN_SEPARATOR;
        let segments = (0..1_000)
            .map(|index| format!("segment-{index}"))
            .collect::<Vec<_>>();
        let path = segments.join(&sep.to_string());
        let (_, candidate_count) = center_truncate_path_with_candidate_count(&path, 10);
        assert!(candidate_count <= segments.len().saturating_mul(3));
    }

    #[test]
    fn compact_token_formatter_uses_expected_suffixes() {
        assert_eq!(format_tokens_compact(0), "0");
        assert_eq!(format_tokens_compact(999), "999");
        assert_eq!(format_tokens_compact(1_234), "1.23K");
        assert_eq!(format_tokens_compact(12_340), "12.3K");
        assert_eq!(format_tokens_compact(123_400), "123K");
        assert_eq!(format_tokens_compact(999_949), "999K");
        assert_eq!(format_tokens_compact(999_950), "1.0M");
        assert_eq!(format_tokens_compact(1_234_000), "1.23M");
        assert_eq!(format_tokens_compact(1_234_000_000), "1.23B");
        assert_eq!(format_tokens_compact(1_234_000_000_000), "1.23T");
    }

    #[test]
    fn small_text_helpers_format_expected_strings() {
        let empty: Vec<String> = vec![];
        assert_eq!(proper_join(&empty), "");
        assert_eq!(proper_join(&["apple"]), "apple");
        assert_eq!(proper_join(&["apple", "banana"]), "apple and banana");
        assert_eq!(
            proper_join(&["apple", "banana", "cherry"]),
            "apple, banana and cherry"
        );
        assert_eq!(capitalize_first("hello"), "Hello");
        assert_eq!(capitalize_first(""), "");
    }
}
