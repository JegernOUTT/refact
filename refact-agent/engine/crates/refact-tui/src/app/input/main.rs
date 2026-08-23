use super::super::*;

impl App {
    pub(super) fn handle_history_search_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        if !self.composer.history_search_active() {
            return None;
        }
        let dispatch = self.keymap.dispatch(KeyContext::Main, key);
        match dispatch.action {
            Some(KeyAction::HistorySearch) => self.composer.start_or_cycle_history_search(),
            Some(KeyAction::Backspace) => self.composer.history_search_backspace(),
            Some(KeyAction::Accept) => self.composer.accept_history_search(),
            Some(KeyAction::Cancel) => self.composer.cancel_history_search(),
            _ => {
                if let Some(ch) = composer_search_text(key) {
                    self.composer.history_search_insert_char(ch);
                }
            }
        }
        Some(AppAction::None)
    }

    pub(super) fn handle_main_dispatch(
        &mut self,
        dispatch: KeyDispatch,
        key: KeyEvent,
    ) -> AppAction {
        match dispatch.action {
            Some(KeyAction::ShowHelp) => {
                self.help_open = true;
                AppAction::None
            }
            Some(KeyAction::ToggleEvents) => {
                self.events_pane.toggle();
                AppAction::RefreshWorkers
            }
            Some(KeyAction::Quit) => self.quit_action(),
            Some(KeyAction::NewChat) => {
                self.new_chat();
                AppAction::SubscribeCurrent
            }
            Some(KeyAction::PreviousSession) => self.switch_recent_session(-1),
            Some(KeyAction::NextSession) => self.switch_recent_session(1),
            Some(KeyAction::OpenProjects) => AppAction::LoadProjects,
            Some(KeyAction::OpenModels) => AppAction::LoadModels,
            Some(KeyAction::OpenModes) => AppAction::LoadModes,
            Some(KeyAction::OpenTranscriptOverlay) => self.open_transcript_overlay(),
            Some(KeyAction::OpenExternalEditor) => AppAction::OpenExternalEditor {
                draft: self.composer.text().to_string(),
            },
            Some(KeyAction::ToggleReasoning) => {
                self.toggle_reasoning_items();
                AppAction::None
            }
            Some(KeyAction::HistorySearch) => {
                self.input_queue.clear_selection();
                self.composer.start_or_cycle_history_search();
                AppAction::None
            }
            Some(KeyAction::KillToLineEnd) => {
                self.input_queue.clear_selection();
                self.composer.kill_to_line_end();
                AppAction::None
            }
            Some(KeyAction::KillToLineStart) => {
                self.input_queue.clear_selection();
                self.composer.kill_to_line_start();
                AppAction::None
            }
            Some(KeyAction::Yank) => {
                self.input_queue.clear_selection();
                self.composer.yank();
                AppAction::None
            }
            Some(KeyAction::Undo) => {
                self.input_queue.clear_selection();
                self.composer.undo();
                AppAction::None
            }
            Some(KeyAction::Redo) => {
                self.input_queue.clear_selection();
                self.composer.redo();
                AppAction::None
            }
            Some(KeyAction::CtrlC) => self.ctrl_c_action(),
            Some(KeyAction::Cancel) => self.cancel_action(),
            Some(KeyAction::CycleToolSelection) => {
                self.cycle_tool_selection();
                AppAction::None
            }
            Some(KeyAction::ToggleSelectedTool) => {
                if self.composer.is_empty() {
                    self.toggle_selected_tool();
                    AppAction::None
                } else {
                    self.insert_dispatch_text(dispatch)
                }
            }
            Some(KeyAction::OpenSlashCommands) => {
                if self.composer.is_empty() {
                    self.input_queue.clear_selection();
                    self.composer.insert_text("/");
                    self.open_slash_command_picker();
                    AppAction::None
                } else {
                    self.insert_dispatch_text(dispatch)
                }
            }
            Some(KeyAction::OpenFileMention) => {
                self.input_queue.clear_selection();
                self.composer.insert_text("@");
                self.start_file_mention_lookup()
            }
            Some(KeyAction::InsertNewline) => {
                self.composer.insert_explicit_newline(Instant::now());
                AppAction::None
            }
            Some(KeyAction::Accept) => self.accept_action(),
            Some(KeyAction::Backspace) => self.backspace_action(),
            Some(KeyAction::Delete) => self.delete_action(),
            Some(KeyAction::MoveLeft) => {
                self.composer
                    .move_left(key.modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            Some(KeyAction::MoveRight) => {
                self.composer
                    .move_right(key.modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            Some(KeyAction::MoveHome) => {
                self.composer
                    .move_home(key.modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            Some(KeyAction::MoveEnd) => {
                self.composer
                    .move_end(key.modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            Some(KeyAction::MoveUp) => self.move_up_action(key),
            Some(KeyAction::MoveDown) => self.move_down_action(key),
            Some(KeyAction::ScrollPageUp) => {
                self.scroll_offset = self.scroll_offset.saturating_add(5);
                AppAction::None
            }
            Some(KeyAction::ScrollPageDown) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(5);
                AppAction::None
            }
            Some(KeyAction::ToggleVimMode) => {
                let enabled = self.vim.toggle();
                let label = if enabled { "enabled" } else { "disabled" };
                self.add_notice(format!("Composer vim mode {label}"));
                AppAction::None
            }
            Some(_) => AppAction::None,
            None => self.insert_dispatch_text(dispatch),
        }
    }

    pub(super) fn insert_dispatch_text(&mut self, dispatch: KeyDispatch) -> AppAction {
        let Some(ch) = dispatch.text else {
            return AppAction::None;
        };
        if self.selected_backtrack_index.is_some() && self.composer.is_empty() {
            self.clear_backtrack_selection();
        }
        self.input_queue.clear_selection();
        self.composer.insert_char(ch, Instant::now());
        AppAction::None
    }

    pub(super) fn toggle_reasoning_items(&mut self) {
        for item in &mut self.transcript {
            if let TranscriptItem::Reasoning(_, collapsed) = item {
                *collapsed = !*collapsed;
            }
        }
    }

    pub(super) fn cancel_action(&mut self) -> AppAction {
        if self.cancel_queue_edit() {
            AppAction::None
        } else if self.backtrack_pending.is_some() {
            self.cancel_backtrack();
            AppAction::None
        } else if self.events_pane.open {
            self.events_pane.open = false;
            AppAction::None
        } else if matches!(
            self.session_state,
            SessionState::Generating
                | SessionState::ExecutingTools
                | SessionState::Paused
                | SessionState::WaitingUserInput
        ) {
            self.cancel_queue_edit();
            self.abort_in_flight = true;
            self.add_notice("Cancel requested");
            AppAction::Abort
        } else {
            self.handle_idle_escape()
        }
    }

    pub(super) fn accept_action(&mut self) -> AppAction {
        if self.selected_backtrack_index.is_some() && self.composer.is_empty() {
            return self.accept_backtrack_selection();
        }
        if self.input_queue.selected_index().is_some() && !self.input_queue.is_editing() {
            return self.begin_queue_edit();
        }
        if self.composer.is_empty() && self.toggle_selected_tool() {
            return AppAction::None;
        }
        match self.composer.enter(Instant::now()) {
            EnterDecision::InsertedNewline => AppAction::None,
            EnterDecision::Submit => self.submit_composer().unwrap_or(AppAction::None),
        }
    }

    pub(super) fn backspace_action(&mut self) -> AppAction {
        if self.composer.is_empty()
            && self.input_queue.selected_index().is_some()
            && !self.input_queue.is_editing()
        {
            return self.remove_selected_queue_item();
        }
        self.input_queue.clear_selection();
        self.composer.backspace();
        AppAction::None
    }

    pub(super) fn delete_action(&mut self) -> AppAction {
        if self.composer.is_empty()
            && self.input_queue.selected_index().is_some()
            && !self.input_queue.is_editing()
        {
            return self.remove_selected_queue_item();
        }
        self.input_queue.clear_selection();
        self.composer.delete();
        AppAction::None
    }

    pub(super) fn move_up_action(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.is_empty()
            && !self.composer_state().can_move_up()
            && !self.input_queue.is_empty()
            && self.input_queue.select_prev()
        {
            return AppAction::None;
        }
        self.composer
            .move_up_or_history(key.modifiers.contains(KeyModifiers::SHIFT));
        AppAction::None
    }

    pub(super) fn move_down_action(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.is_empty()
            && self.input_queue.selected_index().is_some()
            && self.input_queue.select_next_or_clear()
        {
            return AppAction::None;
        }
        self.composer
            .move_down_or_history(key.modifiers.contains(KeyModifiers::SHIFT));
        AppAction::None
    }

    pub(super) fn ctrl_c_action(&mut self) -> AppAction {
        self.dismiss_interrupt_surfaces();
        if matches!(
            self.session_state,
            SessionState::Generating
                | SessionState::ExecutingTools
                | SessionState::Paused
                | SessionState::WaitingUserInput
        ) {
            self.cancel_queue_edit();
            self.abort_in_flight = true;
            self.clear_approvals();
            self.clear_active_ask_questions();
            self.add_notice("Cancel requested");
            self.last_ctrl_c = None;
            return AppAction::Abort;
        }
        let now = Instant::now();
        if self
            .last_ctrl_c
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(900))
        {
            self.should_quit = true;
        } else {
            self.add_notice("Press Ctrl-C again to exit");
            self.last_ctrl_c = Some(now);
        }
        AppAction::None
    }

    pub(super) fn dismiss_interrupt_surfaces(&mut self) {
        self.help_open = false;
        self.transcript_overlay = None;
        self.cancel_modal_picker();
        self.composer_mode = ComposerMode::Chat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_history_search_text() {
        assert_eq!(
            composer_search_text(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty())),
            Some('x'),
        );
    }
}
