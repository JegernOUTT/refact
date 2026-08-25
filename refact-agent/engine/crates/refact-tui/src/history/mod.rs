use std::collections::{HashMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
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
const HISTORY_RETENTION_CELL_CAP: usize = 1_000;
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
    item: Option<TranscriptItem>,
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
    emitted_history_trailing_blank: bool,
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
        self.emitted_history_trailing_blank = false;
    }

    pub fn enqueue(&mut self, item: TranscriptItem) -> u64 {
        let id = self.next_available_id();
        let entry = HistoryEntry {
            id,
            cell: cells::cell_from_transcript_item(&item, false),
            item: Some(item),
        };
        self.history.push_back(entry.clone());
        self.pending.push_back(entry);
        self.enforce_history_retention();
        id
    }

    pub fn enqueue_cell(&mut self, cell: Box<dyn cells::HistoryCell>) -> u64 {
        let id = self.next_available_id();
        let entry = HistoryEntry {
            id,
            item: None,
            cell,
        };
        self.history.push_back(entry.clone());
        self.pending.push_back(entry);
        self.enforce_history_retention();
        id
    }

    pub fn remove_non_final_cells(&mut self, kind: cells::HistoryCellKind) -> usize {
        let mut removed_ids = Vec::new();
        self.history.retain(|entry| {
            let remove = entry.cell.kind() == kind && !entry.cell.is_final();
            if remove {
                removed_ids.push(entry.id);
            }
            !remove
        });
        self.pending.retain(|entry| {
            let remove = entry.cell.kind() == kind && !entry.cell.is_final();
            if remove && !removed_ids.iter().any(|id| *id == entry.id) {
                removed_ids.push(entry.id);
            }
            !remove
        });
        self.evict_cache_entries(&removed_ids);
        removed_ids.len()
    }

    pub fn replace_first_item_kind(&mut self, item: &TranscriptItem) -> Option<bool> {
        let cell = cells::cell_from_transcript_item(item, false);
        let kind = cell.kind();
        let id = self
            .history
            .iter()
            .chain(self.pending.iter())
            .find_map(|entry| {
                (entry.item.is_some() && entry.cell.kind() == kind).then_some(entry.id)
            })?;
        let revision = cell.revision();
        let changed = self
            .history
            .iter()
            .chain(self.pending.iter())
            .filter(|entry| entry.id == id)
            .any(|entry| entry.cell.revision() != revision);
        if !changed {
            for entry in &mut self.history {
                if entry.id == id {
                    entry.item = Some(item.clone());
                }
            }
            for entry in &mut self.pending {
                if entry.id == id {
                    entry.item = Some(item.clone());
                }
            }
            return Some(false);
        }
        for entry in &mut self.history {
            if entry.id == id {
                entry.cell = cell.clone();
                entry.item = Some(item.clone());
            }
        }
        for entry in &mut self.pending {
            if entry.id == id {
                entry.cell = cell.clone();
                entry.item = Some(item.clone());
            }
        }
        self.evict_cache_entries(&[id]);
        Some(true)
    }

    pub fn replace_non_final_cells(
        &mut self,
        kind: cells::HistoryCellKind,
        cell: Box<dyn cells::HistoryCell>,
    ) -> (u64, usize) {
        let removed = self.remove_non_final_cells(kind);
        let id = self.enqueue_cell(cell);
        (id, removed)
    }

    pub fn drain_pending(&mut self, width: u16) -> Vec<HistoryInsertion> {
        self.drain_pending_capped(width, self.pending.len())
    }

    pub fn drain_pending_capped(&mut self, width: u16, max_cells: usize) -> Vec<HistoryInsertion> {
        let insertions = self.pending_insertions_capped(width, max_cells);
        self.drain_insertions(insertions)
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
        let mut last_emitted_line_blank = self
            .emitted_history_lines
            .then_some(self.emitted_history_trailing_blank);
        let entries = self
            .pending
            .iter()
            .take(max_cells)
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            let mut lines = self.render_entry(&entry, width);
            if !lines.is_empty() && !entry.cell.is_stream_continuation() {
                if let Some(previous_blank) = last_emitted_line_blank {
                    if !previous_blank && !hyperlink_line_is_blank(&lines[0]) {
                        lines.insert(0, HyperlinkLine::new(Line::default()));
                    }
                }
            }
            let lines_trailing_blank = lines.last().map(hyperlink_line_is_blank);
            if lines.len() > MAX_INSERTION_LINES {
                if !current_ids.is_empty() {
                    insertions.push(HistoryInsertion {
                        cell_ids: std::mem::take(&mut current_ids),
                        lines: std::mem::take(&mut current_lines),
                    });
                }
                insertions.extend(split_history_insertion(vec![entry.id], lines));
            } else {
                if !current_lines.is_empty()
                    && current_lines.len() + lines.len() > MAX_INSERTION_LINES
                {
                    insertions.push(HistoryInsertion {
                        cell_ids: std::mem::take(&mut current_ids),
                        lines: std::mem::take(&mut current_lines),
                    });
                }
                current_ids.push(entry.id);
                current_lines.extend(lines);
            }
            if let Some(trailing_blank) = lines_trailing_blank {
                last_emitted_line_blank = Some(trailing_blank);
            }
        }
        if !current_ids.is_empty() {
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

        let mut lines = Vec::new();
        let mut displayed_cells = Vec::new();
        let mut last_emitted_line_blank = None::<bool>;
        for display in displays {
            if !display.lines.is_empty() && !display.is_stream_continuation {
                if let Some(previous_blank) = last_emitted_line_blank {
                    if !previous_blank && !hyperlink_line_is_blank(&display.lines[0]) {
                        lines.push(HyperlinkLine::new(Line::default()));
                        last_emitted_line_blank = Some(true);
                    }
                }
            }
            let content_start = lines.len();
            let trailing_blank = display.lines.last().map(hyperlink_line_is_blank);
            lines.extend(display.lines);
            if let Some(trailing_blank) = trailing_blank {
                last_emitted_line_blank = Some(trailing_blank);
            }
            displayed_cells.push((display.id, content_start, lines.len()));
        }

        let visible_start = lines.len().saturating_sub(max_rows);
        let cell_ids = displayed_cells
            .into_iter()
            .filter_map(|(id, start, end)| (start == end || end > visible_start).then_some(id))
            .collect::<Vec<_>>();
        lines = lines.split_off(visible_start);
        self.emitted_history_lines = !lines.is_empty();
        self.emitted_history_trailing_blank = lines.last().is_some_and(hyperlink_line_is_blank);
        let insertions = split_history_insertion(cell_ids, lines);
        let insertions = self.drain_insertions(insertions);
        self.cache.clear();

        insertions
    }

    fn drain_insertions(&mut self, insertions: Vec<HistoryInsertion>) -> Vec<HistoryInsertion> {
        let mut emitted_ids = Vec::new();
        for id in insertions
            .iter()
            .flat_map(|insertion| insertion.cell_ids.iter().copied())
        {
            if !emitted_ids.contains(&id) {
                emitted_ids.push(id);
            }
        }
        let pending_ids = emitted_ids
            .iter()
            .copied()
            .filter(|id| self.pending.iter().any(|entry| entry.id == *id))
            .collect::<Vec<_>>();
        self.inserted_cell_count += pending_ids.len();
        self.note_insertions_emitted(&insertions);
        self.pending
            .retain(|entry| !pending_ids.iter().any(|id| *id == entry.id));
        self.evict_cache_entries(&emitted_ids);
        self.enforce_history_retention();
        insertions
    }

    fn note_insertions_emitted(&mut self, insertions: &[HistoryInsertion]) {
        for insertion in insertions {
            if let Some(last_line) = insertion.lines.last() {
                self.emitted_history_lines = true;
                self.emitted_history_trailing_blank = hyperlink_line_is_blank(last_line);
            }
        }
    }

    pub fn pending_cell_count(&self) -> usize {
        self.pending.len()
    }

    pub fn source_cell_count(&self) -> usize {
        self.history.len()
    }

    pub fn remove_approval_scope(&mut self, scope: &str) -> bool {
        let ids = self
            .history
            .iter()
            .chain(self.pending.iter())
            .filter_map(|entry| match entry.item.as_ref() {
                Some(TranscriptItem::Approval(approval, Some(_))) if approval.scope() == scope => {
                    Some(entry.id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut ids = ids;
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return false;
        }
        self.history
            .retain(|entry| !ids.iter().any(|id| *id == entry.id));
        self.pending
            .retain(|entry| !ids.iter().any(|id| *id == entry.id));
        self.evict_cache_entries(&ids);
        true
    }

    pub fn set_tool_statuses(&mut self, statuses: &[(String, crate::tools::ToolStatus)]) -> bool {
        let mut changed_ids = update_tool_statuses(&mut self.history, statuses);
        changed_ids.extend(update_tool_statuses(&mut self.pending, statuses));
        changed_ids.sort_unstable();
        changed_ids.dedup();
        self.evict_cache_entries(&changed_ids);
        !changed_ids.is_empty()
    }

    pub fn tool_statuses(
        &self,
        tool_call_ids: &[String],
    ) -> Vec<(String, crate::tools::ToolStatus)> {
        self.history
            .iter()
            .filter_map(|entry| match entry.item.as_ref() {
                Some(TranscriptItem::Tool(card))
                    if tool_call_ids.iter().any(|id| id == &card.id) =>
                {
                    Some((card.id.clone(), card.status))
                }
                _ => None,
            })
            .collect()
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

    fn next_available_id(&mut self) -> u64 {
        let first_candidate = self.next_id;
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if !self.id_is_live(id) {
                return id;
            }
            assert_ne!(
                self.next_id, first_candidate,
                "history cell id space exhausted"
            );
        }
    }

    fn id_is_live(&self, id: u64) -> bool {
        self.history
            .iter()
            .chain(self.pending.iter())
            .any(|entry| entry.id == id)
    }

    fn enforce_history_retention(&mut self) {
        let mut evicted_ids = Vec::new();
        while self.history.len() > HISTORY_RETENTION_CELL_CAP {
            if let Some(entry) = self.history.pop_front() {
                evicted_ids.push(entry.id);
            }
        }
        self.pending
            .retain(|entry| !evicted_ids.iter().any(|id| *id == entry.id));
        self.evict_cache_entries(&evicted_ids);
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

fn split_history_insertion(cell_ids: Vec<u64>, lines: Vec<HyperlinkLine>) -> Vec<HistoryInsertion> {
    if lines.len() <= MAX_INSERTION_LINES {
        return vec![HistoryInsertion { cell_ids, lines }];
    }

    lines
        .chunks(MAX_INSERTION_LINES)
        .map(|chunk| HistoryInsertion {
            cell_ids: cell_ids.clone(),
            lines: chunk.to_vec(),
        })
        .collect()
}

fn update_tool_statuses(
    entries: &mut VecDeque<HistoryEntry>,
    statuses: &[(String, crate::tools::ToolStatus)],
) -> Vec<u64> {
    let mut changed_ids = Vec::new();
    for entry in entries {
        let Some(TranscriptItem::Tool(card)) = entry.item.as_mut() else {
            continue;
        };
        let Some((_, status)) = statuses.iter().find(|(id, _)| id == &card.id) else {
            continue;
        };
        if card.status == *status {
            continue;
        }
        card.status = *status;
        if status.is_final() {
            card.subchat_active = false;
        }
        entry.cell = cells::cell_from_transcript_item(&TranscriptItem::Tool(card.clone()), false);
        changed_ids.push(entry.id);
    }
    changed_ids
}

fn hyperlink_line_is_blank(line: &HyperlinkLine) -> bool {
    line.line
        .spans
        .iter()
        .all(|span| span.content.trim().is_empty())
}

fn fill_line_backgrounds(buffer: &mut Buffer, area: Rect, lines: &[HyperlinkLine]) {
    for (row, line) in lines.iter().enumerate().take(usize::from(area.height)) {
        if line.line.style.bg.is_some() {
            buffer.set_style(
                Rect::new(area.x, area.y.saturating_add(row as u16), area.width, 1),
                line.line.style,
            );
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
    crate::vendored::terminal_hyperlinks::clear_buffer_hyperlinks();
    terminal.insert_before(height, move |buffer| {
        let area = buffer.area;
        fill_line_backgrounds(buffer, area, &lines);
        let visible = visible_lines(lines.clone());
        Paragraph::new(visible).render(area, buffer);
        mark_buffer_hyperlinks(&*buffer, area, &lines, enabled);
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
    use crate::approvals::{ApprovalModalState, PauseReason};
    use crate::history::cells::NoticeCell;
    use crate::render::wrapping::line_to_plain;
    use crate::tools::{ToolCard, ToolStatus};
    use ratatui::backend::TestBackend;
    use ratatui::{TerminalOptions, Viewport};
    use serde_json::json;

    #[derive(Debug, Clone)]
    struct FixedCell {
        text: String,
        continuation: bool,
    }

    impl FixedCell {
        fn new(text: impl Into<String>, continuation: bool) -> Self {
            Self {
                text: text.into(),
                continuation,
            }
        }
    }

    #[derive(Debug, Clone)]
    struct MultiLineCell {
        lines: &'static [&'static str],
    }

    impl MultiLineCell {
        fn new(lines: &'static [&'static str]) -> Self {
            Self { lines }
        }
    }

    #[derive(Debug, Clone)]
    struct RepeatedLineCell {
        line_count: usize,
    }

    impl RepeatedLineCell {
        fn new(line_count: usize) -> Self {
            Self { line_count }
        }
    }

    impl cells::HistoryCell for MultiLineCell {
        fn kind(&self) -> cells::HistoryCellKind {
            cells::HistoryCellKind::Info
        }

        fn render(&self, _width: usize) -> Vec<Line<'static>> {
            self.lines.iter().copied().map(Line::from).collect()
        }

        fn revision(&self) -> u64 {
            self.lines.len() as u64
        }
    }

    impl cells::HistoryCell for RepeatedLineCell {
        fn kind(&self) -> cells::HistoryCellKind {
            cells::HistoryCellKind::Info
        }

        fn render(&self, _width: usize) -> Vec<Line<'static>> {
            (0..self.line_count)
                .map(|index| Line::from(format!("line {index}")))
                .collect()
        }

        fn revision(&self) -> u64 {
            self.line_count as u64
        }
    }

    impl cells::HistoryCell for FixedCell {
        fn kind(&self) -> cells::HistoryCellKind {
            cells::HistoryCellKind::Info
        }

        fn render(&self, _width: usize) -> Vec<Line<'static>> {
            vec![Line::from(self.text.clone())]
        }

        fn is_stream_continuation(&self) -> bool {
            self.continuation
        }

        fn revision(&self) -> u64 {
            self.text.len() as u64 + u64::from(self.continuation)
        }
    }

    #[derive(Debug, Clone)]
    struct EmptyCell;

    impl cells::HistoryCell for EmptyCell {
        fn kind(&self) -> cells::HistoryCellKind {
            cells::HistoryCellKind::Info
        }

        fn render(&self, _width: usize) -> Vec<Line<'static>> {
            Vec::new()
        }

        fn revision(&self) -> u64 {
            0
        }
    }

    fn approval(scope: &str, tool_call_id: &str) -> ApprovalModalState {
        ApprovalModalState::with_scope(
            scope,
            vec![PauseReason {
                reason_type: "confirmation".to_string(),
                tool_name: "shell".to_string(),
                command: "echo hi".to_string(),
                rule: "default".to_string(),
                tool_call_id: tool_call_id.to_string(),
                integr_config_path: None,
                args: None,
                diff: None,
            }],
        )
    }

    fn tool_card(id: &str, status: ToolStatus) -> ToolCard {
        let mut card = ToolCard::from_tool_call(&json!({
            "id": id,
            "function": {"name": "shell", "arguments": "{}"}
        }));
        card.status = status;
        card.subchat_active = true;
        card
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
        assert_eq!(terminal.backend().size().unwrap().width, 40);
    }

    #[test]
    fn enqueue_retains_transcript_identity_but_enqueue_cell_does_not() {
        let mut history = HistoryBuffer::new();
        let item_id = history.enqueue(TranscriptItem::Notice("item".to_string()));
        let cell_id = history.enqueue_cell(Box::new(NoticeCell::new("cell")));

        assert!(matches!(
            history.history.iter().find(|entry| entry.id == item_id),
            Some(HistoryEntry {
                item: Some(TranscriptItem::Notice(text)),
                ..
            }) if text == "item"
        ));
        assert!(history
            .history
            .iter()
            .find(|entry| entry.id == cell_id)
            .is_some_and(|entry| entry.item.is_none()));
        assert!(history
            .pending
            .iter()
            .find(|entry| entry.id == item_id)
            .is_some_and(|entry| entry.item.is_some()));
        assert!(history
            .pending
            .iter()
            .find(|entry| entry.id == cell_id)
            .is_some_and(|entry| entry.item.is_none()));
    }

    #[test]
    fn remove_approval_scope_removes_only_optimistic_matching_entries_and_cache() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Approval(
            approval("scope-a", "call-a"),
            Some(ToolStatus::ApprovedOnce),
        ));
        history.enqueue(TranscriptItem::Approval(
            approval("scope-b", "call-b"),
            Some(ToolStatus::Denied),
        ));
        history.enqueue(TranscriptItem::Approval(
            approval("scope-a", "call-c"),
            None,
        ));
        history.pending_insertions(80);
        assert_eq!(history.cache_entry_count(), 3);

        assert!(history.remove_approval_scope("scope-a"));
        assert_eq!(history.source_cell_count(), 2);
        assert_eq!(history.pending_cell_count(), 2);
        assert_eq!(history.cache_entry_count(), 2);
        assert!(!history.remove_approval_scope("scope-a"));
        assert!(history.history.iter().any(|entry| {
            matches!(
                entry.item,
                Some(TranscriptItem::Approval(ref modal, Some(ToolStatus::Denied)))
                    if modal.scope() == "scope-b"
            )
        }));
        assert!(history.history.iter().any(|entry| {
            matches!(
                entry.item,
                Some(TranscriptItem::Approval(ref modal, None)) if modal.scope() == "scope-a"
            )
        }));
    }

    #[test]
    fn tool_statuses_read_and_update_history_pending_and_cache() {
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::Tool(tool_card(
            "call-a",
            ToolStatus::AwaitingApproval,
        )));
        history.enqueue_cell(Box::new(NoticeCell::new("cell-only")));
        history.pending_insertions(80);
        assert_eq!(history.cache_entry_count(), 2);
        assert_eq!(
            history.tool_statuses(&[
                "call-a".to_string(),
                "missing".to_string(),
                "call-b".to_string(),
            ]),
            vec![("call-a".to_string(), ToolStatus::AwaitingApproval)]
        );

        assert!(history.set_tool_statuses(&[("call-a".to_string(), ToolStatus::Denied)]));
        assert_eq!(history.cache_entry_count(), 1);
        assert!(!history.set_tool_statuses(&[("call-a".to_string(), ToolStatus::Denied)]));
        assert!(!history.set_tool_statuses(&[("missing".to_string(), ToolStatus::Succeeded)]));
        for entry in history.history.iter().chain(history.pending.iter()) {
            if let Some(TranscriptItem::Tool(card)) = &entry.item {
                assert_eq!(card.id, "call-a");
                assert_eq!(card.status, ToolStatus::Denied);
                assert!(!card.subchat_active);
            }
        }
        let lines = history
            .pending_insertions(80)
            .into_iter()
            .flat_map(|insertion| insertion.lines)
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lines.contains("denied"));
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
        assert_eq!(lines, vec!["  • one", "  ", "  • two"]);

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
    fn replace_first_kind_synchronizes_item_backed_source_and_cell() {
        let mut history = HistoryBuffer::new();
        let cell_only_id = history.enqueue_cell(Box::new(cells::SessionCell::new(
            "cell only",
            Some("model: raw".to_string()),
        )));
        let item_id = history.enqueue(TranscriptItem::Session {
            title: "New chat".to_string(),
            subtitle: Some("model: default".to_string()),
        });
        history.pending_insertions(80);
        assert_eq!(history.cache_entry_count(), 2);

        let updated = TranscriptItem::Session {
            title: "New chat".to_string(),
            subtitle: Some("model: gpt-demo".to_string()),
        };

        let changed = history.replace_first_item_kind(&updated);

        assert_eq!(changed, Some(true));
        assert_eq!(history.cache_entry_count(), 1);
        for entry in history.history.iter().chain(history.pending.iter()) {
            if entry.id == item_id {
                assert_eq!(entry.item.as_ref(), Some(&updated));
                assert_eq!(
                    entry.cell.revision(),
                    cells::cell_from_transcript_item(&updated, false).revision()
                );
            }
            if entry.id == cell_only_id {
                assert!(entry.item.is_none());
            }
        }
        assert_eq!(history.pending_cell_count(), 2);
        assert_eq!(history.source_cell_count(), 2);
        let lines = history
            .pending_insertions(80)
            .into_iter()
            .flat_map(|insertion| insertion.lines)
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lines.contains("model: gpt-demo"));
        assert!(!lines.contains("model: default"));

        history.drain_pending(80);
        let reflow = history
            .reflow_insertions(80, 1_000)
            .into_iter()
            .flat_map(|insertion| insertion.lines)
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(reflow.contains("model: gpt-demo"));
        assert!(reflow.contains("model: raw"));
    }

    #[test]
    fn replace_first_kind_syncs_source_when_cell_revision_is_unchanged() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue(TranscriptItem::Session {
            title: "Original title".to_string(),
            subtitle: Some("model: default".to_string()),
        });
        let updated = TranscriptItem::Session {
            title: "Renamed title".to_string(),
            subtitle: Some("model: default".to_string()),
        };

        history.replace_pending_cell(id, cells::cell_from_transcript_item(&updated, false));
        assert_eq!(history.replace_first_item_kind(&updated), Some(false));
        assert!(history
            .history
            .iter()
            .chain(history.pending.iter())
            .filter(|entry| entry.id == id)
            .all(|entry| entry.item.as_ref() == Some(&updated)));
    }

    #[test]
    fn pending_insertions_keep_one_blank_line_between_message_turns() {
        let _color = crate::style::override_color_enabled_for_test(true);
        let _bg = crate::terminal_palette::override_default_bg_for_test(Some((0, 0, 0)));
        let mut history = HistoryBuffer::new();
        history.enqueue(TranscriptItem::User("first".to_string()));
        history.enqueue(TranscriptItem::Assistant("answer".to_string()));
        history.enqueue(TranscriptItem::User("second".to_string()));

        let preview = history.pending_insertions(40);
        let lines = preview[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();

        assert_eq!(
            lines,
            vec![
                "  ",
                "  › first",
                "  ",
                "  • answer",
                "  ",
                "  › second",
                "  "
            ]
        );

        let expected_bg = crate::style::user_message_style().bg.unwrap();
        assert_eq!(preview[0].lines[0].line.style.bg, Some(expected_bg));
        assert_eq!(preview[0].lines[1].line.style.bg, Some(expected_bg));
        assert_eq!(preview[0].lines[2].line.style.bg, Some(expected_bg));
        assert_eq!(preview[0].lines[3].line.style.bg, None);
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
    fn markdown_link_inserted_into_scrollback_keeps_cell_symbols_visible() {
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
        mark_buffer_hyperlinks(&buffer, area, &lines, true);
        let raw = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!raw.contains('\x1b'));
        assert!(raw.contains("Read docs (https://example.com/docs) now"));
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
        for idx in 0..RESIZE_REFLOW_PENDING_CELL_CAP {
            history.enqueue(TranscriptItem::Assistant(format!(
                "row {idx} alpha beta gamma delta epsilon"
            )));
        }

        let first = history.drain_pending_capped(12, RESIZE_REFLOW_PENDING_CELL_CAP - 1);
        assert_eq!(
            first
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            RESIZE_REFLOW_PENDING_CELL_CAP - 1
        );
        assert_eq!(history.pending_cell_count(), 1);
        assert_eq!(history.render_count(), RESIZE_REFLOW_PENDING_CELL_CAP - 1);

        let second = history.drain_pending_capped(40, RESIZE_REFLOW_PENDING_CELL_CAP);
        assert_eq!(
            second
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(history.pending_cell_count(), 0);
        assert_eq!(history.render_count(), RESIZE_REFLOW_PENDING_CELL_CAP);
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
    fn resize_reflow_drains_partially_visible_cells_and_keeps_trimmed_cells_pending() {
        let mut history = HistoryBuffer::new();
        let first_id = history.enqueue_cell(Box::new(MultiLineCell::new(&["cell0a", "cell0b"])));
        let second_id = history.enqueue_cell(Box::new(MultiLineCell::new(&["cell1a", "cell1b"])));
        let third_id = history.enqueue_cell(Box::new(MultiLineCell::new(&["cell2a", "cell2b"])));

        let reflow = history.reflow_insertions(40, 4);
        assert_eq!(reflow.len(), 1);
        assert_eq!(reflow[0].cell_ids, vec![second_id, third_id]);
        let reflow_lines = reflow[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(reflow_lines, vec!["  cell1b", "", "  cell2a", "  cell2b"]);
        assert_eq!(
            history
                .pending
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![first_id]
        );

        let later = history.drain_pending(40);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].cell_ids, vec![first_id]);
        let later_lines = later[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(later_lines, vec!["", "  cell0a", "  cell0b"]);
        assert_eq!(history.pending_cell_count(), 0);
        assert_eq!(history.inserted_cell_count(), 3);
    }

    #[test]
    fn resize_reflow_drains_a_partially_visible_ten_line_cell_once() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue_cell(Box::new(MultiLineCell::new(&[
            "line0", "line1", "line2", "line3", "line4", "line5", "line6", "line7", "line8",
            "line9",
        ])));

        let reflow = history.reflow_insertions(40, 4);
        assert_eq!(reflow.len(), 1);
        assert_eq!(reflow[0].cell_ids, vec![id]);
        let reflow_lines = reflow[0]
            .lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(
            reflow_lines,
            vec!["  line6", "  line7", "  line8", "  line9"]
        );
        assert_eq!(history.pending_cell_count(), 0);

        assert!(history.drain_pending(40).is_empty());
        assert_eq!(history.inserted_cell_count(), 1);
    }

    #[test]
    fn resize_reflow_drains_zero_line_cells_once() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue_cell(Box::new(EmptyCell));

        let reflow = history.reflow_insertions(40, 4);
        assert_eq!(reflow.len(), 1);
        assert_eq!(reflow[0].cell_ids, vec![id]);
        assert!(reflow[0].lines.is_empty());
        assert_eq!(history.pending_cell_count(), 0);

        assert!(history.drain_pending(40).is_empty());
        assert_eq!(history.inserted_cell_count(), 1);
    }

    #[test]
    fn zero_line_cells_are_drained_once() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue_cell(Box::new(EmptyCell));

        let first = history.drain_pending(40);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].cell_ids, vec![id]);
        assert!(first[0].lines.is_empty());
        assert_eq!(history.pending_cell_count(), 0);
        assert!(history.drain_pending(40).is_empty());
        assert_eq!(history.inserted_cell_count(), 1);
    }

    #[test]
    fn history_retention_bounds_a_500_turn_trajectory() {
        let mut history = HistoryBuffer::new();
        let oldest_id = history.enqueue(TranscriptItem::Notice("initial".to_string()));
        history.drain_pending(80);

        for turn in 0..500 {
            history.enqueue(TranscriptItem::User(format!("user {turn}")));
            history.enqueue(TranscriptItem::Assistant(format!("assistant {turn}")));
            history.drain_pending(80);
        }

        assert_eq!(history.source_cell_count(), HISTORY_RETENTION_CELL_CAP);
        assert!(!history.history.iter().any(|entry| entry.id == oldest_id));
        assert_eq!(history.pending_cell_count(), 0);
    }

    #[test]
    fn history_retention_keeps_only_the_chronological_undrained_tail() {
        let mut history = HistoryBuffer::new();
        let ids = (0..=HISTORY_RETENTION_CELL_CAP)
            .map(|index| {
                history.enqueue_cell(Box::new(FixedCell::new(format!("cell {index}"), false)))
            })
            .collect::<Vec<_>>();

        let expected = ids[1..].to_vec();
        assert_eq!(history.source_cell_count(), HISTORY_RETENTION_CELL_CAP);
        assert_eq!(history.pending_cell_count(), HISTORY_RETENTION_CELL_CAP);
        assert_eq!(
            history
                .history
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            history
                .pending
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            expected
        );

        let drained = history.drain_pending(80);
        assert_eq!(
            drained
                .iter()
                .flat_map(|insertion| insertion.cell_ids.iter().copied())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(history.pending_cell_count(), 0);
    }

    #[test]
    fn history_retention_keeps_mixed_reflow_and_pending_drains_in_the_retained_tail() {
        let mut history = HistoryBuffer::new();
        let initial_ids = (0..500)
            .map(|index| {
                history.enqueue_cell(Box::new(FixedCell::new(format!("initial {index}"), false)))
            })
            .collect::<Vec<_>>();
        history.drain_pending_capped(80, 250);

        let pending_ids = (0..751)
            .map(|index| {
                history.enqueue_cell(Box::new(FixedCell::new(format!("pending {index}"), false)))
            })
            .collect::<Vec<_>>();
        let expected = initial_ids[251..]
            .iter()
            .chain(pending_ids.iter())
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(history.source_cell_count(), HISTORY_RETENTION_CELL_CAP);
        assert_eq!(
            history
                .history
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            history
                .pending
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            initial_ids[251..]
                .iter()
                .chain(pending_ids.iter())
                .copied()
                .collect::<Vec<_>>()
        );

        let reflow = history.reflow_insertions(80, 3);
        let reflow_ids = reflow
            .iter()
            .flat_map(|insertion| insertion.cell_ids.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(reflow_ids, expected[expected.len() - 2..]);

        let later = history.drain_pending(80);
        assert_eq!(
            later
                .iter()
                .flat_map(|insertion| insertion.cell_ids.iter().copied())
                .collect::<Vec<_>>(),
            initial_ids[251..]
                .iter()
                .chain(pending_ids[..pending_ids.len() - 2].iter())
                .copied()
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn oversized_cells_split_at_rendered_line_boundaries_and_drain_once() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue_cell(Box::new(RepeatedLineCell::new(MAX_INSERTION_LINES * 2 + 1)));

        let insertions = history.drain_pending(80);
        assert_eq!(insertions.len(), 3);
        assert!(insertions
            .iter()
            .all(|insertion| insertion.lines.len() <= MAX_INSERTION_LINES));
        assert_eq!(insertions[0].cell_ids, vec![id]);
        assert_eq!(insertions[1].cell_ids, vec![id]);
        assert_eq!(insertions[2].cell_ids, vec![id]);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.lines.len())
                .collect::<Vec<_>>(),
            vec![MAX_INSERTION_LINES, MAX_INSERTION_LINES, 1]
        );
        assert_eq!(history.pending_cell_count(), 0);
        assert_eq!(history.inserted_cell_count(), 1);
        assert!(history.drain_pending(80).is_empty());
    }

    #[test]
    fn reflow_insertions_split_oversized_visible_suffixes() {
        let mut history = HistoryBuffer::new();
        let id = history.enqueue_cell(Box::new(RepeatedLineCell::new(MAX_INSERTION_LINES + 1)));

        let insertions = history.reflow_insertions(80, MAX_INSERTION_LINES + 1);
        assert_eq!(insertions.len(), 2);
        assert!(insertions
            .iter()
            .all(|insertion| insertion.lines.len() <= MAX_INSERTION_LINES));
        assert_eq!(insertions[0].cell_ids, vec![id]);
        assert_eq!(insertions[1].cell_ids, vec![id]);
        assert_eq!(history.pending_cell_count(), 0);
        assert_eq!(history.inserted_cell_count(), 1);
    }

    #[test]
    fn history_id_wrap_skips_live_ids() {
        let mut history = HistoryBuffer::new();
        history.next_id = 0;
        let zero_id = history.enqueue_cell(Box::new(FixedCell::new("zero", false)));
        history.next_id = u64::MAX;
        let wrapped_id = history.enqueue_cell(Box::new(FixedCell::new("wrapped", false)));

        let next_id = history.enqueue_cell(Box::new(FixedCell::new("next", false)));

        assert_eq!(zero_id, 0);
        assert_eq!(wrapped_id, u64::MAX);
        assert_eq!(next_id, 1);
        assert_eq!(history.source_cell_count(), 3);
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
