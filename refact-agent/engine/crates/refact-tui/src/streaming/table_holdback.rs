// Adapted from openai/codex codex-rs/tui, Apache-2.0.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableHoldbackState {
    None,
    PendingHeader { header_start: usize },
    Confirmed { table_start: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FenceKind {
    Outside,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct PreviousLineState {
    source_start: usize,
    fence_kind: FenceKind,
    is_header: bool,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct TableHoldbackScanner {
    source_offset: usize,
    previous_line: Option<PreviousLineState>,
    pending_header_start: Option<usize>,
    confirmed_table_start: Option<usize>,
    in_fence: bool,
    fence_marker: Option<char>,
    fence_len: usize,
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
        let fence_kind = if self.in_fence {
            FenceKind::Other
        } else {
            FenceKind::Outside
        };
        let candidate_text = (fence_kind == FenceKind::Outside)
            .then(|| strip_blockquote_prefix(line).trim())
            .filter(|line| line.contains('|'));
        let is_header = candidate_text.is_some_and(is_table_header_line);
        let is_delimiter = candidate_text.is_some_and(is_table_delimiter_line);
        let was_confirmed = self.confirmed_table_start.is_some();

        if self.confirmed_table_start.is_none() {
            if let Some(previous_line) = self.previous_line {
                if previous_line.fence_kind == FenceKind::Outside
                    && fence_kind == FenceKind::Outside
                    && previous_line.is_header
                    && is_delimiter
                {
                    self.confirmed_table_start = Some(previous_line.source_start);
                    self.pending_header_start = None;
                }
            }
        }

        if self.confirmed_table_start.is_none() && !line.trim().is_empty() {
            if fence_kind == FenceKind::Outside && is_header {
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
        self.advance_fence(line);
        self.source_offset = self.source_offset.saturating_add(source_line.len());
    }

    fn advance_fence(&mut self, line: &str) {
        let trimmed = line.trim_start();
        let Some(marker) = trimmed.chars().next().filter(|ch| *ch == '`' || *ch == '~') else {
            return;
        };
        let len = trimmed.chars().take_while(|ch| *ch == marker).count();
        if len < 3 {
            return;
        }
        if self.in_fence {
            if self.fence_marker == Some(marker) && len >= self.fence_len {
                self.in_fence = false;
                self.fence_marker = None;
                self.fence_len = 0;
            }
        } else {
            self.in_fence = true;
            self.fence_marker = Some(marker);
            self.fence_len = len;
        }
    }
}

fn strip_blockquote_prefix(mut line: &str) -> &str {
    loop {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('>') {
            line = rest.strip_prefix(' ').unwrap_or(rest);
        } else {
            return trimmed;
        }
    }
}

fn is_table_header_line(line: &str) -> bool {
    table_segments(line).is_some_and(|segments| {
        segments.len() >= 2 && segments.iter().any(|segment| !segment.trim().is_empty())
    })
}

fn is_table_delimiter_line(line: &str) -> bool {
    table_segments(line).is_some_and(|segments| {
        segments.len() >= 2
            && segments.iter().all(|segment| {
                let segment = segment.trim();
                !segment.is_empty()
                    && segment
                        .chars()
                        .all(|ch| ch == '-' || ch == ':' || ch.is_ascii_whitespace())
                    && segment.chars().filter(|ch| *ch == '-').count() >= 3
            })
    })
}

fn table_segments(line: &str) -> Option<Vec<&str>> {
    let mut line = line.trim();
    if !line.contains('|') {
        return None;
    }
    if let Some(rest) = line.strip_prefix('|') {
        line = rest;
    }
    if let Some(rest) = line.strip_suffix('|') {
        line = rest;
    }
    let segments = line.split('|').collect::<Vec<_>>();
    (segments.len() >= 2).then_some(segments)
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
}
