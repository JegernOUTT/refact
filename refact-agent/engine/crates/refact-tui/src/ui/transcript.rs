use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::app::{App, TranscriptItem};
use crate::history::cells::cell_from_transcript_item;
use crate::render::renderable::{ColumnRenderable, InsetRenderable, Renderable};
use crate::render::Insets;
use crate::vendored::terminal_hyperlinks::{
    hyperlinks_enabled_from_env, mark_buffer_hyperlinks, visible_lines, HyperlinkLine,
};

const TRANSCRIPT_GUTTER: u16 = 2;

pub(crate) fn render_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.native_scrollback() {
        render_live_transcript(frame, app, area);
    } else {
        render_full_transcript(frame, app, area);
    }
}

pub(crate) fn render_live_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    render_transcript_view(
        frame,
        app,
        area,
        "History is in native scrollback. Start typing below.",
    );
}

pub(crate) fn render_full_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    render_transcript_view(
        frame,
        app,
        area,
        "Start typing. Enter sends, Shift-Enter inserts a newline.",
    );
}

fn render_transcript_view(frame: &mut Frame<'_>, app: &mut App, area: Rect, empty_hint: &str) {
    if area.is_empty() {
        return;
    }
    let children = prepare_transcript_children(app, area.width, empty_hint);
    app.note_rendered_messages(app.visible_transcript().len());
    render_prepared_children(frame.buffer_mut(), area, children, app.scroll_offset());
}

fn prepare_transcript_children(app: &App, width: u16, empty_hint: &str) -> Vec<PreparedRenderable> {
    if app.visible_transcript().is_empty() {
        return vec![prepare_hint(width, empty_hint)];
    }
    let mut children = Vec::new();
    let content_width = width.saturating_sub(TRANSCRIPT_GUTTER).max(1) as usize;
    for (index, item) in app.visible_transcript().iter().enumerate() {
        if let Some(child) =
            prepare_item_child(item, index, app, width, content_width, children.is_empty())
        {
            children.push(child);
        }
    }
    if children.is_empty() {
        vec![prepare_hint(width, empty_hint)]
    } else {
        children
    }
}

fn prepare_item_child(
    item: &TranscriptItem,
    index: usize,
    app: &App,
    width: u16,
    content_width: usize,
    first_visible_cell: bool,
) -> Option<PreparedRenderable> {
    let cell = cell_from_transcript_item(item, app.transcript_item_selected(index, item));
    let top = if first_visible_cell || cell.is_stream_continuation() {
        0
    } else {
        1
    };
    let lines = cell.display_hyperlink_lines(content_width);
    if lines.is_empty() {
        return None;
    }
    let child: Box<dyn Renderable> = Box::new(HyperlinkLinesRenderable::new(lines));
    let renderable: Box<dyn Renderable> = Box::new(InsetRenderable::new(
        child,
        Insets::tlbr(top, TRANSCRIPT_GUTTER, 0, 0),
    ));
    PreparedRenderable::new(renderable, width)
}

fn prepare_hint(width: u16, empty_hint: &str) -> PreparedRenderable {
    let lines = vec![HyperlinkLine::new(Line::from(Span::styled(
        empty_hint.to_string(),
        Style::default().fg(Color::DarkGray),
    )))];
    PreparedRenderable::new(Box::new(HyperlinkLinesRenderable::new(lines)), width)
        .expect("empty transcript hint has height")
}

fn render_prepared_children(
    buffer: &mut Buffer,
    area: Rect,
    children: Vec<PreparedRenderable>,
    scroll_offset: usize,
) {
    Clear.render(area, buffer);
    if area.is_empty() {
        return;
    }
    let total_height = children
        .iter()
        .map(|child| usize::from(child.height))
        .sum::<usize>();
    let (start, end) = visible_bounds(total_height, usize::from(area.height), scroll_offset);
    if start >= end {
        return;
    }
    let mut column = ColumnRenderable::new();
    let mut cursor = 0usize;
    for child in children {
        let child_start = cursor;
        let child_end = child_start.saturating_add(usize::from(child.height));
        cursor = child_end;
        let visible_start = child_start.max(start);
        let visible_end = child_end.min(end);
        if visible_start >= visible_end {
            continue;
        }
        column.push(SlicedRenderable {
            child: child.renderable,
            full_height: child.height,
            skip_top: saturating_u16(visible_start - child_start),
            visible_height: saturating_u16(visible_end - visible_start),
        });
    }
    column.render(area, buffer);
}

fn visible_bounds(
    total_height: usize,
    viewport_height: usize,
    scroll_offset: usize,
) -> (usize, usize) {
    let start = total_height
        .saturating_sub(viewport_height)
        .saturating_sub(scroll_offset);
    let end = total_height.saturating_sub(scroll_offset).min(total_height);
    (start, end)
}

