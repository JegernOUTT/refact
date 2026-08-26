use super::*;
use crate::ui::SurfaceLayer;

mod approval;
mod ask;
mod board;
mod browser;
mod history;
mod main;
mod overlay;
mod picker;
mod settings;
mod vim;
mod worktree;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind != KeyEventKind::Press {
            return AppAction::None;
        }
        if is_ctrl_c_key(key) {
            return self.ctrl_c_action();
        }
        self.last_ctrl_c = None;
        if self.surface_layer() == SurfaceLayer::Help {
            self.help_open = false;
            return AppAction::None;
        }
        match self.focused_key_context() {
            KeyContext::Activity => return self.handle_activity_key(key),
            KeyContext::Board => return self.handle_task_board_key(key),
            KeyContext::Browser => return self.handle_browser_key(key),
            KeyContext::History if self.history_surface.is_some() => {
                return self.handle_history_surface_key(key);
            }
            KeyContext::Goal => return self.handle_goal_overlay_key(key),
            KeyContext::Overlay | KeyContext::OverlaySearch => {
                return self.handle_transcript_overlay_key(key);
            }
            KeyContext::Approval => return self.handle_approval_key(key),
            KeyContext::AskForm => return self.handle_ask_questions_key(key),
            KeyContext::Worktree => return self.handle_worktree_key(key),
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
        if self.surface_layer() == SurfaceLayer::Help {
            return;
        }
        match self.focused_key_context() {
            KeyContext::Activity
            | KeyContext::Board
            | KeyContext::Browser
            | KeyContext::Goal
            | KeyContext::Worktree => {}
            KeyContext::Overlay | KeyContext::OverlaySearch => {
                self.handle_transcript_overlay_paste(text)
            }
            KeyContext::Approval => self.handle_approval_paste(text),
            KeyContext::AskForm => self.handle_ask_questions_paste(text),
            KeyContext::ModalPicker => self.handle_modal_picker_paste(text),
            KeyContext::ProjectPicker => self.handle_project_picker_paste(text),
            KeyContext::Settings => self.handle_settings_paste(text),
            KeyContext::History if self.history_surface.is_some() => {
                if let Some(history) = self.history_surface.as_mut() {
                    if history.filter_active() {
                        for ch in text.chars() {
                            history.push_filter(ch);
                        }
                    }
                }
            }
            KeyContext::History => {
                for ch in text.chars() {
                    self.composer.history_search_insert_char(ch);
                }
            }
            _ => self.composer.insert_paste(text),
        }
    }

    pub(super) fn focused_key_context(&self) -> KeyContext {
        match self.surface_layer() {
            SurfaceLayer::Activity => return KeyContext::Activity,
            SurfaceLayer::Board => return KeyContext::Board,
            SurfaceLayer::Browser => return KeyContext::Browser,
            SurfaceLayer::History => return KeyContext::History,
            SurfaceLayer::Overlay => {
                return if self
                    .transcript_overlay
                    .as_ref()
                    .is_some_and(|overlay| overlay.search_input().is_some())
                {
                    KeyContext::OverlaySearch
                } else {
                    KeyContext::Overlay
                };
            }
            SurfaceLayer::Goal => return KeyContext::Goal,
            SurfaceLayer::Approval => return KeyContext::Approval,
            SurfaceLayer::AskForm => return KeyContext::AskForm,
            SurfaceLayer::Worktree => return KeyContext::Worktree,
            SurfaceLayer::ModalPicker => return KeyContext::ModalPicker,
            SurfaceLayer::Settings => return KeyContext::Settings,
            SurfaceLayer::ProjectPicker => return KeyContext::ProjectPicker,
            SurfaceLayer::Main | SurfaceLayer::Help => {}
        }
        if self.composer.history_search_active() {
            return KeyContext::History;
        }
        if self.transcript_cell_context_active() {
            return KeyContext::TranscriptCell;
        }
        KeyContext::Main
    }

    pub(crate) fn surface_layer(&self) -> SurfaceLayer {
        if self.help_open {
            SurfaceLayer::Help
        } else if self.pending_worktree_merge.is_some() {
            SurfaceLayer::Worktree
        } else if self.approval_modal().is_some() {
            SurfaceLayer::Approval
        } else if self.goal_overlay_open() {
            SurfaceLayer::Goal
        } else if self.modal_picker.is_some() {
            SurfaceLayer::ModalPicker
        } else if self.settings_surface_open() {
            SurfaceLayer::Settings
        } else if self.composer_mode == ComposerMode::ProjectPicker {
            SurfaceLayer::ProjectPicker
        } else if self.transcript_overlay.is_some() && self.activity_surface.is_none() {
            SurfaceLayer::Overlay
        } else if self.history_surface.is_some() {
            SurfaceLayer::History
        } else if self.board_surface.is_some() {
            SurfaceLayer::Board
        } else if self.browser_surface.is_some() {
            SurfaceLayer::Browser
        } else if self.activity_surface.is_some() {
            SurfaceLayer::Activity
        } else if self.ask_questions_form.is_some() {
            SurfaceLayer::AskForm
        } else {
            SurfaceLayer::Main
        }
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
    use crate::client::{
        TaskBoardCard, TaskBoardReadyCards, TaskBoardResponse, TaskBoardTask, TaskBoardViewData,
    };
    use crate::pickers::{PickerItem, PickerKind, PickerState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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

    fn approval() -> ApprovalModalState {
        ApprovalModalState::new(vec![PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: "echo test".to_string(),
            rule: "default".to_string(),
            tool_call_id: "call-approval".to_string(),
            integr_config_path: None,
            args: None,
            diff: None,
        }])
    }

    fn board_data() -> TaskBoardViewData {
        TaskBoardViewData {
            task: TaskBoardTask {
                id: "task-1".to_string(),
                name: "Board".to_string(),
                status: "active".to_string(),
            },
            board: TaskBoardResponse {
                cards: vec![TaskBoardCard {
                    id: "T-1".to_string(),
                    title: "Board card".to_string(),
                    column: "planned".to_string(),
                    priority: "P1".to_string(),
                    ..TaskBoardCard::default()
                }],
                ..TaskBoardResponse::default()
            },
            ready: TaskBoardReadyCards::default(),
        }
    }

    fn assert_approval_preempts_surface(
        mut app: App,
        surface: SurfaceLayer,
        close_surface: impl FnOnce(&mut App),
    ) {
        assert_eq!(app.surface_layer(), surface);
        app.test_set_approval(approval());

        assert_eq!(app.surface_layer(), SurfaceLayer::Approval);
        assert_eq!(app.focused_key_context(), KeyContext::Approval);
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty())),
            AppAction::SendToolDecisions { .. }
        ));
        assert_eq!(app.surface_layer(), surface);
        close_surface(&mut app);
    }

    fn assert_approval_renders_over(mut app: App) {
        app.test_set_approval(approval());
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Approval required"));
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

    #[test]
    fn approval_preempts_and_then_releases_each_exclusive_surface() {
        let mut history = App::notice_only("test");
        history.test_open_history_surface(Vec::new());
        let mut history_render = App::notice_only("test");
        history_render.test_open_history_surface(Vec::new());
        assert_approval_renders_over(history_render);
        assert_approval_preempts_surface(history, SurfaceLayer::History, |app| {
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            assert_eq!(app.surface_layer(), SurfaceLayer::Main);
        });

        let mut board = App::notice_only("test");
        board.test_show_task_board(board_data());
        let mut board_render = App::notice_only("test");
        board_render.test_show_task_board(board_data());
        assert_approval_renders_over(board_render);
        assert_approval_preempts_surface(board, SurfaceLayer::Board, |app| {
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            assert_eq!(app.surface_layer(), SurfaceLayer::Main);
        });

        let mut browser = App::notice_only("test");
        browser.browser_surface = Some(surfaces::browser::BrowserSurface::new());
        let mut browser_render = App::notice_only("test");
        browser_render.browser_surface = Some(surfaces::browser::BrowserSurface::new());
        assert_approval_renders_over(browser_render);
        assert_approval_preempts_surface(browser, SurfaceLayer::Browser, |app| {
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            assert_eq!(app.surface_layer(), SurfaceLayer::Main);
        });

        let mut activity = App::notice_only("test");
        activity.activity_surface = Some(surfaces::activity::ActivitySurfaceState::default());
        let mut activity_render = App::notice_only("test");
        activity_render.activity_surface =
            Some(surfaces::activity::ActivitySurfaceState::default());
        assert_approval_renders_over(activity_render);
        assert_approval_preempts_surface(activity, SurfaceLayer::Activity, |app| {
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            assert_eq!(app.surface_layer(), SurfaceLayer::Main);
        });
    }

    #[test]
    fn modal_picker_preempts_settings_for_rendering_and_input() {
        let mut app = App::notice_only("test");
        app.test_open_settings_surface();
        app.modal_picker = Some(PickerState::new(
            PickerKind::Model,
            vec![PickerItem {
                id: "model".to_string(),
                title: "Model".to_string(),
                description: String::new(),
            }],
        ));

        assert_eq!(app.surface_layer(), SurfaceLayer::ModalPicker);
        assert_eq!(app.focused_key_context(), KeyContext::ModalPicker);
        app.handle_paste("model");
        assert_eq!(app.modal_picker().unwrap().filter, "model");
        assert_eq!(app.settings_selected(), 0);

        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("models: model"));
        assert!(!text.contains("Settings · this chat only"));
    }
}
