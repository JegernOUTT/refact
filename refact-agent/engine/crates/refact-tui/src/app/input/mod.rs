use super::*;

mod approval;
mod ask;
mod board;
mod main;
mod overlay;
mod picker;
mod settings;
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
        if self.activity_surface.is_some() {
            return self.handle_activity_key(key);
        }
        match self.focused_key_context() {
            KeyContext::Board => return self.handle_task_board_key(key),
            KeyContext::Overlay | KeyContext::OverlaySearch => {
                return self.handle_transcript_overlay_key(key);
            }
            KeyContext::Approval => return self.handle_approval_key(key),
            KeyContext::AskForm => return self.handle_ask_questions_key(key),
            KeyContext::ModalPicker => return self.handle_modal_picker_key(key),
            KeyContext::Settings => return self.handle_settings_key(key),
            KeyContext::ProjectPicker => return self.handle_project_picker_key(key),
            _ => {}
        }
        if let Some(action) = self.handle_history_search_key(key) {
            return action;
        }
        let transcript_cell_active = self.focused_key_context() == KeyContext::TranscriptCell;
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

    pub(super) fn handle_paste(&mut self, text: &str) {
        match self.focused_key_context() {
            KeyContext::Activity => {}
            KeyContext::Overlay | KeyContext::OverlaySearch => {
                self.handle_transcript_overlay_paste(text)
            }
            KeyContext::Board => {}
            KeyContext::Approval => self.handle_approval_paste(text),
            KeyContext::AskForm => self.handle_ask_questions_paste(text),
            KeyContext::ModalPicker => self.handle_modal_picker_paste(text),
            KeyContext::ProjectPicker => self.handle_project_picker_paste(text),
            KeyContext::Settings => self.handle_settings_paste(text),
            KeyContext::History => {
                for ch in text.chars() {
                    self.composer.history_search_insert_char(ch);
                }
            }
            _ => self.composer.insert_paste(text),
        }
    }

    pub(super) fn focused_key_context(&self) -> KeyContext {
        if self.activity_surface.is_some() {
            return KeyContext::Activity;
        }
        if self.board_surface.is_some() {
            return KeyContext::Board;
        }
        if let Some(overlay) = self.transcript_overlay.as_ref() {
            return if overlay.search_input().is_some() {
                KeyContext::OverlaySearch
            } else {
                KeyContext::Overlay
            };
        }
        if self.approval_modal().is_some() {
            return KeyContext::Approval;
        }
        if self.ask_questions_form.is_some() {
            return KeyContext::AskForm;
        }
        if self.modal_picker.is_some() {
            return KeyContext::ModalPicker;
        }
        if self.settings_surface.is_some() {
            return KeyContext::Settings;
        }
        if self.composer_mode == ComposerMode::ProjectPicker {
            return KeyContext::ProjectPicker;
        }
        if self.composer.history_search_active() {
            return KeyContext::History;
        }
        if self.transcript_cell_context_active() {
            return KeyContext::TranscriptCell;
        }
        KeyContext::Main
    }

    fn handle_activity_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Activity, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                self.activity_surface = None;
                self.transcript_overlay = None;
                AppAction::None
            }
            Some(KeyAction::MoveUp) => {
                self.move_activity_selection(-1);
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                self.move_activity_selection(1);
                AppAction::None
            }
            Some(KeyAction::Accept) => self.open_selected_activity_agent(),
            _ => AppAction::None,
        }
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
    use crate::approvals::PauseReason;

    fn free_text_form() -> AskQuestionsForm {
        let request = AskQuestionsRequest::from_tool_content(
            &serde_json::json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": [{"id": "notes", "type": "free_text", "text": "Notes?"}],
            })
            .to_string(),
            None,
        )
        .expect("valid ask form");
        AskQuestionsForm::new(request)
    }

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

    #[test]
    fn paste_routes_to_the_focused_consumer() {
        let mut ask = App::notice_only("test");
        ask.test_set_ask_questions_form(free_text_form());
        assert_eq!(ask.focused_key_context(), KeyContext::AskForm);
        ask.handle_paste("answer");
        assert_eq!(
            ask.ask_questions_form().unwrap().current_text(),
            Some("answer")
        );
        assert_eq!(ask.composer(), "");

        let mut picker = App::notice_only("test");
        picker.modal_picker = Some(PickerState::new(
            PickerKind::Model,
            vec![PickerItem {
                id: "model".to_string(),
                title: "Model".to_string(),
                description: String::new(),
            }],
        ));
        assert_eq!(picker.focused_key_context(), KeyContext::ModalPicker);
        picker.handle_paste("model");
        assert_eq!(picker.modal_picker().unwrap().filter, "model");
        assert_eq!(picker.composer(), "");

        let mut overlay = App::notice_only("test");
        overlay.open_transcript_overlay();
        overlay.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()));
        assert_eq!(overlay.focused_key_context(), KeyContext::OverlaySearch);
        overlay.handle_paste("query");
        assert_eq!(
            overlay.transcript_overlay().unwrap().search_input(),
            Some("query")
        );
        assert_eq!(overlay.composer(), "");

        let mut approval = App::notice_only("test");
        approval.composer.set_text("draft");
        approval.test_set_approval(ApprovalModalState::new(vec![PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: "echo test".to_string(),
            rule: "default".to_string(),
            tool_call_id: "call-approval".to_string(),
            integr_config_path: None,
            args: None,
            diff: None,
        }]));
        assert_eq!(approval.focused_key_context(), KeyContext::Approval);
        approval.handle_paste("ignored");
        assert_eq!(approval.composer(), "draft");

        let mut history = App::notice_only("test");
        history.composer.start_or_cycle_history_search();
        assert_eq!(history.focused_key_context(), KeyContext::History);
        history.handle_paste("past");
        assert_eq!(history.composer_history_search().unwrap().query, "past");
        assert_eq!(history.composer(), "");
    }
}
