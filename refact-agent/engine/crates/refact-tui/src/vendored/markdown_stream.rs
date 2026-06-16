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
pub(crate) fn simulate_stream_markdown_for_tests(
    deltas: &[&str],
    finalize: bool,
) -> Vec<Line<'static>> {
    let cwd = test_cwd();
    let mut collector = MarkdownStreamCollector::new(None, &cwd);
    let mut out = Vec::new();
    for delta in deltas {
        collector.push_delta(delta);
        if delta.contains('\n') {
            out.extend(collector.commit_complete_lines());
        }
    }
    if finalize {
        out.extend(collector.finalize_and_drain());
    }
    out
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
    fn no_commit_until_newline() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("Hello, world");
        assert!(collector.commit_complete_lines().is_empty());
        collector.push_delta("!\n");
        let out = collector.commit_complete_lines();
        assert_eq!(plain_strings(&out), vec!["Hello, world!".to_string()]);
    }

    #[test]
    fn finalize_commits_partial_line() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("Line without newline");
        assert_eq!(
            plain_strings(&collector.finalize_and_drain()),
            vec!["Line without newline".to_string()]
        );
    }

    #[test]
    fn heading_starts_on_new_line_when_following_paragraph() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("Hello.\n");
        assert_eq!(
            plain_strings(&collector.commit_complete_lines()),
            vec!["Hello.".to_string()]
        );

        collector.push_delta("## Heading\n");
        assert_eq!(
            plain_strings(&collector.commit_complete_lines()),
            vec!["".to_string(), "## Heading".to_string()]
        );
    }

    #[test]
    fn heading_not_inlined_when_split_across_chunks() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("Sounds good!");
        assert!(collector.commit_complete_lines().is_empty());

        collector.push_delta("\n## Adding Bird subcommand");
        assert_eq!(
            plain_strings(&collector.commit_complete_lines()),
            vec!["Sounds good!".to_string()]
        );

        collector.push_delta("\n");
        assert_eq!(
            plain_strings(&collector.commit_complete_lines()),
            vec!["".to_string(), "## Adding Bird subcommand".to_string()]
        );
    }

    #[test]
    fn table_header_commits_without_collector_holdback() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("| A | B |\n");
        assert_eq!(
            plain_strings(&collector.commit_complete_lines()),
            vec!["| A | B |".to_string()]
        );

        collector.push_delta("| --- | --- |\n");
        assert!(!collector.commit_complete_lines().is_empty());
        collector.push_delta("| 1 | 2 |\n");
        assert!(!collector.commit_complete_lines().is_empty());
    }

    #[test]
    fn pipe_text_without_table_prefix_is_not_delayed() {
        let cwd = test_cwd();
        let mut collector = MarkdownStreamCollector::new(None, &cwd);
        collector.push_delta("Escaped pipe in text: a | b | c\n");
        assert_eq!(
            plain_strings(&collector.commit_complete_lines()),
            vec!["Escaped pipe in text: a | b | c".to_string()]
        );
    }

    #[test]
    fn blockquote_and_ordered_marker_styles_survive_streaming() {
        let out = simulate_stream_markdown_for_tests(
            &["> quoted\n\n", "1. ordered\n", "   1. nested\n"],
            true,
        );
        let quote = out
            .iter()
            .find(|line| line_to_plain(line) == "> quoted")
            .unwrap();
        assert_eq!(quote.style.fg, Some(ratatui::style::Color::Green));

        let nested = out
            .iter()
            .find(|line| line_to_plain(line).contains("nested"))
            .unwrap();
        assert!(nested
            .spans
            .iter()
            .any(|span| span.style.fg == Some(ratatui::style::Color::LightBlue)));
    }

    #[test]
    fn lists_and_fences_commit_without_duplication() {
        assert_streamed_equals_full(&["- a\n- ", "b\n- c\n"]);
        assert_streamed_equals_full(&["```", "\nco", "de 1\ncode 2\n", "```\n"]);
    }

    #[test]
    fn utf8_boundary_safety_and_wide_chars() {
        let input = "🙂🙂🙂\n汉字漢字\nA\u{0003}0\u{0304}\n";
        let deltas = [
            "🙂",
            "🙂",
            "🙂\n汉",
            "字漢",
            "字\nA",
            "\u{0003}",
            "0",
            "\u{0304}",
            "\n",
        ];
        let streamed = simulate_stream_markdown_for_tests(&deltas, true);
        let full = render_test_markdown(input, None);
        assert_eq!(plain_strings(&streamed), plain_strings(&full));
    }

    #[test]
    fn loose_list_with_split_dashes_matches_full_render() {
        assert_streamed_equals_full(&["- item.\n\n", "-"]);
    }

    #[test]
    fn loose_vs_tight_list_items_streaming_matches_full() {
        let deltas = [
            "\n\n",
            "Loose",
            " vs",
            ".",
            " tight",
            " list",
            " items",
            ":\n",
            "1",
            ".",
            " Tight",
            " item",
            "\n",
            "2",
            ".",
            " Another",
            " tight",
            " item",
            "\n\n",
            "1",
            ".",
            " Loose",
            " item",
            " with",
            " its",
            " own",
            " paragraph",
            ".\n\n",
            "  ",
            " This",
            " paragraph",
            " belongs",
            " to",
            " the",
            " same",
            " list",
            " item",
            ".\n\n",
            "2",
            ".",
            " Second",
            " loose",
            " item",
            " with",
            " a",
            " nested",
            " list",
            " after",
            " a",
            " blank",
            " line",
            ".\n\n",
            "  ",
            " -",
            " Nested",
            " bullet",
            " under",
            " a",
            " loose",
            " item",
            "\n",
            "  ",
            " -",
            " Another",
            " nested",
            " bullet",
            "\n\n",
        ];
        assert_streamed_equals_full(&deltas);
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

    fn assert_streamed_equals_full(deltas: &[&str]) {
        let streamed = simulate_stream_markdown_for_tests(deltas, true);
        let full_source = deltas.join("");
        let full = render_test_markdown(&full_source, None);
        assert_eq!(plain_strings(&streamed), plain_strings(&full));
    }

    fn plain_strings(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }
}
