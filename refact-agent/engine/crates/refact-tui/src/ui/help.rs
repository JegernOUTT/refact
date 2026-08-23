use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::key_hint;
use crate::keymap::{HelpRow, KeyContext};
use crate::style::user_message_style;
use crate::theme::ThemeRole;

pub(crate) fn render_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = help_rows_for_display(app.keymap_help_rows());
    let row_count = rows.len().min(24) as u16;
    let popup = super::centered(
        area,
        area.width.saturating_sub(8).min(92),
        row_count.saturating_add(5).min(area.height),
    );
    frame.render_widget(Clear, popup);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Help",
        app.theme().style(ThemeRole::Accent),
    )));
    lines.push(Line::from(vec![
        Span::styled("Theme ", app.theme().style(ThemeRole::Muted)),
        Span::raw(app.theme().name().to_string()),
        Span::styled(" · vim ", app.theme().style(ThemeRole::Muted)),
        Span::raw(if app.vim_enabled() {
            app.vim_mode().label().to_string()
        } else {
            "off".to_string()
        }),
    ]));
    lines.push(Line::from(""));
    for row in rows {
        lines.push(help_row_line(row, app));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(user_message_style()),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn help_rows_for_display(rows: Vec<HelpRow>) -> Vec<HelpRow> {
    KeyContext::ALL
        .into_iter()
        .filter_map(|context| {
            let action =
                (context == KeyContext::Main).then_some(crate::keymap::KeyAction::ShowHelp);
            rows.iter()
                .find(|row| {
                    row.context == context && action.is_none_or(|action| row.action == action)
                })
                .cloned()
        })
        .collect()
}

pub(crate) fn help_row_line(row: HelpRow, app: &App) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{:>12} ", row.context.label()),
        app.theme().style(ThemeRole::Muted),
    )];

    let mut key_width = 0usize;
    for (index, binding) in row.bindings.split(',').map(str::trim).enumerate() {
        if index > 0 {
            spans.push(Span::styled(", ", app.theme().style(ThemeRole::Muted)));
            key_width += 2;
        }
        spans.push(key_hint::key_with_theme(app.theme(), binding.to_string()));
        key_width += binding.len();
    }

    let gap = 22usize.saturating_sub(key_width).max(2);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::styled(
        row.description.to_string(),
        app.theme()
            .style(ThemeRole::Muted)
            .add_modifier(Modifier::DIM),
    ));

    Line::from(spans)
}
