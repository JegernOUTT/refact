use super::super::*;

impl App {
    pub(super) fn handle_project_picker_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::ProjectPicker, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                self.composer_mode = ComposerMode::Chat;
                AppAction::None
            }
            Some(KeyAction::Accept) => self
                .picker
                .selected_project()
                .map(|project| AppAction::OpenProject(project.root))
                .unwrap_or(AppAction::None),
            Some(KeyAction::MoveUp) => {
                self.picker.selected = self.picker.selected.saturating_sub(1);
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                self.picker.selected = self.picker.selected.saturating_add(1);
                self.picker.clamp_selection();
                AppAction::None
            }
            Some(KeyAction::Backspace) => {
                self.picker.filter.pop();
                self.picker.clamp_selection();
                AppAction::None
            }
            None => {
                if let Some(ch) = dispatch.text {
                    self.picker.filter.push(ch);
                    self.picker.selected = 0;
                    self.picker.clamp_selection();
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    pub(super) fn handle_project_picker_paste(&mut self, text: &str) {
        self.picker.filter.push_str(text);
        self.picker.selected = 0;
        self.picker.clamp_selection();
    }

    pub(super) fn handle_modal_picker_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::ModalPicker, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                self.cancel_modal_picker();
                AppAction::None
            }
            Some(KeyAction::Accept) => {
                let Some(picker) = self.modal_picker.take() else {
                    return AppAction::None;
                };
                self.accept_modal_picker(picker.kind, picker.accept())
            }
            Some(KeyAction::MoveUp) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_prev();
                }
                self.preview_current_theme_picker_selection();
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_next();
                }
                self.preview_current_theme_picker_selection();
                AppAction::None
            }
            Some(KeyAction::MoveHome) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_first();
                }
                self.preview_current_theme_picker_selection();
                AppAction::None
            }
            Some(KeyAction::MoveEnd) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_last();
                }
                self.preview_current_theme_picker_selection();
                AppAction::None
            }
            Some(KeyAction::ScrollPageUp) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_page_up();
                }
                self.preview_current_theme_picker_selection();
                AppAction::None
            }
            Some(KeyAction::ScrollPageDown) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_page_down();
                }
                self.preview_current_theme_picker_selection();
                AppAction::None
            }
            Some(KeyAction::ToggleSelectedTool) => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    if picker.is_multi() {
                        picker.toggle_selected();
                    } else if let Some(ch) = dispatch.text {
                        self.composer.insert_char(ch, Instant::now());
                        self.update_slash_picker_filter();
                    }
                }
                AppAction::None
            }
            Some(KeyAction::Backspace) => {
                if self.slash_picker_tracks_composer() {
                    self.composer.backspace();
                    self.update_slash_picker_filter();
                } else if let Some(picker) = self.modal_picker.as_mut() {
                    picker.pop_filter();
                    self.preview_current_theme_picker_selection();
                }
                AppAction::None
            }
            None => {
                if let Some(ch) = dispatch.text {
                    if self.slash_picker_tracks_composer() {
                        self.composer.insert_char(ch, Instant::now());
                        self.update_slash_picker_filter();
                    } else if let Some(picker) = self.modal_picker.as_mut() {
                        picker.push_filter(ch);
                        self.preview_current_theme_picker_selection();
                    }
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    pub(super) fn handle_modal_picker_paste(&mut self, text: &str) {
        if self.slash_picker_tracks_composer() {
            self.composer.insert_paste(text);
            self.update_slash_picker_filter();
        } else if let Some(picker) = self.modal_picker.as_mut() {
            picker.push_filter_text(text);
        }
    }

    fn slash_picker_tracks_composer(&self) -> bool {
        self.modal_picker
            .as_ref()
            .is_some_and(|picker| picker.kind == PickerKind::SlashCommand)
            && self.composer.text().starts_with('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_modes_are_distinct() {
        assert_ne!(ComposerMode::Chat, ComposerMode::ProjectPicker);
    }

    #[test]
    fn modal_picker_home_end_and_paging_change_selection() {
        let mut app = App::notice_only("test");
        app.modal_picker = Some(PickerState::new(
            PickerKind::Model,
            (0..25)
                .map(|index| PickerItem {
                    id: index.to_string(),
                    title: index.to_string(),
                    description: String::new(),
                })
                .collect(),
        ));

        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::empty()));
        assert_eq!(app.modal_picker().unwrap().selected, 24);
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()));
        assert_eq!(app.modal_picker().unwrap().selected, 14);
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::empty()));
        assert_eq!(app.modal_picker().unwrap().selected, 0);
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()));
        assert_eq!(app.modal_picker().unwrap().selected, 10);
    }

    #[test]
    fn slash_picker_closes_after_backspacing_past_the_leading_slash() {
        let mut app = App::notice_only("test");
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()));
        for ch in "mod".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()));
        }
        assert_eq!(app.composer(), "/mod");
        assert_eq!(
            app.modal_picker().map(|picker| picker.kind),
            Some(PickerKind::SlashCommand)
        );

        for _ in 0..4 {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
        }
        assert_eq!(app.composer(), "");
        assert!(app.modal_picker().is_none());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            AppAction::None
        );

        app.composer.set_text("/events");
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            AppAction::RefreshWorkers
        );
    }
}
