use std::collections::{HashMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::vendored::terminal_hyperlinks::{
    hyperlinks_enabled_from_env, mark_buffer_hyperlinks, prefix_hyperlink_lines, visible_lines,
    HyperlinkLine,
};
use ratatui::Terminal;

use crate::app::TranscriptItem;

pub mod cells;

const MAX_INSERTION_LINES: usize = 2048;
const MAX_CACHE_ENTRIES: usize = 256;
const HISTORY_CELL_GUTTER: u16 = 2;
pub const RESIZE_REFLOW_PENDING_CELL_CAP: usize = 1_000;
pub const TRANSCRIPT_REFLOW_DEBOUNCE: Duration = Duration::from_millis(75);

const VSCODE_RESIZE_REFLOW_MAX_ROWS: usize = 1_000;
const WEZTERM_RESIZE_REFLOW_MAX_ROWS: usize = 3_500;
const ALACRITTY_RESIZE_REFLOW_MAX_ROWS: usize = 10_000;
const FALLBACK_RESIZE_REFLOW_MAX_ROWS: usize = 1_000;

#[derive(Debug, Clone)]
struct HistoryEntry {
    id: u64,
    cell: Box<dyn cells::HistoryCell>,
}

struct ReflowEntryDisplay {
    id: u64,
    lines: Vec<HyperlinkLine>,
    is_stream_continuation: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ResizeReflowState {
    last_observed_width: Option<u16>,
    last_reflow_width: Option<u16>,
    pending_reflow_width: Option<u16>,
    pending_until: Option<Instant>,
    ran_during_stream: bool,
    resize_requested_during_stream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeWidthChange {
    pub changed: bool,
    pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryInsertion {
    pub cell_ids: Vec<u64>,
    pub lines: Vec<HyperlinkLine>,
}

impl HistoryInsertion {
    pub fn height(&self) -> u16 {
        self.lines.len().min(u16::MAX as usize) as u16
    }
}

impl ResizeReflowState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn note_width(&mut self, width: u16) -> ResizeWidthChange {
        let previous_width = self.last_observed_width.replace(width);
        if previous_width.is_none() {
            self.last_reflow_width = Some(width);
        }
        ResizeWidthChange {
            changed: previous_width.is_some_and(|previous| previous != width),
            initialized: previous_width.is_none(),
        }
    }

    pub fn reflow_needed_for_width(&self, width: u16) -> bool {
        self.last_reflow_width != Some(width) && self.pending_reflow_width != Some(width)
    }

    pub fn schedule_debounced(&mut self, target_width: Option<u16>) {
        let now = Instant::now();
        if let Some(target_width) = target_width {
            self.pending_reflow_width = Some(target_width);
        }
        self.pending_until = Some(now + TRANSCRIPT_REFLOW_DEBOUNCE);
    }

    pub fn schedule_immediate(&mut self) {
        self.pending_reflow_width = None;
        self.pending_until = Some(Instant::now());
    }

    #[cfg(test)]
    pub fn set_due_for_test(&mut self) {
        self.pending_until = Some(Instant::now() - Duration::from_millis(1));
    }

    pub fn pending_is_due(&self, now: Instant) -> bool {
        self.pending_until.is_some_and(|deadline| now >= deadline)
    }

    pub fn pending_until(&self) -> Option<Instant> {
        self.pending_until
    }

    pub fn has_pending_reflow(&self) -> bool {
        self.pending_until.is_some()
    }

    pub fn clear_pending_reflow(&mut self) {
        self.pending_until = None;
        self.pending_reflow_width = None;
    }

    pub fn mark_reflowed_width(&mut self, width: u16) -> bool {
        self.last_reflow_width.replace(width) != Some(width)
    }

    pub fn mark_ran_during_stream(&mut self) {
        self.ran_during_stream = true;
    }

    pub fn mark_resize_requested_during_stream(&mut self) {
        self.resize_requested_during_stream = true;
    }

    pub fn take_stream_finish_reflow_needed(&mut self) -> bool {
        let needed = self.ran_during_stream || self.resize_requested_during_stream;
        self.ran_during_stream = false;
        self.resize_requested_during_stream = false;
        needed
    }

    pub fn clear_stream_flags(&mut self) {
        self.ran_during_stream = false;
        self.resize_requested_during_stream = false;
    }
}

pub fn resize_reflow_row_cap_from_env() -> usize {
    resize_reflow_row_cap_for_values(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
        std::env::var_os("WEZTERM_EXECUTABLE").is_some(),
        std::env::var_os("ALACRITTY_SOCKET").is_some()
            || std::env::var_os("ALACRITTY_LOG").is_some()
            || std::env::var_os("ALACRITTY_WINDOW_ID").is_some(),
    )
}

fn resize_reflow_row_cap_for_values(
    term_program: Option<&str>,
    term: Option<&str>,
    wezterm_env: bool,
    alacritty_env: bool,
) -> usize {
    let term_lower = term.map(str::to_ascii_lowercase);
    if term_program.is_some_and(|value| value.eq_ignore_ascii_case("vscode")) {
        return VSCODE_RESIZE_REFLOW_MAX_ROWS;
    }
    if term_program.is_some_and(|value| value.eq_ignore_ascii_case("WezTerm"))
        || wezterm_env
        || term_lower
            .as_deref()
            .is_some_and(|value| value.contains("wezterm"))
    {
        return WEZTERM_RESIZE_REFLOW_MAX_ROWS;
    }
    if term_program.is_some_and(|value| value.eq_ignore_ascii_case("Alacritty"))
        || alacritty_env
        || term_lower
            .as_deref()
            .is_some_and(|value| value.contains("alacritty"))
    {
        return ALACRITTY_RESIZE_REFLOW_MAX_ROWS;
    }
    if term_lower
        .as_deref()
        .is_some_and(|value| value.contains("vscode"))
    {
        return VSCODE_RESIZE_REFLOW_MAX_ROWS;
    }
    FALLBACK_RESIZE_REFLOW_MAX_ROWS
}

#[derive(Debug, Default, Clone)]
pub struct HistoryBuffer {
    next_id: u64,
    history: VecDeque<HistoryEntry>,
    pending: VecDeque<HistoryEntry>,
    cache: HashMap<(u64, u16, u64), Vec<HyperlinkLine>>,
    render_count: usize,
    inserted_cell_count: usize,
    emitted_history_lines: bool,
}

impl HistoryBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_pending(&mut self) {
        self.history.clear();
        self.pending.clear();
        self.cache.clear();
        self.emitted_history_lines = false;
    }

    pub fn enqueue(&mut self, item: TranscriptItem) -> u64 {
        self.enqueue_cell(cells::cell_from_transcript_item(&item, false))
    }

    pub fn enqueue_cell(&mut self, cell: Box<dyn cells::HistoryCell>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let entry = HistoryEntry { id, cell };
        self.history.push_back(entry.clone());
        self.pending.push_back(entry);
        id
    }

    pub fn drain_pending(&mut self, width: u16) -> Vec<HistoryInsertion> {
        let insertions = self.pending_insertions(width);
        self.inserted_cell_count += insertions
            .iter()
            .map(|insertion| insertion.cell_ids.len())
            .sum::<usize>();
        if insertions
            .iter()
            .any(|insertion| !insertion.lines.is_empty())
        {
            self.emitted_history_lines = true;
        }
        for insertion in &insertions {
            self.evict_cache_entries(&insertion.cell_ids);
        }
        self.pending.clear();
        insertions
    }

    pub fn drain_pending_capped(&mut self, width: u16, max_cells: usize) -> Vec<HistoryInsertion> {
        let insertions = self.pending_insertions_capped(width, max_cells);
        let drained = insertions
            .iter()
            .map(|insertion| insertion.cell_ids.len())
            .sum::<usize>();
        self.inserted_cell_count += drained;
        if insertions
            .iter()
            .any(|insertion| !insertion.lines.is_empty())
        {
            self.emitted_history_lines = true;
        }
        self.pending.drain(..drained);
        insertions
    }

    pub fn pending_insertions(&mut self, width: u16) -> Vec<HistoryInsertion> {
        self.pending_insertions_capped(width, self.pending.len())
    }

    pub fn pending_insertions_capped(
        &mut self,
        width: u16,
        max_cells: usize,
    ) -> Vec<HistoryInsertion> {
        let mut insertions = Vec::new();
        let mut current_ids = Vec::new();
        let mut current_lines = Vec::new();
        let mut emitted_history_lines = self.emitted_history_lines;
        let entries = self
            .pending
            .iter()
            .take(max_cells)
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            let mut lines = self.render_entry(&entry, width);
            if !lines.is_empty() && !entry.cell.is_stream_continuation() {
                if emitted_history_lines || !current_lines.is_empty() {
                    lines.insert(0, HyperlinkLine::new(Line::default()));
                } else {
                    emitted_history_lines = true;
                }
            }
            if !current_lines.is_empty() && current_lines.len() + lines.len() > MAX_INSERTION_LINES
            {
                insertions.push(HistoryInsertion {
                    cell_ids: std::mem::take(&mut current_ids),
                    lines: std::mem::take(&mut current_lines),
                });
            }
            current_ids.push(entry.id);
            current_lines.extend(lines);
        }
        if !current_lines.is_empty() {
            insertions.push(HistoryInsertion {
                cell_ids: current_ids,
                lines: current_lines,
            });
        }
        insertions
    }

    pub fn reflow_insertions(&mut self, width: u16, max_rows: usize) -> Vec<HistoryInsertion> {
        if self.history.is_empty() || max_rows == 0 {
            return Vec::new();
        }

        let mut displays = VecDeque::new();
        let mut rendered_rows = 0usize;
        let mut start = self.history.len();

        while start > 0 {
            start -= 1;
            let entry = self.history[start].clone();
            let lines = self.render_entry(&entry, width);
            rendered_rows += lines.len();
            displays.push_front(ReflowEntryDisplay {
                id: entry.id,
                lines,
                is_stream_continuation: entry.cell.is_stream_continuation(),
            });
            if rendered_rows > max_rows {
                break;
            }
        }

        while start > 0
            && displays
                .front()
                .is_some_and(|display| display.is_stream_continuation)
        {
            start -= 1;
            let entry = self.history[start].clone();
            displays.push_front(ReflowEntryDisplay {
                id: entry.id,
                lines: self.render_entry(&entry, width),
                is_stream_continuation: entry.cell.is_stream_continuation(),
            });
        }

        let mut cell_ids = Vec::new();
        let mut lines = Vec::new();
        let mut emitted_history_lines = false;
        for display in displays {
            cell_ids.push(display.id);
            if !display.lines.is_empty() && !display.is_stream_continuation {
                if emitted_history_lines {
                    lines.push(HyperlinkLine::new(Line::default()));
                } else {
                    emitted_history_lines = true;
                }
            }
            lines.extend(display.lines);
        }

        if lines.len() > max_rows {
            let trimmed_line_count = lines.len() - max_rows;
            lines = lines.split_off(trimmed_line_count);
        }

        let drained = self.pending.len();
        self.inserted_cell_count += drained;
        self.pending.clear();
        self.emitted_history_lines = !lines.is_empty();
        self.cache.clear();

        if lines.is_empty() {
            Vec::new()
        } else {
            vec![HistoryInsertion { cell_ids, lines }]
        }
    }

    pub fn pending_cell_count(&self) -> usize {
        self.pending.len()
    }

    pub fn source_cell_count(&self) -> usize {
        self.history.len()
    }

    pub fn render_count(&self) -> usize {
        self.render_count
    }

    pub fn inserted_cell_count(&self) -> usize {
        self.inserted_cell_count
    }

    pub fn cache_entry_count(&self) -> usize {
        self.cache.len()
    }

    fn render_entry(&mut self, entry: &HistoryEntry, width: u16) -> Vec<HyperlinkLine> {
        let key = (entry.id, width, entry.cell.revision());
        if let Some(lines) = self.cache.get(&key) {
            return lines.clone();
        }
        let content_width = width.saturating_sub(HISTORY_CELL_GUTTER).max(1) as usize;
        let lines = prefix_hyperlink_lines(
            entry.cell.display_hyperlink_lines(content_width),
            Span::raw(" ".repeat(HISTORY_CELL_GUTTER as usize)),
            Span::raw(" ".repeat(HISTORY_CELL_GUTTER as usize)),
        );
        self.cache.insert(key, lines.clone());
        self.enforce_cache_bound();
        self.render_count += 1;
        lines
    }

    fn evict_cache_entries(&mut self, cell_ids: &[u64]) {
        self.cache
            .retain(|(id, _, _), _| !cell_ids.iter().any(|cell_id| cell_id == id));
    }

    fn enforce_cache_bound(&mut self) {
        if self.cache.len() <= MAX_CACHE_ENTRIES {
            return;
        }
        let mut keys = self.cache.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        let remove_count = self.cache.len().saturating_sub(MAX_CACHE_ENTRIES);
        for key in keys.into_iter().take(remove_count) {
            self.cache.remove(&key);
        }
    }

    #[cfg(test)]
    fn replace_pending_cell(&mut self, id: u64, cell: Box<dyn cells::HistoryCell>) {
        if let Some(entry) = self.pending.iter_mut().find(|entry| entry.id == id) {
            entry.cell = cell.clone();
        }
        if let Some(entry) = self.history.iter_mut().find(|entry| entry.id == id) {
            entry.cell = cell;
        }
    }
}

pub fn insert_history<B: Backend>(
    terminal: &mut Terminal<B>,
    insertion: HistoryInsertion,
) -> io::Result<()> {
    let height = insertion.height();
    if height == 0 {
        return Ok(());
    }
    let lines = insertion.lines;
    let enabled = hyperlinks_enabled_from_env();
    terminal.insert_before(height, move |buffer| {
        let visible = visible_lines(lines.clone());
        Paragraph::new(visible).render(buffer.area, buffer);
        mark_buffer_hyperlinks(buffer, buffer.area, &lines, enabled);
    })
}

pub fn render_transcript_item_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<Line<'static>> {
    cells::render_transcript_item_lines(item, width, selected)
}

