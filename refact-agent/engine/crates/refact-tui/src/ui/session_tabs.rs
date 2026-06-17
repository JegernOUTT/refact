use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::keymap::{KeyAction, KeyContext, KeymapRegistry};
use crate::sessions::SessionTab;
use crate::text_safety::truncate_graphemes;
use crate::theme::{ThemeRole, TuiTheme};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const MAX_TAB_TITLE_GRAPHEMES: usize = 24;

pub(crate) fn height(app: &App) -> u16 {
    height_for_tabs(&app.session_tabs())
}

pub(crate) fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let tabs = app.session_tabs();
    let Some(line) = session_tabs_line(
        &tabs,
        app.theme(),
        &key_label(
            app.keymap(),
            KeyContext::Main,
            KeyAction::PreviousSession,
            "F6",
        ),
        &key_label(app.keymap(), KeyContext::Main, KeyAction::NextSession, "F7"),
    ) else {
        return;
    };
    let line = truncate_line_with_ellipsis_if_overflow(line, area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

pub(crate) fn height_for_tabs(tabs: &[SessionTab]) -> u16 {
    if tabs.len() > 1 {
        1
    } else {
        0
    }
}

pub(crate) fn session_tabs_line(
    tabs: &[SessionTab],
    theme: &TuiTheme,
    previous_key: &str,
    next_key: &str,
) -> Option<Line<'static>> {
    if height_for_tabs(tabs) == 0 {
        return None;
    }
    let accent = theme.style(ThemeRole::Accent);
    let muted = theme.style(ThemeRole::Muted);
    let current = accent.add_modifier(Modifier::BOLD | Modifier::REVERSED);
    let mut spans = vec![
        Span::styled(" chats ", muted),
        Span::styled(previous_key.to_string(), accent),
        Span::styled("/", muted),
        Span::styled(next_key.to_string(), accent),
        Span::styled(" switch ", muted),
    ];
    for (index, tab) in tabs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ", muted));
        }
        let title = truncate_graphemes(&tab.title, MAX_TAB_TITLE_GRAPHEMES).0;
        if tab.is_current {
            spans.push(Span::styled(format!("[{title}]"), current));
        } else {
            spans.push(Span::styled(
                format!(" {title} "),
                inactive_tab_style(muted),
            ));
        }
    }
    Some(Line::from(spans))
}

fn inactive_tab_style(muted: Style) -> Style {
    muted
}

fn key_label(
    keymap: &KeymapRegistry,
    context: KeyContext,
    action: KeyAction,
    fallback: &str,
) -> String {
    keymap
        .binding_label(context, action)
        .and_then(|label| label.split('/').next().map(str::to_string))
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, title: &str, is_current: bool) -> SessionTab {
        SessionTab {
            id: id.to_string(),
            title: title.to_string(),
            description: String::new(),
            is_current,
        }
    }

    #[test]
    fn strip_renders_sessions_with_current_highlighted() {
        let tabs = vec![tab("a", "Alpha", false), tab("b", "Beta", true)];

        let line = session_tabs_line(&tabs, &TuiTheme::dark(), "F6", "F7").unwrap();
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let current = line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "[Beta]")
            .unwrap();

        assert!(text.contains("Alpha"));
        assert!(text.contains("[Beta]"));
        assert!(current.style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn empty_and_single_session_state_hides_strip() {
        assert_eq!(height_for_tabs(&[]), 0);
        assert_eq!(height_for_tabs(&[tab("a", "Alpha", true)]), 0);
        assert_eq!(
            height_for_tabs(&[tab("a", "Alpha", true), tab("b", "Beta", false)]),
            1
        );
        assert!(session_tabs_line(&[], &TuiTheme::dark(), "F6", "F7").is_none());
    }
}
