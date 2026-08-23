use super::super::*;

impl App {
    pub(super) fn handle_ask_questions_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(form) = self.ask_questions_form.as_mut() else {
            return AppAction::None;
        };
        if form.current_question().question_type == AskQuestionType::MultiSelect
            && is_plain_space_key(key)
        {
            form.toggle_current_multi();
            return AppAction::None;
        }
        if form.current_question().question_type == AskQuestionType::FreeText {
            if let KeyCode::Char(ch) = key.code {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    form.insert_char(ch);
                    return AppAction::None;
                }
            }
        }
        let dispatch = self.keymap.dispatch(KeyContext::Main, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                let request = form.request().clone();
                form.cancel();
                self.handled_ask_questions_tool_ids
                    .insert(request.tool_call_id.clone());
                self.pending_manual_ask_questions = Some(request);
                self.ask_questions_form = None;
                self.add_notice(
                    "Question form canceled; type your answer in the composer to reply manually",
                );
                AppAction::None
            }
            Some(KeyAction::Accept) => {
                let tool_call_id = form.tool_call_id().to_string();
                match form.accept() {
                    AskQuestionsOutcome::Submitted(prompt) => {
                        self.handled_ask_questions_tool_ids.insert(tool_call_id);
                        self.pending_manual_ask_questions = None;
                        self.ask_questions_form = None;
                        self.submit_ask_questions_reply(prompt)
                    }
                    AskQuestionsOutcome::None
                    | AskQuestionsOutcome::Incomplete
                    | AskQuestionsOutcome::Canceled => AppAction::None,
                }
            }
            Some(KeyAction::MoveUp) => {
                form.previous_option();
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                form.next_option();
                AppAction::None
            }
            Some(KeyAction::MoveLeft) | Some(KeyAction::MoveHome) => {
                form.previous_question();
                AppAction::None
            }
            Some(KeyAction::MoveRight) | Some(KeyAction::MoveEnd) => {
                form.next_question();
                AppAction::None
            }
            Some(KeyAction::ToggleSelectedTool) => {
                form.toggle_current_multi();
                AppAction::None
            }
            Some(KeyAction::Backspace) => {
                form.backspace();
                AppAction::None
            }
            Some(KeyAction::InsertNewline) => {
                form.insert_newline();
                AppAction::None
            }
            None => {
                if let Some(ch) = dispatch.text {
                    if form.current_question().question_type == AskQuestionType::YesNo {
                        match ch {
                            'y' | 'Y' => form.choose_yes_no(true),
                            'n' | 'N' => form.choose_yes_no(false),
                            _ => form.insert_char(ch),
                        }
                    } else {
                        form.insert_char(ch);
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
    fn recognizes_plain_space() {
        assert!(is_plain_space_key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::empty(),
        )));
    }
}
