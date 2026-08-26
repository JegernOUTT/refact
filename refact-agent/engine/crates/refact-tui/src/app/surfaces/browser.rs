use base64::Engine;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{BrowserContextDecisionOptions, BrowserState};
use crate::terminal_image::InlineImage;
use crate::terminal_probe::ImageProtocol;
use crate::text_safety::{sanitize_tool_inline, sanitize_tool_text};
use crate::ui::menu;

const FRAME_MARKER: &str = "[browser frame]";
const TIMELINE_LIMIT: usize = 6;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BrowserSurface {
    selected_context_field: usize,
    context_options: BrowserContextDecisionOptions,
    last_inline_frame: Option<(u64, ImageProtocol, u16, u16)>,
}

impl BrowserSurface {
    pub(crate) fn new() -> Self {
        Self {
            selected_context_field: 0,
            context_options: BrowserContextDecisionOptions::include_all(),
            last_inline_frame: None,
        }
    }

    pub(crate) fn move_context_selection(&mut self, offset: isize) {
        const FIELD_COUNT: usize = 5;
        self.selected_context_field = (self.selected_context_field as isize + offset)
            .rem_euclid(FIELD_COUNT as isize) as usize;
    }

    pub(crate) fn toggle_context_selection(&mut self) {
        match self.selected_context_field {
            0 => self.context_options.include_actions = !self.context_options.include_actions,
            1 => self.context_options.include_console = !self.context_options.include_console,
            2 => self.context_options.include_network = !self.context_options.include_network,
            3 => self.context_options.include_mutations = !self.context_options.include_mutations,
            4 => self.context_options.include_screenshot = !self.context_options.include_screenshot,
            _ => unreachable!("browser context field is always in range"),
        }
    }

    pub(crate) fn context_options(&self) -> BrowserContextDecisionOptions {
        self.context_options.clone()
    }

    pub(crate) fn next_inline_frame(
        &mut self,
        state: &BrowserState,
        buffer: &Buffer,
        protocol: Option<ImageProtocol>,
    ) -> Option<InlineImage> {
        let protocol = protocol?;
        if !state.is_open {
            return None;
        }
        let frame = state.latest_frame.as_ref()?;
        let position = crate::terminal_image::text_position(buffer, FRAME_MARKER)?;
        let token = (state.frame_version, protocol, position.x, position.y);
        if self.last_inline_frame == Some(token) {
            return None;
        }
        self.last_inline_frame = Some(token);
        let data = base64::engine::general_purpose::STANDARD
            .decode(&frame.data)
            .ok()?;
        Some(InlineImage::new(
            protocol,
            data,
            frame.mime.clone(),
            position,
        ))
    }
}

pub(crate) fn render_browser_surface(
    frame: &mut Frame<'_>,
    state: &BrowserState,
    surface: &BrowserSurface,
    area: Rect,
) {
    frame.render_widget(Clear, area);
    let inner = menu::render_menu_surface(area, frame.buffer_mut());
    if inner.is_empty() {
        return;
    }
    frame.render_widget(
        Paragraph::new(surface_lines(state, surface)).wrap(Wrap { trim: false }),
        inner,
    );
}

fn surface_lines(state: &BrowserState, surface: &BrowserSurface) -> Vec<Line<'static>> {
    let status = match (state.is_open, state.connected) {
        (true, true) => "open · connected",
        (true, false) => "open · disconnected",
        (false, _) => "closed",
    };
    let mut lines = vec![Line::from(format!("Browser · {status}"))];
    if let Some(runtime_id) = state.runtime_id.as_deref() {
        lines.push(Line::from(format!(
            "Runtime: {}",
            sanitize_tool_inline(runtime_id)
        )));
    }
    lines.push(Line::from(format!(
        "Title: {}",
        state
            .current_title
            .as_deref()
            .map(sanitize_tool_inline)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "(untitled)".to_string())
    )));
    lines.push(Line::from(format!(
        "URL: {}",
        state
            .current_url
            .as_deref()
            .map(sanitize_tool_inline)
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| "(none)".to_string())
    )));
    lines.push(Line::from(tab_summary(state)));
    lines.push(Line::from(""));
    push_frame_lines(&mut lines, state);
    lines.push(Line::from(""));
    push_timeline_lines(&mut lines, state);
    lines.push(Line::from(""));
    push_toolbar_history_lines(&mut lines, state);
    lines.push(Line::from(""));
    if let Some(prompt) = state.context_prompt.as_ref() {
        push_context_prompt_lines(&mut lines, prompt, surface);
    }
    lines
}

fn tab_summary(state: &BrowserState) -> String {
    let active_tab = state
        .active_tab
        .as_deref()
        .map(sanitize_tool_inline)
        .filter(|tab| !tab.is_empty())
        .unwrap_or_else(|| "none".to_string());
    format!("Tabs: {} · active {active_tab}", state.tabs.len())
}

