use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::text_safety::{sanitize_tool_inline, sanitize_tool_text};
use crate::ui::menu;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::client::{TaskBoardCard, TaskBoardViewData};

const BOARD_COLUMNS: &[(&str, &str)] = &[
    ("planned", "Planned"),
    ("doing", "Doing"),
    ("done", "Done"),
    ("failed", "Failed"),
    ("regressed", "Regressed"),
];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoardSurface {
    data: Option<TaskBoardViewData>,
    error: Option<String>,
    selected: usize,
    detail_open: bool,
}

impl BoardSurface {
    pub(crate) fn loading() -> Self {
        Self {
            data: None,
            error: None,
            selected: 0,
            detail_open: false,
        }
    }

    pub(crate) fn loaded(data: TaskBoardViewData) -> Self {
        Self {
            data: Some(data),
            error: None,
            selected: 0,
            detail_open: false,
        }
    }

    pub(crate) fn failed(error: String) -> Self {
        Self {
            data: None,
            error: Some(error),
            selected: 0,
            detail_open: false,
        }
    }

    pub(crate) fn select_next(&mut self) {
        let len = self.cards().len();
        if len > 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    pub(crate) fn select_previous(&mut self) {
        let len = self.cards().len();
        if len > 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(len - 1);
        }
    }

    pub(crate) fn toggle_detail(&mut self) {
        if self.selected_card().is_some() {
            self.detail_open = !self.detail_open;
        }
    }

    pub(crate) fn selected_agent_chat(&self) -> Option<(String, String)> {
        let card = self.selected_card()?;
        let chat_id = card.agent_chat_id.as_deref()?.trim();
        (!chat_id.is_empty()).then(|| (chat_id.to_string(), card.title.clone()))
    }

    fn cards(&self) -> &[TaskBoardCard] {
        self.data
            .as_ref()
            .map(|data| data.board.cards.as_slice())
            .unwrap_or_default()
    }

    fn selected_card(&self) -> Option<&TaskBoardCard> {
        self.cards().get(self.selected)
    }
}

