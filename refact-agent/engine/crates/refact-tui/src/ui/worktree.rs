use ratatui::layout::Rect;
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Frame;

use crate::app::App;
use crate::ui::{centered, menu};

pub(crate) fn render_worktree_merge_confirmation(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(lines) = app.worktree_merge_confirmation_lines() else {
        return;
    };
    let width = area.width.saturating_sub(6).min(88).max(24).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(4).max(1));
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    Paragraph::new(lines.join("\n")).render(inner, frame.buffer_mut());
}