fn saturating_u16(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

struct PreparedRenderable {
    renderable: Box<dyn Renderable>,
    height: u16,
}

impl PreparedRenderable {
    fn new(renderable: Box<dyn Renderable>, width: u16) -> Option<Self> {
        let height = renderable.desired_height(width);
        (height > 0).then_some(Self { renderable, height })
    }
}

struct HyperlinkLinesRenderable {
    lines: Vec<HyperlinkLine>,
}

impl HyperlinkLinesRenderable {
    fn new(lines: Vec<HyperlinkLine>) -> Self {
        Self { lines }
    }
}

impl Renderable for HyperlinkLinesRenderable {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let view = self
            .lines
            .iter()
            .take(usize::from(area.height))
            .cloned()
            .collect::<Vec<_>>();
        Paragraph::new(Text::from(visible_lines(view)))
            .wrap(Wrap { trim: false })
            .render(area, buffer);
        mark_buffer_hyperlinks(buffer, area, &self.lines, hyperlinks_enabled_from_env());
    }

    fn desired_height(&self, width: u16) -> u16 {
        if width == 0 {
            return 0;
        }
        saturating_u16(
            Paragraph::new(Text::from(visible_lines(self.lines.clone())))
                .wrap(Wrap { trim: false })
                .line_count(width),
        )
    }
}

struct SlicedRenderable {
    child: Box<dyn Renderable>,
    full_height: u16,
    skip_top: u16,
    visible_height: u16,
}

impl Renderable for SlicedRenderable {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let height = self.visible_height.min(area.height);
        if height == 0 || area.width == 0 {
            return;
        }
        let target = Rect::new(area.x, area.y, area.width, height);
        if self.skip_top == 0 && height == self.full_height {
            self.child.render(target, buffer);
            return;
        }
        let temp_area = Rect::new(0, 0, area.width, self.full_height);
        let mut temp = Buffer::empty(temp_area);
        self.child.render(temp_area, &mut temp);
        for row in 0..height {
            let source_y = self.skip_top.saturating_add(row);
            if source_y >= self.full_height {
                break;
            }
            for column in 0..area.width {
                let cell = temp[(column, source_y)].clone();
                buffer[(
                    target.x.saturating_add(column),
                    target.y.saturating_add(row),
                )]
                    .clone_from(&cell);
            }
        }
    }

    fn desired_height(&self, _width: u16) -> u16 {
        self.visible_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TranscriptItem;
    use crate::client::OpenProjectResponse;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: false,
            worker: None,
            cron_pending: None,
        }
    }

    fn buffer_rows(buffer: &Buffer) -> Vec<String> {
        (buffer.area.top()..buffer.area.bottom())
            .map(|y| {
                (buffer.area.left()..buffer.area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn text_child(text: &'static str) -> PreparedRenderable {
        PreparedRenderable::new(
            Box::new(HyperlinkLinesRenderable::new(vec![HyperlinkLine::new(
                Line::from(text),
            )])),
            16,
        )
        .unwrap()
    }

    #[test]
    fn full_transcript_ignores_empty_or_zero_width_area() {
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        let backend = TestBackend::new(8, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_full_transcript(frame, &mut app, Rect::new(0, 0, 0, frame.area().height));
                render_full_transcript(frame, &mut app, Rect::new(0, 0, frame.area().width, 0));
            })
            .unwrap();

        assert_eq!(app.rendered_message_count(), 0);
    }

    #[test]
    fn full_transcript_composes_gutters_spacers_and_no_bottom_border() {
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        app.test_push_history_item(TranscriptItem::Notice("one".to_string()));
        let backend = TestBackend::new(48, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_full_transcript(frame, &mut app, frame.area()))
            .unwrap();

        let rows = buffer_rows(terminal.backend().buffer());
        let text = rows.join("\n");
        let opened = rows
            .iter()
            .position(|row| row.contains("Opened project"))
            .unwrap();
        let one = rows.iter().position(|row| row.contains("• one")).unwrap();
        assert!(!text.contains('─'));
        assert!(rows[opened].starts_with("  •"));
        assert!(rows[one].starts_with("  •"));
        assert_eq!(one, opened + 3);
    }

    #[test]
    fn composed_transcript_window_applies_scroll_offset() {
        let area = Rect::new(0, 0, 16, 2);
        let mut bottom = Buffer::empty(area);
        render_prepared_children(
            &mut bottom,
            area,
            vec![text_child("one"), text_child("two"), text_child("three")],
            0,
        );
        assert_eq!(
            buffer_rows(&bottom)
                .into_iter()
                .map(|row| row.trim_end().to_string())
                .collect::<Vec<_>>(),
            vec!["two".to_string(), "three".to_string()]
        );

        let mut scrolled = Buffer::empty(area);
        render_prepared_children(
            &mut scrolled,
            area,
            vec![text_child("one"), text_child("two"), text_child("three")],
            1,
        );
        assert_eq!(
            buffer_rows(&scrolled)
                .into_iter()
                .map(|row| row.trim_end().to_string())
                .collect::<Vec<_>>(),
            vec!["one".to_string(), "two".to_string()]
        );
    }
}
