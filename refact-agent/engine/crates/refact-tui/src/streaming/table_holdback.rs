// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use crate::table_detect::{
    is_table_delimiter_line, is_table_header_line, parse_table_segments, strip_blockquote_prefix,
    FenceKind, FenceTracker,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableHoldbackState {
    None,
    PendingHeader { header_start: usize },
    Confirmed { table_start: usize },
}

#[derive(Debug, Clone, Copy)]
struct PreviousLineState {
    source_start: usize,
    fence_kind: FenceKind,
    is_header: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TableHoldbackScanner {
    source_offset: usize,
    previous_line: Option<PreviousLineState>,
    pending_header_start: Option<usize>,
    confirmed_table_start: Option<usize>,
    fence_tracker: FenceTracker,
}

impl Default for TableHoldbackScanner {
    fn default() -> Self {
        Self {
            source_offset: 0,
            previous_line: None,
            pending_header_start: None,
            confirmed_table_start: None,
            fence_tracker: FenceTracker::new(),
        }
    }
}

impl TableHoldbackScanner {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn replace_prefix(&mut self, source: &str) {
        self.reset();
        self.push_source_chunk(source);
    }

    pub(crate) fn state(&self) -> TableHoldbackState {
        if let Some(table_start) = self.confirmed_table_start {
            TableHoldbackState::Confirmed { table_start }
        } else if let Some(header_start) = self.pending_header_start {
            TableHoldbackState::PendingHeader { header_start }
        } else {
            TableHoldbackState::None
        }
    }

    pub(crate) fn source_offset(&self) -> usize {
        self.source_offset
    }

    pub(crate) fn push_source_chunk(&mut self, source_chunk: &str) {
        for source_line in source_chunk.split_inclusive('\n') {
            self.push_line(source_line);
        }
    }

    fn push_line(&mut self, source_line: &str) {
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        let source_start = self.source_offset;
        let fence_kind = self.fence_tracker.kind();
        let candidate_text = if fence_kind == FenceKind::Other {
            None
        } else {
            table_candidate_text(line)
        };
        let is_header = candidate_text.is_some_and(is_table_header_line);
        let is_delimiter = candidate_text.is_some_and(is_table_delimiter_line);
        let was_confirmed = self.confirmed_table_start.is_some();

        if self.confirmed_table_start.is_none() {
            if let Some(previous_line) = self.previous_line {
                if previous_line.fence_kind != FenceKind::Other
                    && fence_kind != FenceKind::Other
                    && previous_line.is_header
                    && is_delimiter
                {
                    self.confirmed_table_start = Some(previous_line.source_start);
                    self.pending_header_start = None;
                }
            }
        }

        if self.confirmed_table_start.is_none() && !line.trim().is_empty() {
            if fence_kind != FenceKind::Other && is_header {
                self.pending_header_start = Some(source_start);
            } else {
                self.pending_header_start = None;
            }
        }
        if was_confirmed && line.trim().is_empty() {
            self.confirmed_table_start = None;
            self.pending_header_start = None;
        }
        if self.confirmed_table_start.is_none() && line.trim().is_empty() {
            self.pending_header_start = None;
        }

        self.previous_line = Some(PreviousLineState {
            source_start,
            fence_kind,
            is_header,
        });
        self.fence_tracker.advance(line);
        self.source_offset = self.source_offset.saturating_add(source_line.len());
    }
}

fn table_candidate_text(line: &str) -> Option<&str> {
    let stripped = strip_blockquote_prefix(line).trim();
    parse_table_segments(stripped).map(|_| stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_confirmed_table() {
        let mut scanner = TableHoldbackScanner::default();
        scanner.push_source_chunk("intro\n| A | B |\n");
        assert_eq!(
            scanner.state(),
            TableHoldbackState::PendingHeader { header_start: 6 }
        );
        scanner.push_source_chunk("| --- | --- |\n");
        assert_eq!(
            scanner.state(),
            TableHoldbackState::Confirmed { table_start: 6 }
        );
    }

    #[test]
    fn ignores_tables_inside_code_fences() {
        let mut scanner = TableHoldbackScanner::default();
        scanner.push_source_chunk("```\n| A | B |\n| --- | --- |\n```\n");
        assert_eq!(scanner.state(), TableHoldbackState::None);
    }

    #[test]
    fn detects_tables_inside_markdown_fences() {
        let mut scanner = TableHoldbackScanner::default();
        scanner.push_source_chunk("```md\n| A | B |\n| --- | --- |\n");
        assert_eq!(
            scanner.state(),
            TableHoldbackState::Confirmed { table_start: 6 }
        );
    }
}
