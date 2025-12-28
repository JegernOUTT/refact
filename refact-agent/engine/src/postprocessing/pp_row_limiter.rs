pub struct RowLimiter {
    pub max_rows: usize,
    #[allow(dead_code)]
    pub max_cell_chars: usize,
}

impl Default for RowLimiter {
    fn default() -> Self {
        Self {
            max_rows: 100,
            max_cell_chars: 200,
        }
    }
}

impl RowLimiter {
    pub fn new(max_rows: usize, max_cell_chars: usize) -> Self {
        Self { max_rows, max_cell_chars }
    }

    pub fn limit_text_rows(&self, text: &str) -> String {
        let mut lines_iter = text.lines();
        let kept: Vec<&str> = lines_iter.by_ref().take(self.max_rows).collect();
        let remaining = lines_iter.count();

        if remaining == 0 {
            return text.to_string();
        }
        let total = kept.len() + remaining;
        format!(
            "{}\n⚠️ showing {} of {} rows (limit: {}). 💡 Add LIMIT/WHERE to query",
            kept.join("\n"), kept.len(), total, self.max_rows
        )
    }

    #[allow(dead_code)]
    pub fn truncate_cell(&self, cell: &str) -> String {
        let char_count = cell.chars().count();
        if char_count <= self.max_cell_chars {
            cell.to_string()
        } else {
            let truncated: String = cell.chars().take(self.max_cell_chars).collect();
            format!("{}…(+{}ch)", truncated, char_count - self.max_cell_chars)
        }
    }

    #[allow(dead_code)]
    pub fn format_table(&self, headers: &[String], rows: Vec<Vec<String>>, total_rows: usize) -> String {
        let mut result = String::new();

        let truncated_headers: Vec<String> = headers.iter()
            .map(|h| self.truncate_cell(h))
            .collect();
        result.push_str(&truncated_headers.join(" | "));
        result.push('\n');
        result.push_str(&"-".repeat(truncated_headers.join(" | ").len()));
        result.push('\n');

        for row in rows.iter().take(self.max_rows) {
            let truncated_row: Vec<String> = row.iter()
                .map(|c| self.truncate_cell(c))
                .collect();
            result.push_str(&truncated_row.join(" | "));
            result.push('\n');
        }

        if total_rows > self.max_rows {
            result.push_str(&format!(
                "⚠️ showing {} of {} rows (limit: {}). 💡 Add LIMIT/WHERE to query\n",
                self.max_rows, total_rows, self.max_rows
            ));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limit_text_rows() {
        let limiter = RowLimiter::new(3, 50);
        let text = "line1\nline2\nline3\nline4\nline5";
        let result = limiter.limit_text_rows(text);
        assert!(result.contains("line1"));
        assert!(result.contains("line3"));
        assert!(!result.contains("line4"));
        assert!(result.contains("showing 3 of 5 rows"));
    }

    #[test]
    fn test_truncate_cell() {
        let limiter = RowLimiter::new(100, 10);
        assert_eq!(limiter.truncate_cell("short"), "short");
        assert_eq!(limiter.truncate_cell("this is a very long cell"), "this is a …(+14ch)");
    }

    #[test]
    fn test_format_table() {
        let limiter = RowLimiter::new(2, 50);
        let headers = vec!["id".into(), "name".into()];
        let rows = vec![
            vec!["1".into(), "Alice".into()],
            vec!["2".into(), "Bob".into()],
            vec!["3".into(), "Charlie".into()],
        ];
        let result = limiter.format_table(&headers, rows, 3);
        assert!(result.contains("Alice"));
        assert!(result.contains("Bob"));
        assert!(!result.contains("Charlie"));
        assert!(result.contains("showing 2 of 3 rows"));
    }
}
