// Adapted from openai/codex codex-rs/tui/src/bottom_pane/request_user_input, Apache-2.0.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::ask_questions::{AskQuestionType, AskQuestionsForm};
use crate::style::accent_style;
use crate::ui::menu::{self, GenericDisplayRow, ScrollState};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const MAX_TEXTAREA_ROWS: u16 = 4;
const FOOTER_ROWS: u16 = 2;

pub(crate) fn desired_height(form: &AskQuestionsForm, available_height: u16) -> u16 {
    form_height(form).min(available_height).max(1)
}

pub(crate) fn render_ask_form(frame: &mut Frame<'_>, form: &AskQuestionsForm, area: Rect) {
    if area.is_empty() {
        return;
    }
    let inner = menu::render_menu_surface(area, frame.buffer_mut());
    render_ask_content(frame, form, inner);
}

fn form_height(form: &AskQuestionsForm) -> u16 {
    let answer_rows = match form.current_question().question_type {
        AskQuestionType::FreeText => textarea_height(form),
        AskQuestionType::YesNo | AskQuestionType::SingleSelect | AskQuestionType::MultiSelect => {
            form.current_question().choice_options().len().max(1).min(8) as u16
        }
    };
    let content_height = 1u16
        .saturating_add(1)
        .saturating_add(1)
        .saturating_add(answer_rows)
        .saturating_add(1)
        .saturating_add(FOOTER_ROWS);
    content_height.saturating_add(2)
}

fn render_ask_content(frame: &mut Frame<'_>, form: &AskQuestionsForm, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let bottom = area.y.saturating_add(area.height);
    let mut y = area.y;

    render_line(frame, progress_line(form), row(area, y));
    y = y.saturating_add(1);
    if y >= bottom {
        return;
    }

    render_line(
        frame,
        question_line(form, area.width as usize),
        row(area, y),
    );
    y = y.saturating_add(1);
    if y < bottom {
        y = y.saturating_add(1);
    }

    let footer_rows = FOOTER_ROWS.min(area.height.saturating_sub(4));
    let footer_top = bottom.saturating_sub(footer_rows);
    let answer_bottom = footer_top.saturating_sub(u16::from(footer_rows > 0));
    let answer_height = answer_bottom.saturating_sub(y);
    if answer_height > 0 {
        match form.current_question().question_type {
            AskQuestionType::FreeText => {
                render_textarea(frame, form, area_with_height(area, y, answer_height))
            }
            AskQuestionType::YesNo
            | AskQuestionType::SingleSelect
            | AskQuestionType::MultiSelect => {
                render_options(frame, form, area_with_height(area, y, answer_height));
            }
        }
    }

    if footer_rows > 0 {
        render_line(
            frame,
            menu::standard_popup_hint_line().dim(),
            row(area, footer_top),
        );
    }
    if footer_rows > 1 {
        render_line(
            frame,
            Line::from(type_specific_hint(form.current_question().question_type)).dim(),
            row(area, footer_top.saturating_add(1)),
        );
    }
}

fn render_options(frame: &mut Frame<'_>, form: &AskQuestionsForm, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = option_rows(form);
    let mut state = ScrollState {
        selected_idx: form.current_choice_index(),
        scroll_top: 0,
    };
    let visible_rows = rows.len().max(1).min(area.height as usize);
    state.clamp_selection(rows.len());
    state.ensure_visible(rows.len(), visible_rows);
    menu::render_rows_single_line(
        area,
        frame.buffer_mut(),
        &rows,
        &state,
        visible_rows,
        "No options",
    );
}

fn option_rows(form: &AskQuestionsForm) -> Vec<GenericDisplayRow> {
    let question = form.current_question();
    question
        .choice_options()
        .into_iter()
        .enumerate()
        .map(|(idx, option)| {
            let cursor = if form.current_choice_index() == Some(idx) {
                "› "
            } else {
                "  "
            };
            let marker = match question.question_type {
                AskQuestionType::MultiSelect if form.option_selected(idx) => "☑",
                AskQuestionType::MultiSelect => "☐",
                _ if form.option_selected(idx) => "◉",
                _ => "○",
            };
            GenericDisplayRow {
                name: format!("{marker} {option}"),
                name_prefix_spans: vec![Span::raw(cursor)],
                ..Default::default()
            }
        })
        .collect()
}

fn render_textarea(frame: &mut Frame<'_>, form: &AskQuestionsForm, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lines = textarea_lines(form, area.width as usize);
    let visible = lines
        .into_iter()
        .take(area.height as usize)
        .collect::<Vec<_>>();
    Paragraph::new(visible).render(area, frame.buffer_mut());
}

