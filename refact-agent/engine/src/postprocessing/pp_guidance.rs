pub fn truncation_guide(shown: usize, total: usize, limit_desc: &str, hint: &str) -> String {
    if shown >= total {
        return String::new();
    }
    let skipped = total.saturating_sub(shown);
    format!("⚠️ {} of {} shown ({} skipped, {}). 💡 {}", shown, total, skipped, limit_desc, hint)
}

pub fn lines_truncated_guide(skipped: usize, limit_desc: &str, hint: &str) -> String {
    if skipped == 0 {
        return String::new();
    }
    format!("⚠️ {} lines truncated ({}). 💡 {}", skipped, limit_desc, hint)
}

pub fn rows_truncated_guide(shown: usize, total: usize, max_rows: usize) -> String {
    if shown >= total {
        return String::new();
    }
    format!("⚠️ showing {} of {} rows (limit: {}). 💡 Add LIMIT/WHERE to query or paginate results", shown, total, max_rows)
}

pub fn cell_truncated_suffix(original_chars: usize, max_chars: usize) -> String {
    if original_chars <= max_chars {
        return String::new();
    }
    format!("…(+{}ch)", original_chars - max_chars)
}

pub fn timeout_guide(tool: &str, seconds: u64, hint: &str) -> String {
    format!("⚠️ {} timed out after {}s. 💡 {}", tool, seconds, hint)
}

pub fn not_found_guide(what: &str, path: &str, suggestions: &[&str]) -> String {
    if suggestions.is_empty() {
        format!("⚠️ {} '{}' not found. 💡 Use tree() to explore or search_pattern() to find", what, path)
    } else {
        format!("⚠️ {} '{}' not found. 💡 Try: {}", what, path, suggestions.join(", "))
    }
}

pub fn no_results_guide(tool: &str, query: &str, hints: &[&str]) -> String {
    let hint_text = if hints.is_empty() {
        "broaden scope to 'workspace' or adjust query".to_string()
    } else {
        hints.join("; ")
    };
    format!("⚠️ {} found no results for '{}'. 💡 {}", tool, query, hint_text)
}

pub fn scope_empty_guide(scope: &str) -> String {
    format!(
        "⚠️ No files found in scope '{}'. 💡 Use 'workspace' for all files, 'dir/' (with trailing slash) for directories, or check path exists",
        scope
    )
}

pub fn output_filtered_guide(skipped: usize, limit: usize, filter_desc: &str) -> String {
    if skipped == 0 {
        return String::new();
    }
    format!(
        "⚠️ {} lines filtered (limit: {}, {}). 💡 Use output_limit:'all' or adjust output_filter",
        skipped, limit, filter_desc
    )
}