fn push_frame_lines(lines: &mut Vec<Line<'static>>, state: &BrowserState) {
    let Some(frame) = state.latest_frame.as_ref() else {
        lines.push(Line::from("Frame: waiting for browser activity"));
        return;
    };
    let byte_count = decoded_base64_len(&frame.data);
    let description = if byte_count == 0 {
        format!("{} update", sanitize_tool_inline(&frame.mime))
    } else {
        format!(
            "{} · {} bytes",
            sanitize_tool_inline(&frame.mime),
            byte_count
        )
    };
    lines.push(Line::from(format!("Frame: {FRAME_MARKER} · {description}")));
    lines.push(Line::from(format!(
        "Tab: {} · {} changed regions",
        sanitize_tool_inline(&frame.tab_id),
        frame.diff_boxes.len()
    )));
    if let Some(changed_text) = frame
        .changed_text
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        lines.push(Line::from(format!(
            "Changed: {}",
            sanitize_tool_text(changed_text)
        )));
    }
}

fn push_timeline_lines(lines: &mut Vec<Line<'static>>, state: &BrowserState) {
    lines.push(Line::from("Timeline"));
    if state.timeline.is_empty() {
        lines.push(Line::from("No browser events yet."));
        return;
    }
    let skipped = state.timeline.len().saturating_sub(TIMELINE_LIMIT);
    if skipped > 0 {
        lines.push(Line::from(format!("… {skipped} earlier events")));
    }
    lines.extend(
        state
            .timeline
            .iter()
            .skip(skipped)
            .map(|event| Line::from(timeline_line(event))),
    );
}

fn push_toolbar_history_lines(lines: &mut Vec<Line<'static>>, state: &BrowserState) {
    lines.push(Line::from("Toolbar history"));
    if state.toolbar_actions.is_empty() {
        lines.push(Line::from("No toolbar actions observed."));
        return;
    }
    lines.extend(
        state
            .toolbar_actions
            .iter()
            .map(|action| Line::from(format!("• {}", sanitize_tool_inline(action)))),
    );
}

fn timeline_line(event: &serde_json::Value) -> String {
    let mut labels = ["timestamp", "source", "type"]
        .into_iter()
        .filter_map(|key| event.get(key).and_then(serde_json::Value::as_str))
        .map(sanitize_tool_inline)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if let Some(summary) = event.get("summary").and_then(serde_json::Value::as_str) {
        labels.push(sanitize_tool_inline(summary));
    }
    if labels.is_empty() {
        labels.push(sanitize_tool_inline(event.to_string()));
    }
    format!("• {}", labels.join(" · "))
}

fn push_context_prompt_lines(
    lines: &mut Vec<Line<'static>>,
    prompt: &crate::app::BrowserContextPromptState,
    surface: &BrowserSurface,
) {
    let event = &prompt.event;
    lines.push(Line::from(format!(
        "Context selection · {} bytes",
        event.total_bytes
    )));
    let choices = [
        (
            "Actions",
            surface.context_options.include_actions,
            event.action_count,
            event.action_bytes,
        ),
        (
            "Console",
            surface.context_options.include_console,
            event.console_count,
            event.console_bytes,
        ),
        (
            "Network",
            surface.context_options.include_network,
            event.network_count,
            event.network_bytes,
        ),
        (
            "Mutations",
            surface.context_options.include_mutations,
            0,
            event.mutation_bytes,
        ),
        (
            "Screenshot",
            surface.context_options.include_screenshot,
            0,
            0,
        ),
    ];
    for (index, (label, enabled, count, bytes)) in choices.into_iter().enumerate() {
        let selected = if surface.selected_context_field == index {
            "›"
        } else {
            " "
        };
        let state = if enabled { "include" } else { "exclude" };
        let details = if count == 0 {
            format!("{bytes} bytes")
        } else {
            format!("{count} items · {bytes} bytes")
        };
        lines.push(Line::from(format!(
            "{selected} {label}: {state} · {details}"
        )));
    }
    lines.push(Line::from(
        "Space toggle · Enter send selection · Esc close",
    ));
}

