use std::collections::HashMap;
use serde_json::Value;
use regex::Regex;

pub use refact_core::chat_types::OutputFilter;

pub fn parse_output_filter_args(
    args: &HashMap<String, Value>,
    default: &OutputFilter,
) -> OutputFilter {
    let output_filter_pattern = args
        .get("output_filter")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let output_limit = args.get("output_limit").and_then(|v| {
        v.as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_u64().map(|n| n.to_string()))
    });

    if output_filter_pattern.is_none() && output_limit.is_none() {
        return default.clone();
    }

    let is_unlimited = output_limit
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("all") || s.eq_ignore_ascii_case("full"))
        .unwrap_or(false);

    let limit_lines = if is_unlimited {
        usize::MAX
    } else {
        output_limit
            .as_deref()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default.limit_lines)
    };

    let skip_filtering = is_unlimited && output_filter_pattern.is_none();

    OutputFilter {
        limit_lines,
        limit_chars: if is_unlimited {
            usize::MAX
        } else {
            limit_lines.saturating_mul(200)
        },
        valuable_top_or_bottom: default.valuable_top_or_bottom.clone(),
        grep: output_filter_pattern.unwrap_or_else(|| {
            if is_unlimited {
                String::new()
            } else {
                default.grep.clone()
            }
        }),
        grep_context_lines: default.grep_context_lines,
        remove_from_output: default.remove_from_output.clone(),
        limit_tokens: if is_unlimited {
            None
        } else {
            Some(limit_lines.saturating_mul(50))
        },
        skip: skip_filtering,
    }
}

