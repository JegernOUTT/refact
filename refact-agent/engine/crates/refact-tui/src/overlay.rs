use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerMode {
    Rendered,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerAction {
    None,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagerOverlay {
    title: String,
    rendered_lines: Vec<String>,
    raw_lines: Vec<String>,
    mode: PagerMode,
    scroll: usize,
    search_input: Option<String>,
    query: String,
    matches: Vec<usize>,
    active_match: usize,
}

impl PagerOverlay {
    pub fn new(
        title: impl Into<String>,
        rendered_lines: Vec<String>,
        raw_lines: Vec<String>,
    ) -> Self {
        Self {
            title: title.into(),
            rendered_lines,
            raw_lines,
            mode: PagerMode::Rendered,
            scroll: 0,
            search_input: None,
            query: String::new(),
            matches: Vec::new(),
            active_match: 0,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn mode(&self) -> PagerMode {
        self.mode
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn search_input(&self) -> Option<&str> {
        self.search_input.as_deref()
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn active_match_line(&self) -> Option<usize> {
        self.matches.get(self.active_match).copied()
    }

    pub fn is_copy_mode(&self) -> bool {
        self.mode == PagerMode::Raw
    }

    pub fn lines(&self) -> &[String] {
        match self.mode {
            PagerMode::Rendered => &self.rendered_lines,
            PagerMode::Raw => &self.raw_lines,
        }
    }

    pub fn visible_lines(&self, height: usize) -> Vec<String> {
        if height == 0 {
            return Vec::new();
        }
        let lines = self.lines();
        let start = self.scroll.min(lines.len());
        let end = start.saturating_add(height).min(lines.len());
        lines[start..end].to_vec()
    }

    pub fn status(&self) -> String {
        let mode = match self.mode {
            PagerMode::Rendered => "rendered",
            PagerMode::Raw => "copy/raw",
        };
        let search = if let Some(input) = &self.search_input {
            format!("search: /{input}")
        } else if self.query.is_empty() {
            "search: /".to_string()
        } else if self.matches.is_empty() {
            format!("/{}, no matches", self.query)
        } else {
            format!(
                "/{} {}/{}",
                self.query,
                self.active_match.saturating_add(1),
                self.matches.len()
            )
        };
        format!("{mode} · {search} · c copy mode · q/Esc close")
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PagerAction {
        if key.kind != KeyEventKind::Press {
            return PagerAction::None;
        }
        if self.search_input.is_some() {
            return self.handle_search_key(key);
        }
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => PagerAction::Close,
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers,
                ..
            } if modifiers.is_empty() => PagerAction::Close,
            KeyEvent {
                code: KeyCode::Char('/'),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                self.search_input = Some(String::new());
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                self.toggle_mode();
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Char('n'),
                modifiers,
                ..
            } if modifiers.is_empty() => {
                self.next_match();
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Char('N'),
                modifiers,
                ..
            } if modifiers == KeyModifiers::SHIFT || modifiers.is_empty() => {
                self.prev_match();
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.scroll = self.scroll.saturating_add(1).min(self.lines().len());
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.scroll = self.scroll.saturating_sub(1);
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => {
                self.scroll = self.scroll.saturating_add(10).min(self.lines().len());
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => {
                self.scroll = self.scroll.saturating_sub(10);
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => {
                self.scroll = 0;
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::End, ..
            } => {
                self.scroll = self.lines().len().saturating_sub(1);
                PagerAction::None
            }
            _ => PagerAction::None,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> PagerAction {
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.search_input = None;
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                let query = self.search_input.take().unwrap_or_default();
                self.apply_search(query);
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                if let Some(input) = self.search_input.as_mut() {
                    input.pop();
                }
                PagerAction::None
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                if let Some(input) = self.search_input.as_mut() {
                    input.push(ch);
                }
                PagerAction::None
            }
            _ => PagerAction::None,
        }
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            PagerMode::Rendered => PagerMode::Raw,
            PagerMode::Raw => PagerMode::Rendered,
        };
        let old_query = self.query.clone();
        self.apply_search(old_query);
    }

    fn apply_search(&mut self, query: String) {
        self.query = query;
        self.matches.clear();
        self.active_match = 0;
        if self.query.is_empty() {
            return;
        }
        let needle = self.query.to_ascii_lowercase();
        self.matches = self
            .lines()
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| line.to_ascii_lowercase().contains(&needle).then_some(idx))
            .collect();
        if let Some(line) = self.matches.first().copied() {
            self.scroll = line;
        }
    }

    fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.active_match = (self.active_match + 1) % self.matches.len();
        self.scroll = self.matches[self.active_match];
    }

    fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.active_match = if self.active_match == 0 {
            self.matches.len() - 1
        } else {
            self.active_match - 1
        };
        self.scroll = self.matches[self.active_match];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn pager_scrolls_searches_and_cycles_matches() {
        let mut pager = PagerOverlay::new(
            "transcript",
            vec!["alpha".into(), "beta".into(), "alphabet".into()],
            vec!["raw alpha".into()],
        );
        pager.handle_key(key(KeyCode::PageDown));
        assert_eq!(pager.scroll(), 3);
        pager.handle_key(key(KeyCode::PageUp));
        assert_eq!(pager.scroll(), 0);
        pager.handle_key(key(KeyCode::Char('/')));
        pager.handle_key(key(KeyCode::Char('a')));
        pager.handle_key(key(KeyCode::Char('l')));
        pager.handle_key(key(KeyCode::Enter));
        assert_eq!(pager.query(), "al");
        assert_eq!(pager.match_count(), 2);
        assert_eq!(pager.active_match_line(), Some(0));
        pager.handle_key(key(KeyCode::Char('n')));
        assert_eq!(pager.active_match_line(), Some(2));
    }

    #[test]
    fn pager_copy_mode_uses_raw_lines_and_closes() {
        let mut pager = PagerOverlay::new(
            "transcript",
            vec!["rendered".into()],
            vec!["raw one".into(), "raw two".into()],
        );
        assert_eq!(pager.mode(), PagerMode::Rendered);
        pager.handle_key(key(KeyCode::Char('c')));
        assert!(pager.is_copy_mode());
        assert_eq!(pager.visible_lines(3), vec!["raw one", "raw two"]);
        assert_eq!(pager.handle_key(key(KeyCode::Esc)), PagerAction::Close);
    }
}
