// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::path::Path;

#[derive(Debug, Clone)]
pub struct MarkdownStreamCollector {
    buffer: String,
    committed_source_len: usize,
    width: Option<usize>,
}

impl MarkdownStreamCollector {
    pub fn new(width: Option<usize>, cwd: &Path) -> Self {
        let _ = cwd;
        Self {
            buffer: String::new(),
            committed_source_len: 0,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_only_complete_newline_boundaries() {
        let mut collector = MarkdownStreamCollector::new(Some(80), std::path::Path::new("."));
        collector.push_delta("hello");
        assert_eq!(collector.commit_complete_source(), None);
        collector.push_delta(" world\nnext");
        assert_eq!(
            collector.commit_complete_source(),
            Some("hello world\n".to_string())
        );
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
        let mut collector = MarkdownStreamCollector::new(None, std::path::Path::new("."));
        collector.push_delta("tail");
        assert_eq!(collector.finalize_and_drain_source(), "tail\n");
        assert_eq!(collector.finalize_and_drain_source(), "");
    }
}