fn textarea_lines(form: &AskQuestionsForm, width: usize) -> Vec<Line<'static>> {
    let text = form.current_text().unwrap_or_default();
    let lines = if text.is_empty() {
        vec![Line::from(vec![
            Span::styled("› ", accent_style()),
            Span::styled(
                "Type your answer…",
                Style::default().add_modifier(Modifier::DIM),
            ),
        ])]
    } else {
        text.lines()
            .map(|line| Line::from(vec![Span::raw("  "), Span::raw(line.to_string())]))
            .collect::<Vec<_>>()
    };
    lines
        .into_iter()
        .map(|line| truncate_line_with_ellipsis_if_overflow(line, width))
        .collect()
}

fn textarea_height(form: &AskQuestionsForm) -> u16 {
    form.current_text()
        .filter(|text| !text.is_empty())
        .map(|text| text.lines().count().max(1).min(MAX_TEXTAREA_ROWS as usize) as u16)
        .unwrap_or(1)
}

fn progress_line(form: &AskQuestionsForm) -> Line<'static> {
    Line::from(format!(
        "Question {}/{}",
        form.current_index() + 1,
        form.question_count()
    ))
    .dim()
}

fn question_line(form: &AskQuestionsForm, width: usize) -> Line<'static> {
    let line = Line::from(form.current_question().text.clone());
    let line = if current_question_answered(form) {
        line
    } else {
        line.cyan()
    };
    truncate_line_with_ellipsis_if_overflow(line, width)
}

fn current_question_answered(form: &AskQuestionsForm) -> bool {
    match form.current_question().question_type {
        AskQuestionType::YesNo | AskQuestionType::SingleSelect => false,
        AskQuestionType::MultiSelect => {
            (0..form.current_question().choice_options().len()).any(|idx| form.option_selected(idx))
        }
        AskQuestionType::FreeText => form
            .current_text()
            .is_some_and(|text| !text.trim().is_empty()),
    }
}

fn render_line(frame: &mut Frame<'_>, line: Line<'static>, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = truncate_line_with_ellipsis_if_overflow(line, area.width as usize);
    line.render(area, frame.buffer_mut());
}

fn row(area: Rect, y: u16) -> Rect {
    Rect {
        x: area.x,
        y,
        width: area.width,
        height: 1,
    }
}

fn area_with_height(area: Rect, y: u16, height: u16) -> Rect {
    Rect {
        x: area.x,
        y,
        width: area.width,
        height,
    }
}

