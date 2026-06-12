pub mod queue;

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const HISTORY_LIMIT: usize = 200;
const PASTE_BURST_WINDOW: Duration = Duration::from_millis(35);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerView {
    pub lines: Vec<String>,
    pub cursor_row: u16,
    pub cursor_col: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterDecision {
    Submit,
    InsertedNewline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerState {
    editor: TextEditor,
    history: InputHistory,
    paste: PasteBurst,
}

impl ComposerState {
    pub fn new(history: Vec<String>) -> Self {
        Self {
            editor: TextEditor::new(),
            history: InputHistory::new(history),
            paste: PasteBurst::new(),
        }
    }

    pub fn text(&self) -> &str {
        self.editor.text()
    }

    pub fn is_empty(&self) -> bool {
        self.editor.text().is_empty()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.paste.reset();
        self.history.reset_navigation();
        self.editor.set_text(text.into());
    }

    pub fn clear(&mut self) {
        self.paste.reset();
        self.history.reset_navigation();
        self.editor.clear();
    }

    pub fn insert_char(&mut self, ch: char, now: Instant) {
        self.flush_pending_paste(now);
        self.history.reset_navigation();
        let action = self.paste.push_char(ch, now);
        self.apply_paste_action(action);
    }

    pub fn insert_explicit_newline(&mut self, now: Instant) {
        self.flush_pending_paste(now);
        self.history.reset_navigation();
        let action = self.paste.push_explicit_newline(now);
        self.apply_paste_action(action);
    }

    pub fn enter(&mut self, now: Instant) -> EnterDecision {
        if self.flush_pending_paste(now) {
            return EnterDecision::Submit;
        }
        match self.paste.push_enter(now) {
            PasteAction::None => EnterDecision::Submit,
            action => {
                self.history.reset_navigation();
                self.apply_paste_action(action);
                EnterDecision::InsertedNewline
            }
        }
    }

    pub fn insert_paste(&mut self, text: &str) {
        self.flush_pending_paste_force();
        self.paste.reset();
        self.history.reset_navigation();
        self.editor.insert_str(text);
    }

    pub fn insert_text(&mut self, text: &str) {
        self.flush_pending_paste_force();
        self.paste.reset();
        self.history.reset_navigation();
        self.editor.insert_str(text);
    }

    pub fn flush_pending_paste(&mut self, now: Instant) -> bool {
        if let Some(text) = self.paste.take_if_expired(now) {
            self.editor.insert_str(&text);
            self.history.reset_navigation();
            true
        } else {
            false
        }
    }

    pub fn flush_pending_paste_force(&mut self) -> bool {
        if let Some(text) = self.paste.take_pending() {
            self.editor.insert_str(&text);
            true
        } else {
            false
        }
    }

    pub fn submit_text(&mut self) -> Option<String> {
        self.flush_pending_paste_force();
        let prompt = self.editor.text().trim().to_string();
        if prompt.is_empty() {
            return None;
        }
        self.history.push(prompt.clone());
        self.editor.clear();
        self.paste.reset();
        Some(prompt)
    }

    pub fn history_entries(&self) -> &[String] {
        self.history.entries()
    }

    pub fn move_left(&mut self, select: bool) {
        self.cancel_edit_tracking();
        self.editor.move_left(select);
    }

    pub fn move_right(&mut self, select: bool) {
        self.cancel_edit_tracking();
        self.editor.move_right(select);
    }

    pub fn move_home(&mut self, select: bool) {
        self.cancel_edit_tracking();
        self.editor.move_line_start(select);
    }

    pub fn move_end(&mut self, select: bool) {
        self.cancel_edit_tracking();
        self.editor.move_line_end(select);
    }

    pub fn backspace(&mut self) {
        self.cancel_edit_tracking();
        self.history.reset_navigation();
        self.editor.backspace();
    }

    pub fn delete(&mut self) {
        self.cancel_edit_tracking();
        self.history.reset_navigation();
        self.editor.delete();
    }

    pub fn move_up_or_history(&mut self, select: bool) {
        self.flush_pending_paste_force();
        if select || self.editor.can_move_up() {
            self.paste.reset();
            self.history.reset_navigation();
            self.editor.move_up(select);
            return;
        }
        let current = self.editor.text().to_string();
        if let Some(text) = self.history.previous(current) {
            self.editor.set_text(text);
        }
    }

    pub fn move_down_or_history(&mut self, select: bool) {
        self.flush_pending_paste_force();
        if select || self.editor.can_move_down() {
            self.paste.reset();
            self.history.reset_navigation();
            self.editor.move_down(select);
            return;
        }
        let current = self.editor.text().to_string();
        if let Some(text) = self.history.next(current) {
            self.editor.set_text(text);
        }
    }

    pub fn can_move_up(&self) -> bool {
        self.editor.can_move_up()
    }

    pub fn can_move_down(&self) -> bool {
        self.editor.can_move_down()
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.editor
            .selection_range()
            .map(|range| &self.editor.text()[range])
    }

    pub fn cursor_char_offset(&self) -> i64 {
        self.editor.text()[..self.editor.cursor()].chars().count() as i64
    }

    pub fn replace_current_token(&mut self, marker: char, replacement: &str) {
        self.flush_pending_paste_force();
        self.paste.reset();
        self.history.reset_navigation();
        let cursor = self.editor.cursor();
        let prefix = &self.editor.text()[..cursor];
        let start = prefix
            .char_indices()
            .rev()
            .find_map(|(idx, ch)| (ch == marker).then_some(idx));
        if let Some(start) = start {
            self.editor.remove_range(start..cursor);
        }
        self.editor.insert_str(replacement);
    }

    pub fn height(&self, width: u16, max_rows: u16) -> u16 {
        self.view(width, max_rows).lines.len().max(1) as u16 + 2
    }

    pub fn view(&self, width: u16, max_rows: u16) -> ComposerView {
        self.editor
            .view(width.max(1) as usize, max_rows.max(1) as usize)
    }

    fn apply_paste_action(&mut self, action: PasteAction) {
        match action {
            PasteAction::InsertAndTrack { ch, reset_start } => {
                let start = self.editor.cursor();
                self.editor.insert_char(ch);
                if reset_start {
                    self.paste.set_candidate_range(start, self.editor.cursor());
                } else {
                    self.paste.set_candidate_end(self.editor.cursor());
                }
            }
            PasteAction::Insert(text) => self.editor.insert_str(&text),
            PasteAction::RemoveAndHold { start, end } => self.editor.remove_range(start..end),
            PasteAction::Hold | PasteAction::None => {}
        }
    }

    fn cancel_edit_tracking(&mut self) {
        self.flush_pending_paste_force();
        self.paste.reset();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditor {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
}

impl TextEditor {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            selection_anchor: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.selection_anchor = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selection_anchor = None;
    }

    pub fn insert_char(&mut self, ch: char) {
        self.insert_str(&ch.to_string());
    }

    pub fn insert_str(&mut self, value: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, value);
        self.cursor += value.len();
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let Some(prev) = previous_boundary(&self.text, self.cursor) else {
            return;
        };
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        let Some(next) = next_boundary(&self.text, self.cursor) else {
            return;
        };
        self.text.replace_range(self.cursor..next, "");
    }

    pub fn move_left(&mut self, select: bool) {
        let target = previous_boundary(&self.text, self.cursor).unwrap_or(0);
        self.set_cursor(target, select);
    }

    pub fn move_right(&mut self, select: bool) {
        let target = next_boundary(&self.text, self.cursor).unwrap_or(self.text.len());
        self.set_cursor(target, select);
    }

    pub fn move_line_start(&mut self, select: bool) {
        let start = self.text[..self.cursor]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(0);
        self.set_cursor(start, select);
    }

    pub fn move_line_end(&mut self, select: bool) {
        let end = self.text[self.cursor..]
            .find('\n')
            .map(|idx| self.cursor + idx)
            .unwrap_or(self.text.len());
        self.set_cursor(end, select);
    }

    pub fn can_move_up(&self) -> bool {
        self.line_col().0 > 0
    }

    pub fn can_move_down(&self) -> bool {
        self.line_col().0 + 1 < self.line_count()
    }

    pub fn move_up(&mut self, select: bool) {
        let (line, col) = self.line_col();
        if line == 0 {
            self.set_cursor(0, select);
            return;
        }
        self.set_cursor(self.offset_for_line_col(line - 1, col), select);
    }

    pub fn move_down(&mut self, select: bool) {
        let (line, col) = self.line_col();
        if line + 1 >= self.line_count() {
            self.set_cursor(self.text.len(), select);
            return;
        }
        self.set_cursor(self.offset_for_line_col(line + 1, col), select);
    }

    pub fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            None
        } else if anchor < self.cursor {
            Some(anchor..self.cursor)
        } else {
            Some(self.cursor..anchor)
        }
    }

    fn set_cursor(&mut self, target: usize, select: bool) {
        let target = clamp_boundary(&self.text, target);
        if select {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = target;
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.selection_anchor = None;
        true
    }

    fn remove_range(&mut self, range: Range<usize>) {
        let start = clamp_boundary(&self.text, range.start);
        let end = clamp_boundary(&self.text, range.end);
        if start <= end {
            self.text.replace_range(start..end, "");
            self.cursor = start;
            self.selection_anchor = None;
        }
    }

    fn line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for ch in self.text[..self.cursor].chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn line_count(&self) -> usize {
        self.text.chars().filter(|ch| *ch == '\n').count() + 1
    }

    fn offset_for_line_col(&self, target_line: usize, target_col: usize) -> usize {
        let mut line = 0;
        let mut col = 0;
        for (idx, ch) in self.text.char_indices() {
            if line == target_line && col == target_col {
                return idx;
            }
            if ch == '\n' {
                if line == target_line {
                    return idx;
                }
                line += 1;
                col = 0;
            } else if line == target_line {
                col += 1;
            }
        }
        self.text.len()
    }

    fn view(&self, width: usize, max_rows: usize) -> ComposerView {
        let (rows, cursor_row, cursor_col) = wrap_rows(&self.text, self.cursor, width.max(1));
        let max_rows = max_rows.max(1);
        let scroll_top = cursor_row.saturating_add(1).saturating_sub(max_rows);
        let visible = rows
            .into_iter()
            .skip(scroll_top)
            .take(max_rows)
            .collect::<Vec<_>>();
        ComposerView {
            lines: if visible.is_empty() {
                vec![String::new()]
            } else {
                visible
            },
            cursor_row: cursor_row.saturating_sub(scroll_top) as u16,
            cursor_col: cursor_col as u16,
        }
    }
}

impl Default for TextEditor {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteBurst {
    pending: Option<PendingPaste>,
    candidate: Option<PasteCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPaste {
    text: String,
    last_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteCandidate {
    start: usize,
    end: usize,
    text: String,
    last_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PasteAction {
    None,
    InsertAndTrack { ch: char, reset_start: bool },
    Insert(String),
    RemoveAndHold { start: usize, end: usize },
    Hold,
}

impl PasteBurst {
    fn new() -> Self {
        Self {
            pending: None,
            candidate: None,
        }
    }

    fn push_char(&mut self, ch: char, now: Instant) -> PasteAction {
        if let Some(pending) = self.pending.as_mut() {
            pending.text.push(ch);
            pending.last_at = now;
            return PasteAction::Hold;
        }

        if let Some(candidate) = self.candidate.as_mut() {
            if now.duration_since(candidate.last_at) <= PASTE_BURST_WINDOW {
                candidate.text.push(ch);
                candidate.last_at = now;
                return PasteAction::InsertAndTrack {
                    ch,
                    reset_start: false,
                };
            }
        }

        self.candidate = Some(PasteCandidate {
            start: 0,
            end: 0,
            text: ch.to_string(),
            last_at: now,
        });
        PasteAction::InsertAndTrack {
            ch,
            reset_start: true,
        }
    }

    fn push_explicit_newline(&mut self, _now: Instant) -> PasteAction {
        if self.pending.is_some() {
            return self
                .take_pending()
                .map(|mut text| {
                    text.push('\n');
                    PasteAction::Insert(text)
                })
                .unwrap_or(PasteAction::None);
        }
        self.candidate = None;
        PasteAction::Insert("\n".to_string())
    }

    fn push_enter(&mut self, now: Instant) -> PasteAction {
        if let Some(pending) = self.pending.as_mut() {
            pending.text.push('\n');
            pending.last_at = now;
            return PasteAction::Hold;
        }
        if let Some(candidate) = self.candidate.as_mut() {
            if now.duration_since(candidate.last_at) <= PASTE_BURST_WINDOW
                && candidate.text.chars().count() > 1
            {
                let mut candidate = self.candidate.take().expect("candidate checked");
                candidate.text.push('\n');
                self.pending = Some(PendingPaste {
                    text: candidate.text,
                    last_at: now,
                });
                return PasteAction::RemoveAndHold {
                    start: candidate.start,
                    end: candidate.end,
                };
            }
        }
        self.candidate = None;
        PasteAction::None
    }

    fn set_candidate_range(&mut self, start: usize, end: usize) {
        if let Some(candidate) = self.candidate.as_mut() {
            candidate.start = start;
            candidate.end = end;
        }
    }

    fn set_candidate_end(&mut self, end: usize) {
        if let Some(candidate) = self.candidate.as_mut() {
            candidate.end = end;
        }
    }

    fn take_if_expired(&mut self, now: Instant) -> Option<String> {
        let pending = self.pending.as_ref()?;
        if now.duration_since(pending.last_at) >= PASTE_BURST_WINDOW {
            self.take_pending()
        } else {
            None
        }
    }

    fn take_pending(&mut self) -> Option<String> {
        self.pending.take().map(|pending| pending.text)
    }

    fn reset(&mut self) {
        self.pending = None;
        self.candidate = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHistory {
    entries: Vec<String>,
    nav: Option<HistoryNav>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryNav {
    index: usize,
    draft: String,
}

impl InputHistory {
    pub fn new(entries: Vec<String>) -> Self {
        let mut history = Self {
            entries: Vec::new(),
            nav: None,
        };
        for entry in entries {
            history.push(entry);
        }
        history
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn push(&mut self, prompt: String) {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.entries.retain(|entry| entry != &prompt);
        self.entries.push(prompt);
        if self.entries.len() > HISTORY_LIMIT {
            let excess = self.entries.len() - HISTORY_LIMIT;
            self.entries.drain(0..excess);
        }
        self.nav = None;
    }

    pub fn previous(&mut self, current: String) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let nav = match self.nav.as_mut() {
            Some(nav) => {
                nav.index = nav.index.saturating_sub(1);
                nav
            }
            None => {
                self.nav = Some(HistoryNav {
                    index: self.entries.len() - 1,
                    draft: current,
                });
                self.nav.as_mut().expect("nav set")
            }
        };
        self.entries.get(nav.index).cloned()
    }

    pub fn next(&mut self, _current: String) -> Option<String> {
        let nav = self.nav.as_mut()?;
        if nav.index + 1 < self.entries.len() {
            nav.index += 1;
            return self.entries.get(nav.index).cloned();
        }
        let draft = nav.draft.clone();
        self.nav = None;
        Some(draft)
    }

    pub fn reset_navigation(&mut self) {
        self.nav = None;
    }
}

pub fn load_history(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&content).unwrap_or_default()
}

pub fn save_history(path: &Path, entries: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(entries).unwrap_or_else(|_| "[]".to_string());
    fs::write(&tmp, content)?;
    fs::rename(tmp, path)
}

fn previous_boundary(text: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    text[..cursor].char_indices().last().map(|(idx, _)| idx)
}

fn next_boundary(text: &str, cursor: usize) -> Option<usize> {
    if cursor >= text.len() {
        return None;
    }
    text[cursor..]
        .char_indices()
        .nth(1)
        .map(|(idx, _)| cursor + idx)
        .or(Some(text.len()))
}

fn clamp_boundary(text: &str, target: usize) -> usize {
    if target >= text.len() {
        return text.len();
    }
    let mut cursor = target;
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

fn wrap_rows(text: &str, cursor: usize, width: usize) -> (Vec<String>, usize, usize) {
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut row_width = 0;
    let mut cursor_row = 0;
    let mut cursor_col = 0;
    let mut byte_idx = 0;
    let mut cursor_seen = false;

    if cursor == 0 {
        cursor_seen = true;
    }

    for ch in text.chars() {
        if !cursor_seen && byte_idx == cursor {
            cursor_seen = true;
            cursor_row = rows.len();
            cursor_col = row_width;
        }

        if ch == '\n' {
            rows.push(row);
            row = String::new();
            row_width = 0;
            byte_idx += ch.len_utf8();
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if row_width > 0 && row_width + ch_width > width {
            rows.push(row);
            row = String::new();
            row_width = 0;
        }
        row.push(ch);
        row_width += ch_width;
        byte_idx += ch.len_utf8();
    }

    if !cursor_seen {
        cursor_row = rows.len();
        cursor_col = row_width;
    }
    rows.push(row);
    if cursor_seen && cursor == text.len() && text.ends_with('\n') {
        cursor_row = rows.len() - 1;
        cursor_col = 0;
    } else if cursor_seen && cursor == text.len() {
        cursor_row = rows.len() - 1;
        cursor_col = UnicodeWidthStr::width(rows.last().map(String::as_str).unwrap_or_default());
    }
    (rows, cursor_row, cursor_col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn t(offset_ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(offset_ms)
    }

    #[test]
    fn editor_inserts_moves_and_deletes() {
        let mut editor = TextEditor::new();
        editor.insert_str("helo");
        editor.move_left(false);
        editor.insert_char('l');
        assert_eq!(editor.text(), "hello");
        editor.move_left(false);
        editor.backspace();
        assert_eq!(editor.text(), "helo");
        editor.move_line_end(false);
        editor.move_left(false);
        editor.delete();
        assert_eq!(editor.text(), "hel");
    }

    #[test]
    fn editor_selects_and_replaces_text() {
        let mut editor = TextEditor::new();
        editor.insert_str("abcd");
        editor.move_left(true);
        editor.move_left(true);
        assert_eq!(editor.selection_range(), Some(2..4));
        editor.insert_str("XY");
        assert_eq!(editor.text(), "abXY");
        assert_eq!(editor.cursor(), 4);
    }

    #[test]
    fn editor_moves_vertically_across_lines() {
        let mut editor = TextEditor::new();
        editor.insert_str("one\ntwo\nthree");
        editor.move_up(false);
        editor.move_up(false);
        editor.insert_char('!');
        assert_eq!(editor.text(), "one!\ntwo\nthree");
    }

    #[test]
    fn explicit_newline_and_ctrl_j_path_insert_newline() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_char('a', t(0));
        composer.insert_explicit_newline(t(100));
        composer.insert_char('b', t(200));
        composer.flush_pending_paste_force();
        assert_eq!(composer.text(), "a\nb");
    }

    #[test]
    fn enter_submits_after_pending_paste_flush() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_char('h', t(0));
        composer.flush_pending_paste(t(100));
        assert_eq!(composer.enter(t(120)), EnterDecision::Submit);
        assert_eq!(composer.submit_text().as_deref(), Some("h"));
        assert_eq!(composer.text(), "");
    }

    #[test]
    fn rapid_multiline_paste_buffers_without_submit() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_char('a', t(0));
        composer.insert_char('b', t(1));
        assert_eq!(composer.enter(t(2)), EnterDecision::InsertedNewline);
        composer.insert_char('c', t(3));
        assert_eq!(composer.enter(t(4)), EnterDecision::InsertedNewline);
        composer.insert_char('d', t(5));
        assert_eq!(composer.text(), "");
        assert!(composer.flush_pending_paste(t(80)));
        assert_eq!(composer.text(), "ab\nc\nd");
    }

    #[test]
    fn bracketed_paste_inserts_text_directly() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_paste("one\ntwo\n");
        assert_eq!(composer.text(), "one\ntwo\n");
    }

    #[test]
    fn insert_text_flushes_paste_and_inserts_at_cursor() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_paste("ab");
        composer.move_left(false);
        composer.insert_text("@src/lib.rs ");
        assert_eq!(composer.text(), "a@src/lib.rs b");
    }

    #[test]
    fn replace_current_token_inserts_file_mention() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_paste("read @sr");
        composer.replace_current_token('@', "@src/lib.rs ");
        assert_eq!(composer.text(), "read @src/lib.rs ");
        assert_eq!(composer.cursor_char_offset(), 17);
    }

    #[test]
    fn history_dedupes_and_restores_draft() {
        let mut history = InputHistory::new(vec!["one".to_string(), "two".to_string()]);
        history.push("one".to_string());
        assert_eq!(history.entries(), &["two".to_string(), "one".to_string()]);
        assert_eq!(
            history.previous("draft".to_string()).as_deref(),
            Some("one")
        );
        assert_eq!(history.previous("one".to_string()).as_deref(), Some("two"));
        assert_eq!(history.next("two".to_string()).as_deref(), Some("one"));
        assert_eq!(history.next("one".to_string()).as_deref(), Some("draft"));
    }

    #[test]
    fn composer_history_navigation_preserves_draft() {
        let mut composer = ComposerState::new(vec!["first".to_string(), "second".to_string()]);
        composer.set_text("draft");
        composer.move_up_or_history(false);
        assert_eq!(composer.text(), "second");
        composer.move_up_or_history(false);
        assert_eq!(composer.text(), "first");
        composer.move_down_or_history(false);
        assert_eq!(composer.text(), "second");
        composer.move_down_or_history(false);
        assert_eq!(composer.text(), "draft");
    }

    #[test]
    fn view_caps_rows_and_tracks_cursor() {
        let mut composer = ComposerState::new(Vec::new());
        composer.insert_paste("1\n2\n3\n4\n5");
        let view = composer.view(20, 3);
        assert_eq!(view.lines, vec!["3", "4", "5"]);
        assert_eq!(view.cursor_row, 2);
        assert_eq!(view.cursor_col, 1);
    }

    #[test]
    fn history_persists_multiline_prompts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        let entries = vec!["one".to_string(), "multi\nline".to_string()];
        save_history(&path, &entries).unwrap();
        assert_eq!(load_history(&path), entries);
    }
}