fn decoded_base64_len(data: &str) -> usize {
    let padding = data
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    (data.len().saturating_mul(3) / 4).saturating_sub(padding.min(2))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::json;

    use super::*;
    use crate::app::App;
    use crate::client::{ChatEvent, OpenProjectResponse};

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "fixture".to_string(),
            root: PathBuf::from("/tmp/fixture"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    fn browser_app() -> App {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_status".to_string(),
            raw: json!({
                "type": "browser_status",
                "runtime_id": "browser-1",
                "connected": true,
                "active_tab": "tab-1",
                "url": "https://example.test/checkout",
                "title": "Checkout",
                "tabs": [{"tab_id": "tab-1", "url": "https://example.test/checkout", "title": "Checkout"}]
            }),
        });
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_frame".to_string(),
            raw: json!({
                "type": "browser_frame",
                "tab_id": "tab-1",
                "mime": "image/png",
                "data": "QUJDRA==",
                "diff_boxes": [{"x": 1, "y": 2, "width": 3, "height": 4}],
                "changed_text": "Clicked checkout"
            }),
        });
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_timeline".to_string(),
            raw: json!({
                "type": "browser_timeline",
                "events": [{"timestamp": "12:00", "source": "agent", "type": "click", "summary": "Clicked checkout"}]
            }),
        });
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "browser_toolbar_action".to_string(),
            raw: json!({"type": "browser_toolbar_action", "action": "screenshot"}),
        });
        app.browser_surface = Some(BrowserSurface::new());
        app
    }

    fn render_app(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn browser_surface_renders_status_url_timeline_frame_and_toolbar() {
        let mut app = browser_app();

        let rendered = render_app(&mut app, 120, 40);

        for expected in [
            "Browser · open · connected",
            "https://example.test/checkout",
            "Checkout",
            FRAME_MARKER,
            "Clicked checkout",
            "Timeline",
            "Toolbar history",
            "• screenshot",
        ] {
            assert!(rendered.contains(expected), "{expected}: {rendered}");
        }
    }

    #[test]
    fn browser_surface_renders_toolbar_history_in_chronological_order() {
        let mut app = browser_app();
        let chat_id = app.chat_id().to_string();
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "browser_toolbar_action".to_string(),
            raw: json!({"type": "browser_toolbar_action", "action": "refresh"}),
        });
        let surface = app.browser_surface.as_ref().unwrap();
        let lines = surface_lines(app.browser_state(), surface)
            .into_iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let screenshot = lines
            .iter()
            .position(|line| line == "• screenshot")
            .unwrap();
        let refresh = lines.iter().position(|line| line == "• refresh").unwrap();
        assert!(screenshot < refresh);
    }

    #[test]
    fn frame_updates_do_not_place_payload_or_escape_bytes_in_cells() {
        let mut app = browser_app();
        let _ = render_app(&mut app, 100, 30);
        let chat_id = app.chat_id().to_string();
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "browser_frame".to_string(),
            raw: json!({
                "type": "browser_frame",
                "tab_id": "tab-1",
                "mime": "image/png",
                "data": "\u{001b}_Gunsafe_payload",
                "changed_text": "\u{001b}[2JUpdated safely"
            }),
        });

        let rendered = render_app(&mut app, 100, 30);

        assert!(rendered.contains("Updated safely"));
        assert!(!rendered.contains("unsafe_payload"));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn unsupported_terminals_keep_the_text_first_frame() {
        let mut app = browser_app();
        let rendered = render_app(&mut app, 100, 30);
        let surface = app.browser_surface.as_mut().unwrap();
        let frame = app.browser_state.latest_frame.clone().unwrap();
        let state = BrowserState {
            latest_frame: Some(frame),
            ..BrowserState::default()
        };
        let buffer = Buffer::empty(Rect::new(0, 0, 1, 1));

        assert!(rendered.contains(FRAME_MARKER));
        assert!(surface.next_inline_frame(&state, &buffer, None).is_none());
    }

    #[test]
    fn supported_terminals_emit_each_frame_once_after_buffer_rendering() {
        let mut surface = BrowserSurface::new();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 1));
        buffer.set_string(2, 0, FRAME_MARKER, ratatui::style::Style::default());
        let state = BrowserState {
            is_open: true,
            frame_version: 4,
            latest_frame: Some(crate::protocol::BrowserFrameEvent {
                tab_id: "tab-1".to_string(),
                mime: "image/png".to_string(),
                data: "QUJDRA==".to_string(),
                ..Default::default()
            }),
            ..BrowserState::default()
        };

        assert!(surface
            .next_inline_frame(&state, &buffer, Some(ImageProtocol::Kitty))
            .is_some());
        assert!(surface
            .next_inline_frame(&state, &buffer, Some(ImageProtocol::Kitty))
            .is_none());
        assert!(buffer
            .content()
            .iter()
            .all(|cell| !cell.symbol().contains('\u{1b}')));
    }

    #[test]
    fn browser_surface_degrades_at_forty_by_fifteen() {
        let mut app = browser_app();
        let rendered = render_app(&mut app, 40, 15);

        assert!(rendered.contains("Browser"), "{rendered}");
        assert!(rendered.contains("URL:"), "{rendered}");
        assert!(!rendered.chars().any(|character| {
            matches!(
                character,
                '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '─' | '│'
            )
        }));
    }

    #[test]
    fn context_prompt_selection_toggles_and_submits_the_visible_choice() {
        let mut app = browser_app();
        let chat_id = app.chat_id().to_string();
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "browser_context_oversize".to_string(),
            raw: json!({
                "type": "browser_context_oversize",
                "total_bytes": 1000,
                "action_count": 2,
                "action_bytes": 200,
                "pending_message_id": "pending-1"
            }),
        });

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty())),
            crate::app::AppAction::None
        );
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));

        assert!(matches!(
            action,
            crate::app::AppAction::SendBrowserContextDecision { decision, .. }
                if !decision.include_actions && decision.include_console
        ));
    }
}
