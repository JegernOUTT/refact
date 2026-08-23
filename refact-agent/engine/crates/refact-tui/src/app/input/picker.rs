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
                if self
                    .modal_picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == PickerKind::SlashCommand)
                {
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
                    if self
                        .modal_picker
                        .as_ref()
                        .is_some_and(|picker| picker.kind == PickerKind::SlashCommand)
                    {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_modes_are_distinct() {
        assert_ne!(ComposerMode::Chat, ComposerMode::ProjectPicker);
    }
}