pub(crate) fn task_board_enabled() -> bool {
    std::env::var("REFACT_TUI_SURFACES")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub(crate) fn render_task_board(frame: &mut Frame<'_>, board: &BoardSurface, area: Rect) {
    frame.render_widget(Clear, area);
    let inner = menu::render_menu_surface(area, frame.buffer_mut());
    if inner.is_empty() {
        return;
    }
    let [header, body, hint] = Layout::vertical([
        Constraint::Length(2.min(inner.height)),
        Constraint::Fill(1),
        Constraint::Length(u16::from(inner.height > 2)),
    ])
    .areas(inner);
    render_header(frame, board, header);
    if board.detail_open {
        render_detail(frame, board, body);
    } else if body.width >= 80 {
        render_grid(frame, board, body);
    } else {
        render_list(frame, board, body);
    }
    if hint.height > 0 {
        let hint_text = if board.detail_open {
            "Space board · Enter agent chat · Esc close"
        } else {
            "↑/↓ card · Space details · Enter agent chat · Esc close"
        };
        frame.render_widget(
            Paragraph::new(truncate_line_with_ellipsis_if_overflow(
                Line::from(Span::styled(
                    hint_text,
                    Style::default().add_modifier(Modifier::DIM),
                )),
                hint.width as usize,
            )),
            hint,
        );
    }
}

fn render_header(frame: &mut Frame<'_>, board: &BoardSurface, area: Rect) {
    if area.is_empty() {
        return;
    }
    let lines = match &board.data {
        Some(data) => vec![
            Line::from(vec![
                Span::styled("Task board", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(" · {}", sanitize_tool_inline(&data.task.name))),
            ]),
            Line::from(format!(
                "{} cards · {} ready · {} blocked · rev {}",
                data.board.cards.len(),
                data.ready.ready.len(),
                data.ready.blocked.len(),
                data.board.rev
            )),
        ],
        None => vec![Line::from("Task board"), Line::from("Loading task board…")],
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_grid(frame: &mut Frame<'_>, board: &BoardSurface, area: Rect) {
    let Some(data) = &board.data else {
        render_message(frame, board, area);
        return;
    };
    let columns = board_columns(data);
    let constraints = vec![Constraint::Fill(1); columns.len()];
    let areas = Layout::horizontal(constraints).spacing(1).split(area);
    for ((column_id, column_title), column_area) in columns.iter().zip(areas.iter().copied()) {
        let cards = data
            .board
            .cards
            .iter()
            .enumerate()
            .filter(|(_, card)| card.column == *column_id)
            .flat_map(|(index, card)| grid_card_lines(board, card, index))
            .collect::<Vec<_>>();
        let mut lines = vec![Line::from(Span::styled(
            column_title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        if cards.is_empty() {
            lines.push(Line::from("—"));
        } else {
            lines.extend(cards);
        }
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            column_area,
        );
    }
}

fn render_list(frame: &mut Frame<'_>, board: &BoardSurface, area: Rect) {
    let Some(data) = &board.data else {
        render_message(frame, board, area);
        return;
    };
    if area.width < 60 || area.height < 15 {
        render_compact_list(frame, board, data, area);
        return;
    }
    let mut lines = Vec::new();
    for (column_id, column_title) in board_columns(data) {
        let cards = data
            .board
            .cards
            .iter()
            .enumerate()
            .filter(|(_, card)| card.column == column_id)
            .collect::<Vec<_>>();
        lines.push(Line::from(Span::styled(
            column_title,
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if cards.is_empty() {
            lines.push(Line::from("  —"));
        }
        for (index, card) in cards {
            lines.extend(list_card_lines(board, card, index));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_compact_list(
    frame: &mut Frame<'_>,
    board: &BoardSurface,
    data: &TaskBoardViewData,
    area: Rect,
) {
    let mut lines = board_columns(data)
        .into_iter()
        .map(|(id, title)| {
            let count = data
                .board
                .cards
                .iter()
                .filter(|card| card.column == id)
                .count();
            Line::from(format!("{title}: {count}"))
        })
        .collect::<Vec<_>>();
    lines.push(Line::from(""));
    if let Some(card) = board.selected_card() {
        lines.extend(list_card_lines(board, card, board.selected));
    } else {
        lines.push(Line::from("No cards recorded."));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_detail(frame: &mut Frame<'_>, board: &BoardSurface, area: Rect) {
    let lines = detail_lines(board);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn detail_lines(board: &BoardSurface) -> Vec<Line<'static>> {
    let Some(card) = board.selected_card() else {
        return vec![Line::from("No card selected.")];
    };
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{} · {}",
            sanitize_tool_inline(&card.id),
            sanitize_tool_inline(&card.title)
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(card_metadata(board, card)));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Instructions",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    push_text(&mut lines, &card.instructions, "No instructions recorded.");
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Final report",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if let Some(report) = &card.final_report {
        push_text(&mut lines, report, "No final report recorded.");
    } else if let Some(report) = &card.final_report_structured {
        push_json(&mut lines, report);
    } else {
        lines.push(Line::from("No final report recorded."));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Verifier report",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if let Some(report) = &card.verifier_report {
        push_json(&mut lines, report);
    } else {
        lines.push(Line::from("No verifier report recorded."));
    }
    lines
}

fn render_message(frame: &mut Frame<'_>, board: &BoardSurface, area: Rect) {
    let message = board
        .error
        .as_deref()
        .map(|error| format!("Failed to load task board: {}", sanitize_tool_text(error)))
        .unwrap_or_else(|| "Loading task board…".to_string());
    frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), area);
}

fn board_columns(data: &TaskBoardViewData) -> Vec<(String, String)> {
    BOARD_COLUMNS
        .iter()
        .map(|(id, title)| {
            data.board
                .columns
                .iter()
                .find(|column| column.id == *id)
                .map(|column| (column.id.clone(), column.title.clone()))
                .unwrap_or_else(|| ((*id).to_string(), (*title).to_string()))
        })
        .collect()
}

fn grid_card_lines(board: &BoardSurface, card: &TaskBoardCard, index: usize) -> Vec<Line<'static>> {
    let marker = if board.selected == index { "›" } else { " " };
    vec![
        Line::from(format!(
            "{marker} {} {}",
            sanitize_tool_inline(&card.id),
            priority(card)
        )),
        Line::from(format!("  {}", sanitize_tool_inline(&card.title))),
        Line::from(format!("  {}", state_label(board, card))),
        Line::from(""),
    ]
}

fn list_card_lines(board: &BoardSurface, card: &TaskBoardCard, index: usize) -> Vec<Line<'static>> {
    let marker = if board.selected == index { "›" } else { " " };
    vec![
        Line::from(format!(
            "{marker} {} {} · {}",
            sanitize_tool_inline(&card.id),
            priority(card),
            sanitize_tool_inline(&card.title)
        )),
        Line::from(format!("  {}", state_label(board, card))),
    ]
}

fn card_metadata(board: &BoardSurface, card: &TaskBoardCard) -> String {
    let assignee = card
        .assignee
        .as_deref()
        .map(sanitize_tool_inline)
        .unwrap_or_else(|| "unassigned".to_string());
    let chat = card
        .agent_chat_id
        .as_deref()
        .filter(|chat| !chat.trim().is_empty())
        .map(sanitize_tool_inline)
        .unwrap_or_else(|| "none".to_string());
    format!(
        "{} · {} · assignee {} · agent chat {} · {}",
        priority(card),
        state_label(board, card),
        assignee,
        chat,
        dependency_label(board, card)
    )
}

fn state_label(board: &BoardSurface, card: &TaskBoardCard) -> String {
    let state = match &board.data {
        Some(data) if data.ready.ready.iter().any(|id| id == &card.id) => "ready",
        Some(data) if data.ready.blocked.iter().any(|id| id == &card.id) => "blocked",
        _ => card.column.as_str(),
    };
    format!("{} · {}", card.column, state)
}

fn dependency_label(board: &BoardSurface, card: &TaskBoardCard) -> String {
    if card.depends_on.is_empty() {
        return "no dependencies".to_string();
    }
    let state = if board
        .data
        .as_ref()
        .is_some_and(|data| data.ready.ready.iter().any(|id| id == &card.id))
    {
        "ready"
    } else {
        "blocked"
    };
    format!("depends on {} · {state}", card.depends_on.join(", "))
}

fn priority(card: &TaskBoardCard) -> String {
    if card.priority.trim().is_empty() {
        "P1".to_string()
    } else {
        sanitize_tool_inline(&card.priority)
    }
}

fn push_text(lines: &mut Vec<Line<'static>>, text: &str, empty: &str) {
    let text = sanitize_tool_text(text);
    if text.trim().is_empty() {
        lines.push(Line::from(empty.to_string()));
    } else {
        lines.extend(text.lines().map(|line| Line::from(line.to_string())));
    }
}

fn push_json(lines: &mut Vec<Line<'static>>, value: &serde_json::Value) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    push_text(lines, &text, "No report recorded.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        TaskBoardCard, TaskBoardReadyCards, TaskBoardResponse, TaskBoardTask, TaskBoardViewData,
    };

    fn card(id: &str, column: &str, depends_on: &[&str]) -> TaskBoardCard {
        TaskBoardCard {
            id: id.to_string(),
            title: format!("{id} title"),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: depends_on.iter().map(|id| (*id).to_string()).collect(),
            ..TaskBoardCard::default()
        }
    }

    fn data() -> TaskBoardViewData {
        let mut done = card("T-1", "done", &[]);
        done.final_report = Some("Finished with a snack-sized patch".to_string());
        let mut planned = card("T-2", "planned", &["T-1"]);
        planned.instructions = "Implement the follow-up.".to_string();
        planned.agent_chat_id = Some("agent-chat-2".to_string());
        planned.verifier_report =
            Some(serde_json::json!({"passed": true, "recommendation": "ship it"}));
        let blocked = card("T-3", "planned", &["T-4"]);
        TaskBoardViewData {
            task: TaskBoardTask {
                id: "task-1".to_string(),
                name: "Board fixture".to_string(),
                status: "active".to_string(),
            },
            board: TaskBoardResponse {
                rev: 4,
                cards: vec![done, planned, blocked],
                ..TaskBoardResponse::default()
            },
            ready: TaskBoardReadyCards {
                ready: vec!["T-2".to_string()],
                blocked: vec!["T-3".to_string()],
                completed: vec!["T-1".to_string()],
                ..TaskBoardReadyCards::default()
            },
        }
    }

    #[test]
    fn dependency_state_marks_ready_and_blocked_cards() {
        let board = BoardSurface::loaded(data());
        let ready = state_label(&board, &board.cards()[1]);
        let blocked = state_label(&board, &board.cards()[2]);
        assert!(ready.contains("ready"));
        assert!(blocked.contains("blocked"));
        assert!(dependency_label(&board, &board.cards()[1]).contains("T-1"));
    }

    #[test]
    fn board_rendering_shows_ready_and_blocked_states() {
        let board = BoardSurface::loaded(data());
        let backend = ratatui::backend::TestBackend::new(120, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_task_board(frame, &board, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("ready"));
        assert!(text.contains("blocked"));
    }

    #[test]
    fn detail_contains_final_and_verifier_reports() {
        let mut board = BoardSurface::loaded(data());
        board.select_next();
        board.toggle_detail();
        let detail = detail_lines(&board)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(detail.contains("Final report"));
        assert!(detail.contains("Verifier report"));
        assert!(detail.contains("ship it"));
    }

    #[test]
    fn selected_card_exposes_agent_chat_navigation() {
        let mut board = BoardSurface::loaded(data());
        board.select_next();
        assert_eq!(
            board.selected_agent_chat(),
            Some(("agent-chat-2".to_string(), "T-2 title".to_string()))
        );
    }

    #[test]
    fn feature_flag_recognizes_truthy_values() {
        let previous = std::env::var_os("REFACT_TUI_SURFACES");
        std::env::set_var("REFACT_TUI_SURFACES", "on");
        assert!(task_board_enabled());
        std::env::set_var("REFACT_TUI_SURFACES", "0");
        assert!(!task_board_enabled());
        match previous {
            Some(value) => std::env::set_var("REFACT_TUI_SURFACES", value),
            None => std::env::remove_var("REFACT_TUI_SURFACES"),
        }
    }
}