pub fn render_transcript_item_hyperlink_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<HyperlinkLine> {
    cells::render_transcript_item_hyperlink_lines(item, width, selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::NoticeCell;
    use crate::render::wrapping::line_to_plain;
    use ratatui::backend::TestBackend;
    use ratatui::{TerminalOptions, Viewport};

    #[derive(Debug, Clone)]
    struct FixedCell {
        text: &'static str,
        continuation: bool,
    }

    impl FixedCell {
        fn new(text: &'static str, continuation: bool) -> Self {
            Self { text, continuation }
        }
    }

    impl cells::HistoryCell for FixedCell {
        fn kind(&self) -> cells::HistoryCellKind {
            cells::HistoryCellKind::Info
        }

        fn render(&self, _width: usize) -> Vec<Line<'static>> {
            vec![Line::from(self.text)]
        }

        fn is_stream_continuation(&self) -> bool {
            self.continuation
        }

        fn revision(&self) -> u64 {
            self.text.len() as u64 + u64::from(self.continuation)
        }
    }

    fn scrollback_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .scrollback()
            .content()
            .iter()
            .map(|cell| crate::vendored::terminal_hyperlinks::strip_osc8(cell.symbol()))
            .collect::<String>()
    }

    #[test]
    fn pending_cells_render_once_and_insert_once() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Notice("one".to_string()));
        history.enqueue(TranscriptItem::Assistant("two".to_string()));

        let preview = history.pending_insertions(40);
        assert_eq!(preview.len(), 1);
        assert_eq!(history.render_count(), 2);
        let preview_again = history.pending_insertions(40);
        assert_eq!(preview_again, preview);
        assert_eq!(history.render_count(), 2);

        let insertions = history.drain_pending(40);
        assert_eq!(history.pending_cell_count(), 0);
        assert_eq!(history.inserted_cell_count(), 2);
        assert!(history.drain_pending(40).is_empty());
        assert_eq!(history.inserted_cell_count(), 2);

        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(2),
            },
        )
        .unwrap();
        for insertion in insertions {
            insert_history(&mut terminal, insertion).unwrap();
        }
        let text = scrollback_text(&terminal);
        assert!(!text.is_empty());
    }

    #[test]
    fn pending_insertions_add_gutter_and_codex_spacers() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Notice("one".to_string()));
        history.enqueue(TranscriptItem::Assistant("two".to_string()));

        let preview = history.pending_insertions(40);
        let lines = preview[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(lines, vec!["  • one", "  ", "", "  • two"]);

        history.drain_pending(40);
        history.enqueue(TranscriptItem::Notice("three".to_string()));
        let next = history.pending_insertions(40);
        let lines = next[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(lines, vec!["", "  • three", "  "]);
    }

    #[test]
    fn stream_continuation_cells_do_not_get_leading_spacers() {
        let mut history = HistoryBuffer::new();
        history.enqueue_cell(Box::new(FixedCell::new("head", false)));
        history.enqueue_cell(Box::new(FixedCell::new("tail", true)));
        history.enqueue_cell(Box::new(FixedCell::new("next", false)));

        let insertions = history.pending_insertions(40);
        let lines = insertions[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(lines, vec!["  head", "  tail", "", "  next"]);
    }

    #[test]
    fn markdown_link_inserted_into_scrollback_carries_osc8_when_enabled() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Assistant(
            "Read [docs](https://example.com/docs) now".to_string(),
        ));
        let insertions = history.drain_pending(80);
        assert_eq!(
            insertions[0].lines[0].hyperlinks[0].destination,
            "https://example.com/docs"
        );
        assert!(insertions[0].lines[0]
            .hyperlinks
            .iter()
            .all(|link| link.columns.start >= HISTORY_CELL_GUTTER as usize));

        let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 80, 3));
        let area = buffer.area;
        let lines = insertions[0].lines.clone();
        Paragraph::new(visible_lines(lines.clone())).render(area, &mut buffer);
        mark_buffer_hyperlinks(&mut buffer, area, &lines, true);
        let raw = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(raw.contains("\x1b]8;;https://example.com/docs\x1b\\"));
        assert_eq!(
            crate::vendored::terminal_hyperlinks::strip_osc8(&raw)
                .contains("Read docs (https://example.com/docs) now"),
            true
        );
    }

    #[test]
    fn pending_content_rewraps_by_width_before_insert() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Assistant(
            "alpha beta gamma delta epsilon".to_string(),
        ));

        let narrow = history.pending_insertions(12);
        assert_eq!(history.render_count(), 1);
        let wide = history.pending_insertions(40);
        assert_eq!(history.render_count(), 2);
        assert_ne!(narrow[0].lines, wide[0].lines);
        let wide_again = history.drain_pending(40);
        assert_eq!(history.render_count(), 2);
        assert_eq!(wide_again, wide);
    }

    #[test]
    fn resize_rewrap_caps_pending_cells_and_preserves_rest_for_next_frame() {
        let mut history = HistoryBuffer::new();
        for idx in 0..1_001 {
            history.enqueue(TranscriptItem::Assistant(format!(
                "row {idx} alpha beta gamma delta epsilon"
            )));
        }

        let first = history.drain_pending_capped(12, RESIZE_REFLOW_PENDING_CELL_CAP);
        assert_eq!(
            first
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            RESIZE_REFLOW_PENDING_CELL_CAP
        );
        assert_eq!(history.pending_cell_count(), 1);
        assert_eq!(history.render_count(), RESIZE_REFLOW_PENDING_CELL_CAP);

        let second = history.drain_pending_capped(40, RESIZE_REFLOW_PENDING_CELL_CAP);
        assert_eq!(
            second
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(history.pending_cell_count(), 0);
        assert_eq!(history.render_count(), RESIZE_REFLOW_PENDING_CELL_CAP + 1);
    }

    #[test]
    fn resize_reflow_rebuilds_from_source_after_pending_drain() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Assistant(
            "alpha beta gamma delta epsilon".to_string(),
        ));

        let narrow = history.drain_pending(12);
        assert_eq!(history.pending_cell_count(), 0);
        let wide = history.reflow_insertions(40, 1_000);

        assert_eq!(wide.len(), 1);
        assert_ne!(wide[0].lines, narrow[0].lines);
        assert_eq!(history.source_cell_count(), 1);
        assert!(history.pending_cell_count() == 0);
    }

    #[test]
    fn resize_reflow_row_cap_keeps_terminal_tail() {
        let mut history = HistoryBuffer::new();
        for idx in 0..5 {
            history.enqueue_cell(Box::new(FixedCell::new(
                match idx {
                    0 => "cell0",
                    1 => "cell1",
                    2 => "cell2",
                    3 => "cell3",
                    _ => "cell4",
                },
                false,
            )));
        }
        history.drain_pending(40);

        let reflow = history.reflow_insertions(40, 3);
        let lines = reflow[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();

        assert_eq!(lines, vec!["  cell3", "", "  cell4"]);
    }

    #[test]
    fn resize_reflow_row_cap_detects_known_terminals() {
        assert_eq!(
            resize_reflow_row_cap_for_values(Some("vscode"), None, false, false),
            VSCODE_RESIZE_REFLOW_MAX_ROWS
        );
        assert_eq!(
            resize_reflow_row_cap_for_values(None, None, true, false),
            WEZTERM_RESIZE_REFLOW_MAX_ROWS
        );
        assert_eq!(
            resize_reflow_row_cap_for_values(None, Some("wezterm"), false, false),
            WEZTERM_RESIZE_REFLOW_MAX_ROWS
        );
        assert_eq!(
            resize_reflow_row_cap_for_values(Some("Alacritty"), None, false, false),
            ALACRITTY_RESIZE_REFLOW_MAX_ROWS
        );
        assert_eq!(
            resize_reflow_row_cap_for_values(None, Some("alacritty"), false, false),
            ALACRITTY_RESIZE_REFLOW_MAX_ROWS
        );
        assert_eq!(
            resize_reflow_row_cap_for_values(None, Some("xterm-256color"), false, false),
            FALLBACK_RESIZE_REFLOW_MAX_ROWS
        );
    }

    #[test]
    fn resize_reflow_state_debounces_and_tracks_stream_finish() {
        let mut state = ResizeReflowState::default();
        let first = state.note_width(80);
        assert!(first.initialized);
        assert!(!state.reflow_needed_for_width(80));

        let changed = state.note_width(100);
        assert!(changed.changed);
        assert!(state.reflow_needed_for_width(100));
        state.schedule_debounced(Some(100));
        assert!(state.has_pending_reflow());
        assert!(!state.pending_is_due(Instant::now()));
        assert!(!state.reflow_needed_for_width(100));

        state.mark_resize_requested_during_stream();
        assert!(state.take_stream_finish_reflow_needed());
        assert!(!state.take_stream_finish_reflow_needed());
    }

    #[test]
    fn cache_key_includes_revision() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue_cell(Box::new(NoticeCell::new("first")));
        let first = history.pending_insertions(40);
        assert_eq!(history.render_count(), 1);
        history.replace_pending_cell(id, Box::new(NoticeCell::new("second")));
        let second = history.pending_insertions(40);
        assert_eq!(history.render_count(), 2);
        assert_ne!(first, second);
    }

    #[test]
    fn cache_evicted_after_drain_and_bounded_for_pending_cells() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Notice("one".to_string()));
        history.pending_insertions(40);
        assert_eq!(history.cache_entry_count(), 1);
        history.drain_pending(40);
        assert_eq!(history.cache_entry_count(), 0);

        for idx in 0..300 {
            history.enqueue(TranscriptItem::Notice(format!("cell {idx}")));
        }
        history.pending_insertions(40);
        assert!(history.cache_entry_count() <= MAX_CACHE_ENTRIES);
        history.drain_pending(40);
        assert_eq!(history.cache_entry_count(), 0);
    }
}
