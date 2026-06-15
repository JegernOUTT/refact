use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskQuestionType {
    YesNo,
    SingleSelect,
    MultiSelect,
    FreeText,
}

impl AskQuestionType {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "yes_no" => Some(Self::YesNo),
            "single_select" => Some(Self::SingleSelect),
            "multi_select" => Some(Self::MultiSelect),
            "free_text" => Some(Self::FreeText),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestion {
    pub id: String,
    pub question_type: AskQuestionType,
    pub text: String,
    pub options: Vec<String>,
}

impl AskQuestion {
    pub fn choice_options(&self) -> Vec<String> {
        match self.question_type {
            AskQuestionType::YesNo => vec!["Yes".to_string(), "No".to_string()],
            AskQuestionType::SingleSelect | AskQuestionType::MultiSelect => self.options.clone(),
            AskQuestionType::FreeText => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestionsRequest {
    pub tool_call_id: String,
    pub questions: Vec<AskQuestion>,
}

impl AskQuestionsRequest {
    pub fn from_tool_content(content: &str, fallback_tool_call_id: Option<&str>) -> Option<Self> {
        let raw: Value = serde_json::from_str(content).ok()?;
        if raw.get("type").and_then(Value::as_str)? != "ask_questions" {
            return None;
        }
        let tool_call_id = raw
            .get("tool_call_id")
            .and_then(Value::as_str)
            .or(fallback_tool_call_id)
            .filter(|value| !value.trim().is_empty())?
            .to_string();
        let raw_questions = raw.get("questions").and_then(Value::as_array)?;
        let questions = raw_questions
            .iter()
            .map(parse_question)
            .collect::<Option<Vec<_>>>()?;
        (!questions.is_empty()).then_some(Self {
            tool_call_id,
            questions,
        })
    }

    pub fn format_manual_reply(&self, answer: &str) -> String {
        let mut lines = vec![format!("[QA:{}]", self.tool_call_id)];
        for question in &self.questions {
            lines.push(format!("> [{}] {}", question.id, question.text));
            lines.push("(no answer)".to_string());
            lines.push(String::new());
        }
        let answer = answer.trim();
        if !answer.is_empty() {
            lines.push("> [__additional__] Additional comments".to_string());
            push_answer_lines(&mut lines, answer);
        }
        lines.join("\n").trim().to_string()
    }
}

fn parse_question(raw: &Value) -> Option<AskQuestion> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let question_type = raw
        .get("type")
        .or_else(|| raw.get("question_type"))
        .and_then(Value::as_str)
        .and_then(AskQuestionType::from_str)?;
    let text = raw
        .get("text")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let options = raw
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if matches!(
        question_type,
        AskQuestionType::SingleSelect | AskQuestionType::MultiSelect
    ) && options.is_empty()
    {
        return None;
    }
    Some(AskQuestion {
        id,
        question_type,
        text,
        options,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AskAnswerState {
    Choice { selected: usize },
    Multi { cursor: usize, selected: Vec<bool> },
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskQuestionsOutcome {
    None,
    Submitted(String),
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestionsForm {
    request: AskQuestionsRequest,
    current: usize,
    answers: Vec<AskAnswerState>,
}

impl AskQuestionsForm {
    pub fn new(request: AskQuestionsRequest) -> Self {
        let answers = request
            .questions
            .iter()
            .map(|question| match question.question_type {
                AskQuestionType::YesNo | AskQuestionType::SingleSelect => {
                    AskAnswerState::Choice { selected: 0 }
                }
                AskQuestionType::MultiSelect => AskAnswerState::Multi {
                    cursor: 0,
                    selected: vec![false; question.options.len()],
                },
                AskQuestionType::FreeText => AskAnswerState::Text(String::new()),
            })
            .collect();
        Self {
            request,
            current: 0,
            answers,
        }
    }

    pub fn tool_call_id(&self) -> &str {
        &self.request.tool_call_id
    }

    pub fn request(&self) -> &AskQuestionsRequest {
        &self.request
    }

    pub fn questions(&self) -> &[AskQuestion] {
        &self.request.questions
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    pub fn question_count(&self) -> usize {
        self.request.questions.len()
    }

    pub fn current_question(&self) -> &AskQuestion {
        &self.request.questions[self.current]
    }

    pub fn current_answer_text(&self) -> String {
        self.answer_text(self.current)
    }

    pub fn current_choice_index(&self) -> Option<usize> {
        match self.answers.get(self.current)? {
            AskAnswerState::Choice { selected } => Some(*selected),
            AskAnswerState::Multi { cursor, .. } => Some(*cursor),
            AskAnswerState::Text(_) => None,
        }
    }

    pub fn current_text(&self) -> Option<&str> {
        match self.answers.get(self.current)? {
            AskAnswerState::Text(text) => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn option_selected(&self, option_index: usize) -> bool {
        match self.answers.get(self.current) {
            Some(AskAnswerState::Choice { selected }) => *selected == option_index,
            Some(AskAnswerState::Multi { selected, .. }) => {
                selected.get(option_index).copied().unwrap_or(false)
            }
            _ => false,
        }
    }

    pub fn next_option(&mut self) {
        let option_count = self.current_question().choice_options().len();
        if option_count == 0 {
            return;
        }
        match self.answers.get_mut(self.current) {
            Some(AskAnswerState::Choice { selected }) => {
                *selected = (*selected + 1).min(option_count - 1)
            }
            Some(AskAnswerState::Multi { cursor, .. }) => {
                *cursor = (*cursor + 1).min(option_count - 1)
            }
            _ => {}
        }
    }

    pub fn previous_option(&mut self) {
        match self.answers.get_mut(self.current) {
            Some(AskAnswerState::Choice { selected }) => {
                *selected = selected.saturating_sub(1);
            }
            Some(AskAnswerState::Multi { cursor, .. }) => {
                *cursor = cursor.saturating_sub(1);
            }
            _ => {}
        }
    }

    pub fn choose_yes_no(&mut self, yes: bool) {
        if self.current_question().question_type != AskQuestionType::YesNo {
            return;
        }
        if let Some(AskAnswerState::Choice { selected }) = self.answers.get_mut(self.current) {
            *selected = if yes { 0 } else { 1 };
        }
    }

    pub fn toggle_current_multi(&mut self) {
        if let Some(AskAnswerState::Multi { cursor, selected }) = self.answers.get_mut(self.current)
        {
            if let Some(value) = selected.get_mut(*cursor) {
                *value = !*value;
            }
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        if let Some(AskAnswerState::Text(text)) = self.answers.get_mut(self.current) {
            text.push(ch);
        }
    }

    pub fn insert_newline(&mut self) {
        if let Some(AskAnswerState::Text(text)) = self.answers.get_mut(self.current) {
            text.push('\n');
        }
    }

    pub fn backspace(&mut self) {
        if let Some(AskAnswerState::Text(text)) = self.answers.get_mut(self.current) {
            text.pop();
        }
    }

    pub fn next_question(&mut self) {
        self.current = (self.current + 1).min(self.request.questions.len().saturating_sub(1));
    }

    pub fn previous_question(&mut self) {
        self.current = self.current.saturating_sub(1);
    }

    pub fn accept(&mut self) -> AskQuestionsOutcome {
        if self.current + 1 < self.request.questions.len() {
            self.current += 1;
            AskQuestionsOutcome::None
        } else {
            AskQuestionsOutcome::Submitted(self.format_answers())
        }
    }

    pub fn cancel(&self) -> AskQuestionsOutcome {
        AskQuestionsOutcome::Canceled
    }

    pub fn format_answers(&self) -> String {
        let mut lines = vec![format!("[QA:{}]", self.request.tool_call_id)];
        for index in 0..self.request.questions.len() {
            let question = &self.request.questions[index];
            lines.push(format!("> [{}] {}", question.id, question.text));
            let answer = self.answer_text(index);
            push_answer_lines(&mut lines, &answer);
            lines.push(String::new());
        }
        lines.join("\n").trim().to_string()
    }

    fn answer_text(&self, index: usize) -> String {
        let question = &self.request.questions[index];
        match &self.answers[index] {
            AskAnswerState::Choice { selected } => question
                .choice_options()
                .get(*selected)
                .cloned()
                .unwrap_or_else(|| "(no answer)".to_string()),
            AskAnswerState::Multi { selected, .. } => {
                let options = question.choice_options();
                let values = selected
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, selected)| {
                        selected.then(|| options.get(idx).cloned()).flatten()
                    })
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    "(no selection)".to_string()
                } else {
                    values.join(", ")
                }
            }
            AskAnswerState::Text(text) => {
                if text.trim().is_empty() {
                    "(no answer)".to_string()
                } else {
                    text.trim().to_string()
                }
            }
        }
    }
}

fn push_answer_lines(lines: &mut Vec<String>, answer: &str) {
    if answer.contains('\n') {
        lines.push("```".to_string());
        lines.push(answer.to_string());
        lines.push("```".to_string());
    } else {
        lines.push(answer.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request_with_questions(questions: Value) -> AskQuestionsRequest {
        AskQuestionsRequest::from_tool_content(
            &json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": questions,
            })
            .to_string(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn parses_all_supported_question_types() {
        let request = request_with_questions(json!([
            {"id":"confirm","type":"yes_no","text":"Proceed?"},
            {"id":"path","type":"single_select","text":"Path?","options":["A","B"]},
            {"id":"areas","type":"multi_select","text":"Areas?","options":["Tests","Docs"]},
            {"id":"notes","type":"free_text","text":"Notes?"}
        ]));

        assert_eq!(request.questions.len(), 4);
        assert_eq!(request.questions[0].question_type, AskQuestionType::YesNo);
        assert_eq!(
            request.questions[1].question_type,
            AskQuestionType::SingleSelect
        );
        assert_eq!(
            request.questions[2].question_type,
            AskQuestionType::MultiSelect
        );
        assert_eq!(
            request.questions[3].question_type,
            AskQuestionType::FreeText
        );
    }

    #[test]
    fn parses_question_type_alias_from_tool_message() {
        let request = request_with_questions(json!([
            {"id":"confirm","question_type":"yes_no","text":"Proceed?"}
        ]));

        assert_eq!(request.questions[0].question_type, AskQuestionType::YesNo);
    }

    #[test]
    fn state_machine_collects_answers_into_canonical_user_message() {
        let request = request_with_questions(json!([
            {"id":"confirm","type":"yes_no","text":"Proceed?"},
            {"id":"path","type":"single_select","text":"Path?","options":["A","B"]},
            {"id":"areas","type":"multi_select","text":"Areas?","options":["Tests","Docs"]},
            {"id":"notes","type":"free_text","text":"Notes?"}
        ]));
        let mut form = AskQuestionsForm::new(request);

        form.next_option();
        assert_eq!(form.accept(), AskQuestionsOutcome::None);
        form.next_option();
        assert_eq!(form.accept(), AskQuestionsOutcome::None);
        form.toggle_current_multi();
        form.next_option();
        form.toggle_current_multi();
        assert_eq!(form.accept(), AskQuestionsOutcome::None);
        for ch in "Ship it".chars() {
            form.insert_char(ch);
        }

        assert_eq!(
            form.accept(),
            AskQuestionsOutcome::Submitted(
                "[QA:call-ask]\n> [confirm] Proceed?\nNo\n\n> [path] Path?\nB\n\n> [areas] Areas?\nTests, Docs\n\n> [notes] Notes?\nShip it".to_string()
            )
        );
    }

    #[test]
    fn multi_select_toggles_current_option() {
        let request = request_with_questions(json!([
            {"id":"areas","type":"multi_select","text":"Areas?","options":["Tests","Docs"]}
        ]));
        let mut form = AskQuestionsForm::new(request);

        form.toggle_current_multi();
        assert!(form.option_selected(0));
        form.toggle_current_multi();
        assert!(!form.option_selected(0));
        form.next_option();
        form.toggle_current_multi();

        assert_eq!(form.current_answer_text(), "Docs");
    }

    #[test]
    fn free_text_supports_newline_and_backspace() {
        let request = request_with_questions(json!([
            {"id":"notes","type":"free_text","text":"Notes?"}
        ]));
        let mut form = AskQuestionsForm::new(request);

        for ch in "Hi".chars() {
            form.insert_char(ch);
        }
        form.insert_newline();
        for ch in "there!".chars() {
            form.insert_char(ch);
        }
        form.backspace();

        assert_eq!(form.current_answer_text(), "Hi\nthere");
        assert_eq!(
            form.format_answers(),
            "[QA:call-ask]\n> [notes] Notes?\n```\nHi\nthere\n```"
        );
    }

    #[test]
    fn cancel_reports_canceled_without_mutating_answers() {
        let request = request_with_questions(json!([
            {"id":"confirm","type":"yes_no","text":"Proceed?"}
        ]));
        let form = AskQuestionsForm::new(request);

        assert_eq!(form.cancel(), AskQuestionsOutcome::Canceled);
        assert_eq!(form.current_answer_text(), "Yes");
    }
}
