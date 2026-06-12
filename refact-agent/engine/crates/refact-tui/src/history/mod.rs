use std::collections::{HashMap, VecDeque};
use std::io;

use ratatui::backend::Backend;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Terminal;

use crate::app::TranscriptItem;

pub mod cells;

const MAX_INSERTION_LINES: usize = 2048;
const MAX_CACHE_ENTRIES: usize = 256;

#[derive(Debug, Clone)]
struct HistoryEntry {
    id: u64,
    cell: Box<dyn cells::HistoryCell>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryInsertion {
    pub cell_ids: Vec<u64>,
    pub lines: Vec<Line<'static>>,
}

impl HistoryInsertion {
    pub fn height(&self) -> u16 {
        self.lines.len().min(u16::MAX as usize) as u16
    }
}

#[derive(Debug, Default, Clone)]
pub struct HistoryBuffer {
    next_id: u64,
    pending: VecDeque<HistoryEntry>,
    cache: HashMap<(u64, u16, u64), Vec<Line<'static>>>,
    render_count: usize,
    inserted_cell_count: usize,
}

impl HistoryBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
        self.cache.clear();
    }

    pub fn enqueue(&mut self, item: TranscriptItem) -> u64 {
        self.enqueue_cell(cells::cell_from_transcript_item(&item, false))
    }

    pub fn enqueue_cell(&mut self, cell: Box<dyn cells::HistoryCell>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.pending.push_back(HistoryEntry { id, cell });
        id
    }

    pub fn drain_pending(&mut self, width: u16) -> Vec<HistoryInsertion> {
        let insertions = self.pending_insertions(width);
        self.inserted_cell_count += insertions
            .iter()
            .map(|insertion| insertion.cell_ids.len())
            .sum::<usize>();
        for insertion in &insertions {
            self.evict_cache_entries(&insertion.cell_ids);
        }
        self.pending.clear();
        insertions
    }

    pub fn pending_insertions(&mut self, width: u16) -> Vec<HistoryInsertion> {
        let mut insertions = Vec::new();
        let mut current_ids = Vec::new();
        let mut current_lines = Vec::new();
        let entries = self.pending.iter().cloned().collect::<Vec<_>>();
        for entry in entries {
            let lines = self.render_entry(&entry, width);
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

    pub fn pending_cell_count(&self) -> usize {
        self.pending.len()
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

    fn render_entry(&mut self, entry: &HistoryEntry, width: u16) -> Vec<Line<'static>> {
        let key = (entry.id, width, entry.cell.revision());
        if let Some(lines) = self.cache.get(&key) {
            return lines.clone();
        }
        let lines = entry.cell.render(width as usize);
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
    terminal.insert_before(height, move |buffer| {
        Paragraph::new(lines).render(buffer.area, buffer);
    })
}

pub fn render_transcript_item_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<Line<'static>> {
    cells::render_transcript_item_lines(item, width, selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::NoticeCell;
    use ratatui::backend::TestBackend;
    use ratatui::{TerminalOptions, Viewport};

    fn scrollback_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .scrollback()
            .content()
            .iter()
            .map(|cell| cell.symbol())
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
