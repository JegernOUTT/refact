use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestInputToolCell {
    card: ToolCard,
    selected: bool,
}

impl RequestInputToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for RequestInputToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::RequestInput
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(vec![
            dim_span("•"),
            Span::raw(" "),
            bold_span("Questions"),
        ])];
        lines.push(tool_summary_line(
            &self.card,
            request_input_summary(&self.card),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        lines.extend(subchat_lines(&self.card, width));
        if self.card.expanded {
            let questions = request_input_questions(&self.card);
            let answers = request_input_answers(&self.card.result);
            for question in &questions {
                lines.extend(wrap_with_prefix(
                    &question.text,
                    width,
                    dim_span("  • "),
                    dim_span("    "),
                    Style::default(),
                ));
                if let Some(answer) = answer_for_question(question, &answers, questions.len()) {
                    let answer = if question.secret {
                        "••••••".to_string()
                    } else {
                        answer
                    };
                    lines.extend(wrap_with_prefix(
                        &answer,
                        width,
                        dim_span("    answer: "),
                        dim_span("            "),
                        Style::default().fg(Color::Cyan),
                    ));
                }
            }
            if (questions.is_empty() || answers.is_empty())
                && !self.card.result.trim().is_empty()
                && !is_question_payload(&self.card.result)
            {
                lines.extend(prefix_lines(
                    output_lines(&self.card.result, width, EXPANDED_OUTPUT_LINES, false),
                    dim_span("  └ "),
                    dim_span("    "),
                ));
            }
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.card.status != ToolStatus::Running
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RequestQuestion {
    id: Option<String>,
    text: String,
    secret: bool,
}

fn request_input_questions(card: &ToolCard) -> Vec<RequestQuestion> {
    let Some(value) = serde_json::from_str::<Value>(&card.args).ok() else {
        return Vec::new();
    };
    if let Some(questions) = value.get("questions").and_then(Value::as_array) {
        return questions.iter().filter_map(question_from_value).collect();
    }
    question_from_value(&value).into_iter().collect()
}

fn request_input_summary(card: &ToolCard) -> String {
    let questions = request_input_questions(card);
    match questions.len() {
        0 => request_input_label(card),
        1 => questions[0].text.clone(),
        n => format!("{n} questions"),
    }
}

fn question_from_value(value: &Value) -> Option<RequestQuestion> {
    let text = value
        .get("text")
        .or_else(|| value.get("question"))
        .or_else(|| value.get("prompt"))
        .or_else(|| value.get("message"))
        .or_else(|| value.get("title"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let question_type = value
        .get("type")
        .or_else(|| value.get("question_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let secret = value
        .get("is_secret")
        .or_else(|| value.get("secret"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || matches!(question_type.as_str(), "secret" | "password");
    Some(RequestQuestion { id, text, secret })
}

fn request_input_answers(result: &str) -> std::collections::HashMap<String, String> {
    let trimmed = result.trim();
    if trimmed.is_empty() {
        return std::collections::HashMap::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return answers_from_json(&value);
    }
    answers_from_qa_text(trimmed)
        .or_else(|| simple_answer(trimmed))
        .unwrap_or_default()
}

fn answers_from_json(value: &Value) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(answers) = value.get("answers") {
        collect_answer_value(answers, &mut out);
    } else if let Some(answer) = value.get("answer") {
        let answer = value_to_answer(answer);
        if !answer.is_empty() {
            out.insert("__single__".to_string(), answer);
        }
    }
    out
}

fn collect_answer_value(value: &Value, out: &mut std::collections::HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            if map.contains_key("id") || map.contains_key("question_id") {
                if let Some((id, answer)) = answer_entry(value) {
                    out.insert(id, answer);
                }
            } else {
                for (key, value) in map {
                    let answer = value_to_answer(value);
                    if !answer.is_empty() {
                        out.insert(key.clone(), answer);
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some((id, answer)) = answer_entry(item) {
                    out.insert(id, answer);
                }
            }
        }
        _ => {
            let answer = value_to_answer(value);
            if !answer.is_empty() {
                out.insert("__single__".to_string(), answer);
            }
        }
    }
}

fn answer_entry(value: &Value) -> Option<(String, String)> {
    let id = value
        .get("id")
        .or_else(|| value.get("question_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let answer = value
        .get("answer")
        .or_else(|| value.get("answers"))
        .or_else(|| value.get("value"))
        .or_else(|| value.get("text"))
        .map(value_to_answer)
        .unwrap_or_default();
    (!answer.is_empty()).then_some((id, answer))
}

fn value_to_answer(value: &Value) -> String {
    match sanitize_json_strings(value) {
        Value::String(value) => value,
        Value::Array(values) => values
            .iter()
            .map(value_to_string)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => String::new(),
        value => serde_json::to_string(&value).unwrap_or_else(|_| value.to_string()),
    }
}

fn answers_from_qa_text(text: &str) -> Option<std::collections::HashMap<String, String>> {
    if !text.starts_with("[QA:") {
        return None;
    }
    let mut out = std::collections::HashMap::new();
    let mut current_id = None::<String>;
    let mut answer = Vec::<String>::new();
    for line in text.lines().skip(1) {
        if let Some(id) = parse_qa_question_id(line) {
            flush_answer(&mut out, current_id.take(), &mut answer);
            current_id = Some(id);
        } else if !line.trim().is_empty() && line.trim() != "```" {
            answer.push(line.to_string());
        }
    }
    flush_answer(&mut out, current_id, &mut answer);
    Some(out)
}

fn parse_qa_question_id(line: &str) -> Option<String> {
    let rest = line.strip_prefix("> [")?;
    let (id, _) = rest.split_once(']')?;
    (!id.is_empty()).then(|| id.to_string())
}

fn flush_answer(
    out: &mut std::collections::HashMap<String, String>,
    id: Option<String>,
    answer: &mut Vec<String>,
) {
    let Some(id) = id else {
        answer.clear();
        return;
    };
    let value = answer.join("\n").trim().to_string();
    if !value.is_empty() {
        out.insert(id, value);
    }
    answer.clear();
}

fn simple_answer(text: &str) -> Option<std::collections::HashMap<String, String>> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("waiting for user input") || lower.contains("ask_questions") {
        return None;
    }
    let mut out = std::collections::HashMap::new();
    out.insert("__single__".to_string(), text.to_string());
    Some(out)
}

fn answer_for_question(
    question: &RequestQuestion,
    answers: &std::collections::HashMap<String, String>,
    total_questions: usize,
) -> Option<String> {
    question
        .id
        .as_ref()
        .and_then(|id| answers.get(id))
        .or_else(|| {
            (total_questions == 1)
                .then(|| answers.get("__single__"))
                .flatten()
        })
        .cloned()
}

fn is_question_payload(result: &str) -> bool {
    serde_json::from_str::<Value>(result)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|kind| kind == "ask_questions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{text, tool_card};
    use serde_json::json;

    #[test]
    fn request_input_cell_snapshot() {
        let card = tool_card(
            "ask_questions",
            json!({"questions": [{"id": "file", "type": "free_text", "text": "Which file should I edit?"}]}),
            "waiting for user input",
        );
        let rendered = text(&RequestInputToolCell::new(card, true).render(80));
        assert_eq!(
            rendered,
            "• Questions\n▾ ✅ Which file should I edit? · 1.2s\n  • Which file should I edit?\n  └ waiting for user input\n"
        );
    }

    #[test]
    fn request_input_cell_renders_answers_and_masks_secrets() {
        let card = tool_card(
            "request_user_input",
            json!({"questions": [
                {"id": "name", "question": "Name?"},
                {"id": "token", "question": "Token?", "is_secret": true}
            ]}),
            r#"{"answers":{"name":"Pixel","token":"sk-secret"}}"#,
        );
        let rendered = text(&RequestInputToolCell::new(card, false).render(80));
        assert_eq!(
            rendered,
            "• Questions\n▾ ✅ 2 questions · 1.2s\n  • Name?\n    answer: Pixel\n  • Token?\n    answer: ••••••\n"
        );
    }
}