pub(crate) fn type_specific_hint(question_type: AskQuestionType) -> &'static str {
    match question_type {
        AskQuestionType::YesNo => "Y/N choose · ↑/↓ move · ←/→ question",
        AskQuestionType::SingleSelect => "↑/↓ choose · ←/→ question",
        AskQuestionType::MultiSelect => "↑/↓ move · Space toggle · ←/→ question",
        AskQuestionType::FreeText => "Type answer · Ctrl-J newline · ←/→ question",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask_questions::{AskQuestionsForm, AskQuestionsRequest};
    use ratatui::backend::{Backend, TestBackend};
    use ratatui::layout::Position;
    use ratatui::style::Color;
    use ratatui::{Terminal, TerminalOptions, Viewport};
    use serde_json::{json, Value};

    fn form_with_questions(questions: Value) -> AskQuestionsForm {
        let request = AskQuestionsRequest::from_tool_content(
            &json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": questions,
            })
            .to_string(),
            None,
        )
        .unwrap();
        AskQuestionsForm::new(request)
    }

    fn rendered_form(form: &AskQuestionsForm) -> (String, ratatui::buffer::Buffer) {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_ask_form(frame, form, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        (text, buffer)
    }

    fn find_text_start(buffer: &ratatui::buffer::Buffer, text: &str) -> Option<(u16, u16)> {
        let area = buffer.area;
        for y in area.y..area.y.saturating_add(area.height) {
            let row_text = (area.x..area.x.saturating_add(area.width))
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>();
            if let Some(x) = row_text.find(text) {
                return Some((area.x + x as u16, y));
            }
        }
        None
    }

    fn is_drawn_cell(cell: &ratatui::buffer::Cell) -> bool {
        cell.symbol() != " "
    }

    fn assert_no_drawn_cells_outside(buffer: &ratatui::buffer::Buffer, bounds: Rect) -> bool {
        let mut found = false;
        for y in buffer.area.top()..buffer.area.bottom() {
            for x in buffer.area.left()..buffer.area.right() {
                let cell = &buffer[(x, y)];
                if !is_drawn_cell(cell) {
                    continue;
                }
                found = true;
                assert!(
                    x >= bounds.left()
                        && x < bounds.right()
                        && y >= bounds.top()
                        && y < bounds.bottom(),
                    "drawn cell ({x},{y}) escaped bounds {bounds:?}"
                );
            }
        }
        found
    }

    #[test]
    fn ask_form_clamps_to_tiny_areas() {
        let form = form_with_questions(json!([
            {"id":"confirm","type":"yes_no","text":"Proceed?"}
        ]));

        for (width, height) in [(2, 2), (1, 1)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render_ask_form(frame, &form, frame.area()))
                .unwrap();
            assert_eq!(terminal.backend().buffer().area.width, width);
            assert_eq!(terminal.backend().buffer().area.height, height);
        }

        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_ask_form(frame, &form, Rect::new(0, 0, 0, frame.area().height));
                render_ask_form(frame, &form, Rect::new(0, 0, frame.area().width, 0));
            })
            .unwrap();
    }

    #[test]
    fn ask_form_renders_inside_supplied_composer_rect() {
        let form = form_with_questions(json!([
            {"id":"confirm","type":"yes_no","text":"Proceed?"}
        ]));
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let composer = Rect::new(0, 14, 80, 6);

        terminal
            .draw(|frame| render_ask_form(frame, &form, composer))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let question = find_text_start(buffer, "Proceed?").expect("question rendered");

        assert!(question.1 >= composer.y);
        assert!(question.1 < composer.bottom());
        assert!(assert_no_drawn_cells_outside(buffer, composer));
    }

    #[test]
    fn ask_form_stays_inside_inline_viewport() {
        let form = form_with_questions(json!([
            {"id":"areas","type":"multi_select","text":"Areas?","options":["Tests","Docs","Ui"]}
        ]));
        let viewport = Rect::new(0, 21, 72, 8);
        let composer = Rect::new(0, 24, 72, 5);
        let mut backend = TestBackend::new(viewport.width, viewport.y + viewport.height);
        backend
            .set_cursor_position(Position::new(0, viewport.y))
            .unwrap();
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(viewport.height),
            },
        )
        .unwrap();

        let completed = terminal
            .draw(|frame| {
                assert_eq!(frame.area(), viewport);
                render_ask_form(frame, &form, composer);
            })
            .unwrap();
        let buffer = completed.buffer.clone();

        assert_eq!(buffer.area, viewport);
        assert!(assert_no_drawn_cells_outside(&buffer, composer));
        assert!(find_text_start(&buffer, "Areas?").is_some());
    }

    #[test]
    fn ask_form_renders_deboxed_options_and_footer() {
        let form = form_with_questions(json!([
            {"id":"confirm","type":"yes_no","text":"Proceed?"}
        ]));

        let (text, buffer) = rendered_form(&form);

        assert!(text.contains("Question 1/1"));
        assert!(text.contains("› ◉ Yes"));
        assert!(text.contains("○ No"));
        assert!(text.contains("Press Enter to confirm or Esc to go back"));
        assert!(text.contains("Y/N choose"));
        assert!(!text.contains("┌"));
        let cursor = buffer
            .content()
            .iter()
            .find(|cell| cell.symbol() == "›")
            .expect("selected cursor rendered");
        assert_eq!(cursor.style().fg, Some(Color::Cyan));
        assert!(cursor.style().add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn free_text_renders_cyan_question_and_textarea_hint() {
        let form = form_with_questions(json!([
            {"id":"notes","type":"free_text","text":"Notes?"}
        ]));

        let (text, buffer) = rendered_form(&form);

        assert!(text.contains("Question 1/1"));
        assert!(text.contains("Type your answer…"));
        assert!(text.contains("Type answer · Ctrl-J newline"));
        assert!(!text.contains("┌"));
        let (x, y) = find_text_start(&buffer, "Notes?").expect("question rendered");
        assert_eq!(buffer[(x, y)].style().fg, Some(Color::Cyan));
    }

    #[test]
    fn free_text_renders_current_multiline_text() {
        let mut form = form_with_questions(json!([
            {"id":"notes","type":"free_text","text":"Notes?"}
        ]));
        for ch in "First".chars() {
            form.insert_char(ch);
        }
        form.insert_newline();
        for ch in "Second".chars() {
            form.insert_char(ch);
        }

        let (text, _buffer) = rendered_form(&form);

        assert!(text.contains("First"));
        assert!(text.contains("Second"));
        assert!(!text.contains("Type your answer…"));
    }
}
