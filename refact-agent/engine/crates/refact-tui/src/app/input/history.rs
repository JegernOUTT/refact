use super::super::*;

impl App {
    pub(super) fn handle_history_surface_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::History, key);
        if self
            .history_surface
            .as_ref()
            .is_some_and(surfaces::HistorySurface::filter_active)
        {
            match dispatch.action {
                Some(KeyAction::Cancel | KeyAction::Accept) => {
                    if let Some(history) = self.history_surface.as_mut() {
                        history.cancel_filter();
                    }
                }
                Some(KeyAction::Backspace) => {
                    if let Some(history) = self.history_surface.as_mut() {
                        history.pop_filter();
                    }
                }
                _ => {
                    if let Some(ch) = dispatch.text {
                        if let Some(history) = self.history_surface.as_mut() {
                            history.push_filter(ch);
                        }
                    }
                }
            }
            return AppAction::None;
        }
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                if self
                    .history_surface
                    .as_mut()
                    .is_some_and(|history| history.detail_open())
                {
                    if let Some(history) = self.history_surface.as_mut() {
                        history.set_detail_open(false);
                    }
                } else {
                    self.history_surface = None;
                }
                AppAction::None
            }
            Some(KeyAction::Accept) => self.apply_history_action(surfaces::HistoryAction::Resume),
            Some(KeyAction::MoveUp) => {
                if let Some(history) = self.history_surface.as_mut() {
                    if history.detail_open() {
                        history.scroll_detail_up();
                    } else {
                        history.select_prev();
                    }
                }
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                if let Some(history) = self.history_surface.as_mut() {
                    if history.detail_open() {
                        history.scroll_detail_down();
                    } else {
                        history.select_next();
                    }
                }
                AppAction::None
            }
            Some(KeyAction::MoveHome) => {
                if let Some(history) = self.history_surface.as_mut() {
                    history.select_first();
                }
                AppAction::None
            }
            Some(KeyAction::MoveEnd) => {
                if let Some(history) = self.history_surface.as_mut() {
                    history.select_last();
                }
                AppAction::None
            }
            Some(KeyAction::ScrollPageUp) => {
                if let Some(history) = self.history_surface.as_mut() {
                    if history.detail_open() {
                        for _ in 0..10 {
                            history.scroll_detail_up();
                        }
                    } else {
                        history.select_page_up();
                    }
                }
                AppAction::None
            }
            Some(KeyAction::ScrollPageDown) => {
                if let Some(history) = self.history_surface.as_mut() {
                    if history.detail_open() {
                        for _ in 0..10 {
                            history.scroll_detail_down();
                        }
                    } else {
                        history.select_page_down();
                    }
                }
                AppAction::None
            }
            Some(KeyAction::Backspace) => {
                if let Some(history) = self.history_surface.as_mut() {
                    history.pop_filter();
                }
                AppAction::None
            }
            Some(_) | None => match dispatch.text {
                Some('/') => {
                    if let Some(history) = self.history_surface.as_mut() {
                        history.begin_filter();
                    }
                    AppAction::None
                }
                Some('g') => {
                    if let Some(history) = self.history_surface.as_mut() {
                        history.cycle_grouping();
                    }
                    AppAction::None
                }
                Some('t') => {
                    if let Some(history) = self.history_surface.as_mut() {
                        history.toggle_detail();
                    }
                    AppAction::None
                }
                Some('l') => {
                    if self
                        .history_surface
                        .as_mut()
                        .is_some_and(surfaces::HistorySurface::navigate_to_parent)
                    {
                        AppAction::None
                    } else {
                        self.add_notice("Selected chat has no rendered parent relationship");
                        AppAction::None
                    }
                }
                Some('R') => {
                    if self
                        .history_surface
                        .as_mut()
                        .is_some_and(surfaces::HistorySurface::navigate_to_root)
                    {
                        AppAction::None
                    } else {
                        self.add_notice("Selected chat has no rendered root relationship");
                        AppAction::None
                    }
                }
                Some('f') => self.apply_history_action(surfaces::HistoryAction::Fork),
                Some('r') => self.apply_history_action(surfaces::HistoryAction::Rename),
                Some('a') => self.apply_history_action(surfaces::HistoryAction::Archive),
                Some(_) | None => AppAction::None,
            },
        }
    }

    fn apply_history_action(&mut self, action: surfaces::HistoryAction) -> AppAction {
        let Some(request) = self
            .history_surface
            .as_ref()
            .and_then(|history| history.selected_action(action))
        else {
            self.add_notice("No chat selected in history");
            return AppAction::None;
        };
        match request.action {
            surfaces::HistoryAction::Resume => {
                let title = self
                    .history_surface
                    .as_ref()
                    .and_then(|history| history.selected_trajectory())
                    .map(|trajectory| crate::sessions::display_title(&trajectory.title))
                    .unwrap_or_else(|| "Untitled chat".to_string());
                self.history_surface = None;
                self.resume_chat(request.chat_id, title, None)
            }
            surfaces::HistoryAction::Fork => {
                if request.chat_id == self.chat_id {
                    self.history_surface = None;
                    self.fork_chat()
                } else {
                    self.add_notice("Resume the selected chat before forking it");
                    AppAction::None
                }
            }
            surfaces::HistoryAction::Rename => {
                if request.chat_id == self.chat_id {
                    self.history_surface = None;
                    self.add_notice("Type the new title, then use /rename");
                } else {
                    self.add_notice("Resume the selected chat before renaming it");
                }
                AppAction::None
            }
            surfaces::HistoryAction::Archive => {
                if request.chat_id == self.chat_id {
                    self.history_surface = None;
                    self.archive_chat()
                } else {
                    self.add_notice("Resume the selected chat before archiving it");
                    AppAction::None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::OpenProjectResponse;
    use crate::sessions::TrajectoryMeta;
    use std::path::PathBuf;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "project".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: None,
            worker: None,
            cron_pending: None,
        }
    }

    fn trajectory(id: &str, title: &str) -> TrajectoryMeta {
        TrajectoryMeta {
            id: id.to_string(),
            title: title.to_string(),
            updated_at: "2026-08-25T10:00:00Z".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn history_context_routes_navigation_filter_and_resume() {
        let mut app = App::new(project());
        app.history_surface = Some(surfaces::HistorySurface::new(vec![
            trajectory("chat-b", "Beta"),
            trajectory("chat-a", "Alpha"),
        ]));

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()));
        app.handle_paste("a");
        assert_eq!(app.history_surface().unwrap().filter(), "a");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
        assert_eq!(app.history_surface().unwrap().filter(), "");
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));

        assert!(matches!(action, AppAction::SubscribeCurrent));
        assert!(app.history_surface().is_none());
        assert_eq!(app.chat_id(), "chat-a");
    }

    #[test]
    fn history_filter_requires_slash_and_accepts_text() {
        let mut app = App::new(project());
        app.history_surface = Some(surfaces::HistorySurface::new(vec![trajectory(
            "chat-a", "Alpha",
        )]));

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        assert_eq!(app.history_surface().unwrap().filter(), "");
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()));
        assert!(app.history_surface().unwrap().filter_active());
        app.handle_paste("alpha");
        assert_eq!(app.history_surface().unwrap().filter(), "alpha");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        assert!(!app.history_surface().unwrap().filter_active());
    }

    #[test]
    fn history_actions_preserve_non_current_chat_safety() {
        let mut app = App::new(project());
        app.history_surface = Some(surfaces::HistorySurface::new(vec![trajectory(
            "other", "Other",
        )]));

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::empty())),
            AppAction::None
        );
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| matches!(item, TranscriptItem::Notice(text) if text.contains("Resume the selected chat"))));
    }
}
