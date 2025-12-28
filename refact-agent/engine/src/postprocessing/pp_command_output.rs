use std::collections::HashMap;
use serde::Serialize;
use serde::Deserialize;
use serde_json::Value;
use regex::Regex;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OutputFilter {
    #[serde(default = "default_limit_lines")]
    pub limit_lines: usize,
    #[serde(default = "default_limit_chars")]
    pub limit_chars: usize,
    #[serde(default = "default_valuable_top_or_bottom")]
    pub valuable_top_or_bottom: String,
    #[serde(default = "default_grep")]
    pub grep: String,
    #[serde(default = "default_grep_context_lines")]
    pub grep_context_lines: usize,
    #[serde(default = "default_remove_from_output")]
    pub remove_from_output: String,
    #[serde(default = "default_limit_tokens")]
    pub limit_tokens: Option<usize>,
    #[serde(default)]
    pub skip: bool,
}

impl Default for OutputFilter {
    fn default() -> Self {
        OutputFilter {
            limit_lines: default_limit_lines(),
            limit_chars: default_limit_chars(),
            valuable_top_or_bottom: default_valuable_top_or_bottom(),
            grep: default_grep(),
            grep_context_lines: default_grep_context_lines(),
            remove_from_output: default_remove_from_output(),
            limit_tokens: default_limit_tokens(),
            skip: false,
        }
    }
}

impl OutputFilter {
    pub fn no_limits() -> Self {
        OutputFilter {
            limit_lines: usize::MAX,
            limit_chars: usize::MAX,
            limit_tokens: None,
            grep: String::new(),
            remove_from_output: String::new(),
            skip: true,
            ..Default::default()
        }
    }
}

fn default_limit_lines() -> usize { 50 }
fn default_limit_chars() -> usize { 8000 }
fn default_valuable_top_or_bottom() -> String { "top".to_string() }
fn default_grep() -> String { "(?i)(error|failed|exception|warning|fatal|panic|traceback)".to_string() }
fn default_grep_context_lines() -> usize { 3 }
fn default_remove_from_output() -> String { String::new() }
fn default_limit_tokens() -> Option<usize> { Some(8000) }

pub fn parse_output_filter_args(args: &HashMap<String, Value>, default: &OutputFilter) -> OutputFilter {
    let output_filter_pattern = args.get("output_filter")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let output_limit = args.get("output_limit")
        .and_then(|v| v.as_str().map(|s| s.to_string())
            .or_else(|| v.as_u64().map(|n| n.to_string())));

    if output_filter_pattern.is_none() && output_limit.is_none() {
        return default.clone();
    }

    let is_unlimited = output_limit.as_deref()
        .map(|s| s.eq_ignore_ascii_case("all") || s.eq_ignore_ascii_case("full"))
        .unwrap_or(false);

    let limit_lines = if is_unlimited {
        usize::MAX
    } else {
        output_limit.as_deref()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default.limit_lines)
    };

    OutputFilter {
        limit_lines,
        limit_chars: if is_unlimited { usize::MAX } else { limit_lines.saturating_mul(200) },
        valuable_top_or_bottom: default.valuable_top_or_bottom.clone(),
        grep: output_filter_pattern.unwrap_or_else(|| default.grep.clone()),
        grep_context_lines: default.grep_context_lines,
        remove_from_output: default.remove_from_output.clone(),
        limit_tokens: if is_unlimited { None } else { Some(limit_lines.saturating_mul(50)) },
        skip: false,
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
                    ratings[i] = 1.0;
                    for j in 1..=filter.grep_context_lines {
                        if i >= j { ratings[i - j] = 1.0; }
                        if i + j < lines.len() { ratings[i + j] = 1.0; }
                    }
                }
            }
        }
    }

    let mut line_indices: Vec<usize> = (0..lines.len()).collect();
    line_indices.sort_by(|&a, &b| ratings[b].partial_cmp(&ratings[a]).unwrap_or(std::cmp::Ordering::Equal));

    let remove_re = if !filter.remove_from_output.is_empty() {
        Regex::new(&filter.remove_from_output).ok()
    } else {
        None
    };

    let mut current_lines = 0;
    let mut current_chars = 0;

    for &index in &line_indices {
        if current_lines >= filter.limit_lines || current_chars >= filter.limit_chars {
            break;
        }
        let dominated = remove_re.as_ref().map_or(false, |re| re.is_match(lines[index]));
        if !dominated && ratings[index] > 0.0 {
            approve[index] = true;
            current_lines += 1;
            current_chars += lines[index].len();
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
            result.push_str(line);
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
            format!("grep: '{}'", &filter.grep[..filter.grep.len().min(30)])
        } else {
            format!("keep: {}", filter.valuable_top_or_bottom)
        };
        result.push_str(&format!(
            "⚠️ {} lines filtered (limit: {}, {}). 💡 Use output_limit:'all' or adjust output_filter\n",
            total_skipped, filter.limit_lines, filter_desc
        ));
    }
    result
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmdline_output_filter() {
        let output_to_filter = r#"line1
line2
line3
line4
line5
line6
"#;

        let result = output_mini_postprocessing(&OutputFilter {
            limit_lines: 2,
            limit_chars: 1000,
            valuable_top_or_bottom: "top".to_string(),
            grep: "".to_string(),
            grep_context_lines: 1,
            remove_from_output: "".to_string(),
            limit_tokens: Some(8000),
            skip: false,
        }, output_to_filter);
        assert!(result.contains("line1\nline2\n"));
        assert!(result.contains("4 lines"));
        assert!(result.contains("⚠️"));

        let result = output_mini_postprocessing(&OutputFilter {
            limit_lines: 2,
            limit_chars: 1000,
            valuable_top_or_bottom: "bottom".to_string(),
            grep: "".to_string(),
            grep_context_lines: 1,
            remove_from_output: "".to_string(),
            limit_tokens: Some(8000),
            skip: false,
        }, output_to_filter);
        assert!(result.contains("line5\nline6\n"));
        assert!(result.contains("4 lines"));

        let result = output_mini_postprocessing(&OutputFilter {
            limit_lines: 3,
            limit_chars: 1000,
            valuable_top_or_bottom: "".to_string(),
            grep: "line4".to_string(),
            grep_context_lines: 1,
            remove_from_output: "".to_string(),
            limit_tokens: Some(8000),
            skip: false,
        }, output_to_filter);
        assert!(result.contains("line3\nline4\nline5\n"));
        assert!(result.contains("⚠️"));

        let result = output_mini_postprocessing(&OutputFilter {
            limit_lines: 100,
            limit_chars: 8000,
            skip: false,
            valuable_top_or_bottom: "bottom".to_string(),
            ..Default::default()
        }, output_to_filter);
        assert_eq!(result, "line1\nline2\nline3\nline4\nline5\nline6\n");
    }
}