pub fn output_mini_postprocessing(filter: &OutputFilter, output: &str) -> String {
    if filter.skip {
        return output.to_string();
    }

    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() {
        return output.to_string();
    }

    let mut ratings: Vec<f64> = vec![0.0; lines.len()];
    let mut approve: Vec<bool> = vec![false; lines.len()];

    if filter.valuable_top_or_bottom == "bottom" {
        for i in 0..lines.len() {
            ratings[i] += 0.9 * ((i + 1) as f64) / lines.len() as f64;
        }
    } else {
        for i in 0..lines.len() {
            ratings[i] += 0.9 * (lines.len() - i) as f64 / lines.len() as f64;
        }
    }

    if !filter.grep.is_empty() {
        if let Ok(re) = Regex::new(&filter.grep) {
            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    // A requested match must survive before its surrounding context
                    // consumes the output budget.
                    ratings[i] = 2.0;
                    for j in 1..=filter.grep_context_lines {
                        if i >= j {
                            ratings[i - j] = ratings[i - j].max(1.0);
                        }
                        if i + j < lines.len() {
                            ratings[i + j] = ratings[i + j].max(1.0);
                        }
                    }
                }
            }
        }
    }

    let mut line_indices: Vec<usize> = (0..lines.len()).collect();
    line_indices.sort_by(|&a, &b| {
        ratings[b]
            .partial_cmp(&ratings[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let remove_re = if !filter.remove_from_output.is_empty() {
        Regex::new(&filter.remove_from_output).ok()
    } else {
        None
    };

    let byte_limit = filter
        .limit_tokens
        .map(|tokens| tokens.saturating_mul(4))
        .unwrap_or(usize::MAX)
        .min(filter.limit_chars);
    let has_matches = ratings.iter().any(|rating| *rating >= 2.0);
    let mut selected_lengths = vec![0; lines.len()];
    let mut current_lines = 0;
    let mut current_chars = 0;

    for &index in &line_indices {
        if current_lines >= filter.limit_lines || current_chars >= byte_limit {
            break;
        }
        let dominated = remove_re
            .as_ref()
            .map_or(false, |re| re.is_match(lines[index]));
        if !dominated && ratings[index] > 0.0 {
            // Do not spend the remaining budget on a partial context line:
            // rendering it before a later match could push that match out again.
            // Keep clipping for the highest-priority line, including single-line output.
            if has_matches
                && ratings[index] < 2.0
                && current_chars
                    .saturating_add(lines[index].len())
                    .saturating_add(1)
                    > byte_limit / 2
            {
                continue;
            }
            approve[index] = true;
            current_lines += 1;
            let mut end = lines[index]
                .len()
                .min(byte_limit.saturating_sub(current_chars));
            while !lines[index].is_char_boundary(end) {
                end -= 1;
            }
            selected_lengths[index] = end;
            current_chars += end.saturating_add(1);
        }
    }

    let mut result = String::new();
    let mut skipped_lines = 0;
    let mut total_skipped = 0;
    for (i, &line) in lines.iter().enumerate() {
        if approve[i] {
            if skipped_lines > 0 {
                result.push_str(&format!("...{} lines skipped...\n", skipped_lines));
                total_skipped += skipped_lines;
                skipped_lines = 0;
            }
            result.push_str(&line[..selected_lengths[i]]);
            result.push('\n');
        } else {
            skipped_lines += 1;
        }
    }
    if skipped_lines > 0 {
        result.push_str(&format!("...{} lines skipped...\n", skipped_lines));
        total_skipped += skipped_lines;
    }
    if total_skipped > 0 {
        let filter_desc = if !filter.grep.is_empty() {
            format!(
                "grep: '{}'",
                filter.grep.chars().take(30).collect::<String>()
            )
        } else {
            format!("keep: {}", filter.valuable_top_or_bottom)
        };
        result.push_str(&format!(
            "⚠️ {} lines filtered (limit: {}, {}). 💡 Use output_limit:'all' or adjust output_filter\n",
            total_skipped, filter.limit_lines, filter_desc
        ));
    }
    if result.len() > byte_limit
        || lines
            .iter()
            .enumerate()
            .any(|(i, line)| approve[i] && selected_lengths[i] < line.len())
    {
        let marker = "\n[truncated]\n";
        let mut end = result.len().min(byte_limit.saturating_sub(marker.len()));
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
        if marker.len() <= byte_limit {
            result.push_str(marker);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_long_line_respects_character_and_token_budgets() {
        let output = "界".repeat(42_000);
        let mut filter = OutputFilter {
            limit_lines: 40,
            limit_chars: 100,
            limit_tokens: None,
            skip: false,
            ..Default::default()
        };
        let result = output_mini_postprocessing(&filter, &output);
        assert!(result.len() <= 100);
        assert!(result.ends_with("[truncated]\n"));
        filter.limit_chars = 1000;
        filter.limit_tokens = Some(50);
        let result = output_mini_postprocessing(&filter, &output);
        assert!(result.len() > 50);
        assert!(result.len() <= 200);
        assert!(result.ends_with("[truncated]\n"));
    }

    #[test]
    fn explicit_line_limit_retains_four_character_token_allowance() {
        let filter = parse_output_filter_args(
            &HashMap::from([("output_limit".into(), Value::from("40"))]),
            &OutputFilter::default(),
        );
        let output = "x".repeat(6000);
        assert_eq!(
            output_mini_postprocessing(&filter, &output),
            format!("{output}\n")
        );
    }

    #[test]
    fn test_cmdline_output_filter() {
        let output_to_filter = r#"line1
line2
line3
line4
line5
line6
"#;

        let result = output_mini_postprocessing(
            &OutputFilter {
                limit_lines: 2,
                limit_chars: 1000,
                valuable_top_or_bottom: "top".to_string(),
                grep: "".to_string(),
                grep_context_lines: 1,
                remove_from_output: "".to_string(),
                limit_tokens: Some(8000),
                skip: false,
            },
            output_to_filter,
        );
        assert!(result.contains("line1\nline2\n"));
        assert!(result.contains("4 lines"));
        assert!(result.contains("⚠️"));

        let result = output_mini_postprocessing(
            &OutputFilter {
                limit_lines: 2,
                limit_chars: 1000,
                valuable_top_or_bottom: "bottom".to_string(),
                grep: "".to_string(),
                grep_context_lines: 1,
                remove_from_output: "".to_string(),
                limit_tokens: Some(8000),
                skip: false,
            },
            output_to_filter,
        );
        assert!(result.contains("line5\nline6\n"));
        assert!(result.contains("4 lines"));

        let result = output_mini_postprocessing(
            &OutputFilter {
                limit_lines: 3,
                limit_chars: 1000,
                valuable_top_or_bottom: "".to_string(),
                grep: "line4".to_string(),
                grep_context_lines: 1,
                remove_from_output: "".to_string(),
                limit_tokens: Some(8000),
                skip: false,
            },
            output_to_filter,
        );
        assert!(result.contains("line3\nline4\nline5\n"));
        assert!(result.contains("⚠️"));

        let result = output_mini_postprocessing(
            &OutputFilter {
                limit_lines: 100,
                limit_chars: 8000,
                skip: false,
                valuable_top_or_bottom: "bottom".to_string(),
                ..Default::default()
            },
            output_to_filter,
        );
        assert_eq!(result, "line1\nline2\nline3\nline4\nline5\nline6\n");
    }
}
