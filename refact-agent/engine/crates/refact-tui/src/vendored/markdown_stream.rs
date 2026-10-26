// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::path::Path;

#[cfg(test)]
use ratatui::text::Line;

#[derive(Debug, Clone)]
pub struct MarkdownStreamCollector {
    buffer: String,
    committed_source_len: usize,
    #[cfg(test)]
    committed_line_count: usize,
    width: Option<usize>,
}

impl MarkdownStreamCollector {
    pub fn new(width: Option<usize>, cwd: &Path) -> Self {
        let _ = cwd;
        Self {
            buffer: String::new(),
            committed_source_len: 0,
            #[cfg(test)]
            committed_line_count: 0,
            width,
        }
    }

    pub fn set_width(&mut self, width: Option<usize>) {
        self.width = width;
    }

    pub fn width(&self) -> Option<usize> {
        self.width
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.committed_source_len = 0;
        #[cfg(test)]
        {
            self.committed_line_count = 0;
        }
    }

    pub fn push_delta(&mut self, delta: &str) {
        tracing::trace!("push_delta: {delta:?}");
        self.buffer.push_str(delta);
    }

    pub fn pending_source(&self) -> &str {
        &self.buffer[self.committed_source_len..]
    }

    pub fn commit_complete_source(&mut self) -> Option<String> {
        let commit_end = self.buffer.rfind('\n').map(|idx| idx + 1)?;
        if commit_end <= self.committed_source_len {
            return None;
        }

        let out = self.buffer[self.committed_source_len..commit_end].to_string();
        self.committed_source_len = commit_end;
        Some(out)
    }

    pub fn finalize_and_drain_source(&mut self) -> String {
        if self.committed_source_len >= self.buffer.len() {
            self.clear();
            return String::new();
        }

        let mut out = self.buffer[self.committed_source_len..].to_string();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        self.clear();
        out
    }

    #[cfg(test)]
    pub fn commit_complete_lines(&mut self) -> Vec<Line<'static>> {
        let Some(commit_end) = self.buffer.rfind('\n').map(|idx| idx + 1) else {
            return Vec::new();
        };
        if commit_end <= self.committed_source_len {
            return Vec::new();
        }

        let rendered = render_test_markdown(&self.buffer[..commit_end], self.width);
        let complete_line_count = complete_rendered_line_count(&rendered);
        if self.committed_line_count >= complete_line_count {
            return Vec::new();
        }

        let out = rendered[self.committed_line_count..complete_line_count].to_vec();
        self.committed_source_len = commit_end;
        self.committed_line_count = complete_line_count;
        out
    }

    #[cfg(test)]
    pub fn finalize_and_drain(&mut self) -> Vec<Line<'static>> {
        if self.buffer.is_empty() {
            self.clear();
            return Vec::new();
        }

        let mut source = self.buffer.clone();
        if !source.ends_with('\n') {
            source.push('\n');
        }
        let rendered = render_test_markdown(&source, self.width);
        let out = if self.committed_line_count >= rendered.len() {
            Vec::new()
        } else {
            rendered[self.committed_line_count..].to_vec()
        };
        self.clear();
        out
    }
}

#[cfg(test)]
fn render_test_markdown(source: &str, width: Option<usize>) -> Vec<Line<'static>> {
    crate::render::markdown::render_markdown_with_options(
        source,
        crate::render::markdown::RenderOptions {
            width,
            color_enabled: true,
        },
    )
}

#[cfg(test)]
fn complete_rendered_line_count(rendered: &[Line<'static>]) -> usize {
    rendered
        .last()
        .is_some_and(is_blank_line_spaces_only)
        .then_some(rendered.len().saturating_sub(1))
        .unwrap_or(rendered.len())
}

#[cfg(test)]
fn is_blank_line_spaces_only(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .all(|span| span.content.chars().all(|ch| ch == ' '))
}

#[cfg(test)]
fn test_cwd() -> std::path::PathBuf {
    std::env::temp_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::wrapping::line_to_plain;

    #[test]
    fn commits_only_complete_newline_boundaries() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(Some(80), &cwd);
        collector.push_delta("hello");
        assert_eq!(collector.commit_complete_source(), None);
        assert_eq!(collector.pending_source(), "hello");
        collector.push_delta(" world\nnext");
        assert_eq!(
            collector.commit_complete_source(),
            Some("hello world\n".to_string())
        );
        assert_eq!(collector.pending_source(), "next");
        collector.push_delta(" line\n");
        assert_eq!(
            collector.commit_complete_source(),
            Some("next line\n".to_string())
        );
        assert_eq!(collector.finalize_and_drain_source(), "");
        assert_eq!(collector.width(), Some(80));
    }

    #[test]
    fn finalize_newline_terminates_tail() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("tail");
        assert_eq!(collector.finalize_and_drain_source(), "tail\n");
        assert_eq!(collector.finalize_and_drain_source(), "");
    }

    #[test]
    fn collector_source_chunks_round_trip_into_table_rendering() {
        let cwd = test_cwd();
        let deltas = ["| A | B |\n", "|---|---|\n", "| 1 | 2 |\n"];
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        let mut raw_source = String::new();

        for delta in deltas {
            collector.push_delta(delta);
            if let Some(chunk) = collector.commit_complete_source() {
                raw_source.push_str(&chunk);
            }
        }
        raw_source.push_str(&collector.finalize_and_drain_source());

        let rendered = render_test_markdown(&raw_source, None);
        let rendered_strings = plain_strings(&rendered);
        assert_eq!(raw_source, deltas.join(""));
        assert!(rendered_strings.iter().any(|line| line.contains('━')));
        assert!(!rendered_strings
            .iter()
            .any(|line| line.trim() == "| A | B |"));
    }

    fn plain_strings(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }
}
