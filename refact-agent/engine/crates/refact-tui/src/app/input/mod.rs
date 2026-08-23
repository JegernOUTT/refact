use super::*;

mod approval;
mod ask;
mod main;
mod overlay;
mod picker;
mod vim;

impl App {
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind != KeyEventKind::Press {
            return AppAction::None;
        }
        if is_ctrl_c_key(key) {
            return self.ctrl_c_action();
        }
        self.last_ctrl_c = None;
        if self.help_open {
            self.help_open = false;
            return AppAction::None;
        }
        if self.transcript_overlay.is_some() {
            return self.handle_transcript_overlay_key(key);
        }
        if self.approval_modal().is_some() {
            return self.handle_approval_key(key);
        }
        if self.ask_questions_form.is_some() {
            return self.handle_ask_questions_key(key);
        }
        if self.modal_picker.is_some() {
            return self.handle_modal_picker_key(key);
        }
        if self.composer_mode == ComposerMode::ProjectPicker {
            return self.handle_project_picker_key(key);
        }
        if let Some(action) = self.handle_history_search_key(key) {
            return action;
        }
        let main_dispatch = self.keymap.dispatch(KeyContext::Main, key);
        if matches!(
            main_dispatch.action,
            Some(KeyAction::ShowHelp | KeyAction::ToggleVimMode)
        ) {
            return self.handle_main_dispatch(main_dispatch, key);
        }
        if let Some(action) = self.handle_vim_key(key) {
            return action;
        }
        self.handle_main_dispatch(main_dispatch, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_ctrl_c() {
        assert!(is_ctrl_c_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
    }
}
