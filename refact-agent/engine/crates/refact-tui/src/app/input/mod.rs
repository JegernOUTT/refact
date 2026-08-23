use super::*;

mod approval;
mod ask;
mod main;
mod overlay;
mod picker;
mod vim;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
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
        let transcript_cell_active = self.transcript_cell_context_active();
        let main_dispatch = self.keymap.dispatch_main(transcript_cell_active, key);
        if transcript_cell_active && main_dispatch.action == Some(KeyAction::ToggleSelectedTool) {
            self.toggle_selected_tool();
            return AppAction::None;
        }
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

    fn transcript_cell_context_active(&self) -> bool {
        self.composer.is_empty()
            && self.selected_tool_index.is_some_and(|index| {
                matches!(
                    self.transcript.get(index),
                    Some(TranscriptItem::Tool(card)) if card.expanded
                )
            })
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

    #[test]
    fn expanded_selected_tool_uses_transcript_cell_context_before_main() {
        let mut app = App::notice_only("test");
        app.test_push_tool(ToolCard::from_tool_call(
            &serde_json::json!({"id": "call-1", "name": "shell"}),
        ));
        app.toggle_selected_tool();
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Tool(card)) if card.expanded
        ));

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty())),
            AppAction::None
        );
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Tool(card)) if !card.expanded
        ));
        assert_eq!(app.composer(), "");
    }

    #[test]
    fn ctrl_k_opens_command_palette() {
        let mut app = App::notice_only("test");

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert_eq!(
            app.modal_picker().map(|picker| picker.kind),
            Some(PickerKind::SlashCommand)
        );
    }
}
