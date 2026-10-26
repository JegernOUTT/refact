use ratatui::layout::{Constraint, Direction, Layout, Rect, Size};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::app::App;

mod approval;
mod ask;
mod composer;
pub mod events;
pub mod footer;
pub(crate) mod goal_dock;
mod header;
mod help;
mod history;
pub(crate) mod menu;
mod overlay;
pub mod picker;
pub mod session_tabs;
mod settings;
pub mod status_card;
pub mod status_indicator;
mod transcript;
mod worktree;

const ASCII_FRAME_MAX_WIDTH: u16 = 60;
const BORDERLESS_MODAL_MAX_WIDTH: u16 = 39;
const COMPACT_MAX_WIDTH: u16 = 30;
const COMPACT_MAX_HEIGHT: u16 = 10;
const COMPACT_MIN_TRANSCRIPT_HEIGHT: u16 = 2;
const DIALOG_MIN_ROWS_ABOVE_COMPOSER: u16 = 8;
pub const BOX_DRAWING_GLYPHS: &str = concat!(
    "─━│┃┄┅┆┇┈┉┊┋┌┍┎┏┐┑┒┓└┕┖┗┘┙┚┛├┝┞┟",
    "┠┡┢┣┤┥┦┧┨┩┪┫┬┭┮┯┰┱┲┳┴┵┶┷┸┹┺┻┼┽┾┿",
    "╀╁╂╃╄╅╆╇╈╉╊╋╌╍╎╏═║╒╓╔╕╖╗╘╙╚╛╜╝╞╟",
    "╠╡╢╣╤╥╦╧╨╩╪╫╬╭╮╯╰╱╲╳╴╵╶╷╸╹╺╻╼╽╾╿",
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceLayer {
    Main,
    History,
    Board,
    Browser,
    Activity,
    AskForm,
    ProjectPicker,
    Settings,
    ModalPicker,
    Overlay,
    Goal,
    Approval,
    Worktree,
    Help,
}

impl SurfaceLayer {
    fn is_exclusive(self) -> bool {
        matches!(
            self,
            Self::History | Self::Board | Self::Browser | Self::Activity
        )
    }

    pub(crate) fn fills_screen(self) -> bool {
        self.is_exclusive() || matches!(self, Self::Overlay | Self::Help | Self::Settings)
    }

    fn dialog_rows(self) -> u16 {
        match self {
            Self::ModalPicker => 14,
            Self::ProjectPicker => 16,
            Self::Approval => 18,
            Self::Goal | Self::Worktree => 20,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InlineLayout {
    transcript: u16,
    status: u16,
    goal: u16,
    tabs: u16,
    composer: u16,
    footer: u16,
}

impl InlineLayout {
    fn total(self) -> u16 {
        self.transcript
            .saturating_add(self.status)
            .saturating_add(self.goal)
            .saturating_add(self.tabs)
            .saturating_add(self.composer)
            .saturating_add(self.footer)
    }
}

pub fn inline_viewport_height(app: &mut App, size: Size) -> u16 {
    let layer = app.surface_layer();
    if layer.fills_screen() {
        return size.height.max(1);
    }
    inline_layout(app, size.width, size.height, layer)
        .total()
        .clamp(1, size.height.max(1))
}

fn inline_layout(app: &mut App, width: u16, max_height: u16, layer: SurfaceLayer) -> InlineLayout {
    let footer = footer::desired_height(width);
    let screen_height = app.screen_height().max(max_height);
    let composer = match layer {
        SurfaceLayer::AskForm => app
            .ask_questions_form()
            .map(|form| ask::desired_height(form, max_height))
            .unwrap_or_else(|| composer::desired_height(app, width, screen_height)),
        _ => composer::desired_height(app, width, screen_height),
    };
    let tabs = session_tabs::height(app);
    let goal = goal_dock::height(app);
    let status = status_indicator::height(app, width).max(1);
    let fixed = footer
        .saturating_add(composer)
        .saturating_add(tabs)
        .saturating_add(goal)
        .saturating_add(status);
    let room = max_height.saturating_sub(fixed);
    let mut transcript = transcript::live_content_height(app, width).min(room);
    let mut dialog_rows = layer.dialog_rows();
    if layer == SurfaceLayer::Approval
        && app
            .approval_modal()
            .is_some_and(|modal| modal.details_open())
    {
        dialog_rows = 30;
    }
    if dialog_rows > 0 {
        let wanted = dialog_rows.min(max_height).saturating_sub(fixed);
        transcript = transcript.max(wanted);
    }
    InlineLayout {
        transcript,
        status,
        goal,
        tabs,
        composer,
        footer,
    }
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    crate::vendored::terminal_hyperlinks::clear_buffer_hyperlinks();
    app.begin_frame_render();
    let area = frame.area();
    let compact = area.width <= COMPACT_MAX_WIDTH && area.height <= COMPACT_MAX_HEIGHT;
    let layer = app.surface_layer();
    if layer.is_exclusive() {
        render_exclusive_surface(frame, app, area, layer, compact);
        return;
    }
    if app.native_scrollback() {
        render_inline(frame, app, area, layer, compact);
        return;
    }
    let session_tabs_height = session_tabs::height(app);
    let status_height = status_indicator::height(app, area.width);
    let goal_dock_height = goal_dock::height(app);
    let footer_height = footer::desired_height(area.width);
    let composer_height = match layer {
        SurfaceLayer::AskForm => app
            .ask_questions_form()
            .map(|form| ask::desired_height(form, area.height))
            .unwrap_or_else(|| composer::desired_height(app, area.width, area.height)),
        _ => composer::desired_height(app, area.width, area.height),
    };
    let events_open = app.events_pane().open && !compact;
    let main_constraints = if compact {
        compact_constraints(
            area.height,
            session_tabs_height,
            status_height,
            goal_dock_height,
            composer_height,
            footer_height,
        )
    } else if events_open {
        vec![
            Constraint::Length(1),
            Constraint::Length(session_tabs_height),
            Constraint::Percentage(62),
            Constraint::Percentage(38),
            Constraint::Length(status_height),
            Constraint::Length(goal_dock_height),
            Constraint::Length(composer_height),
            Constraint::Length(footer_height),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(session_tabs_height),
            Constraint::Min(1),
            Constraint::Length(status_height),
            Constraint::Length(goal_dock_height),
            Constraint::Length(composer_height),
            Constraint::Length(footer_height),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(main_constraints)
        .split(area);

    header::render_header(frame, app, chunks[0]);
    if session_tabs_height > 0 {
        session_tabs::render(frame, app, chunks[1]);
    }
    transcript::render_transcript(frame, app, chunks[2]);
    let composer_area = if events_open { chunks[6] } else { chunks[5] };
    if events_open {
        events::render_events_pane(frame, app, chunks[3]);
        status_indicator::render(frame, app, chunks[4]);
        goal_dock::render(frame, app, chunks[5]);
        render_composer_region(frame, app, chunks[6], layer);
        footer::render(frame, app, chunks[7]);
    } else {
        status_indicator::render(frame, app, chunks[3]);
        goal_dock::render(frame, app, chunks[4]);
        render_composer_region(frame, app, chunks[5], layer);
        footer::render(frame, app, chunks[6]);
    }
    render_modal_layer(frame, app, area, layer, composer_area);
    finish_frame(frame, area, compact, chunks[2]);
}

fn render_inline(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    layer: SurfaceLayer,
    compact: bool,
) {
    let layout = inline_layout(app, area.width, area.height, layer);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(layout.transcript),
            Constraint::Length(layout.status),
            Constraint::Length(layout.goal),
            Constraint::Length(layout.tabs),
            Constraint::Length(layout.composer),
            Constraint::Length(layout.footer),
        ])
        .split(area);
    transcript::render_transcript(frame, app, chunks[0]);
    status_indicator::render(frame, app, chunks[1]);
    goal_dock::render(frame, app, chunks[2]);
    if layout.tabs > 0 {
        session_tabs::render(frame, app, chunks[3]);
    }
    render_composer_region(frame, app, chunks[4], layer);
    footer::render(frame, app, chunks[5]);
    render_modal_layer(frame, app, area, layer, chunks[4]);
    finish_frame(frame, area, compact, chunks[0]);
}

fn render_modal_layer(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    layer: SurfaceLayer,
    composer_area: Rect,
) {
    let area = dialog_area(area, composer_area);
    match layer {
        SurfaceLayer::ProjectPicker => {
            picker::render_project_picker(frame, app.project_picker(), area);
        }
        SurfaceLayer::ModalPicker => {
            if let Some(picker) = app.modal_picker() {
                picker::render_modal_picker(frame, picker, area, composer_area);
            }
        }
        SurfaceLayer::Settings => settings::render_settings(frame, app, area),
        SurfaceLayer::Overlay => {
            app.set_transcript_overlay_visible_height(overlay::transcript_overlay_body_height(
                area,
            ));
            if let Some(overlay) = app.transcript_overlay() {
                overlay::render_transcript_overlay(frame, overlay, area);
            }
        }
        SurfaceLayer::Goal => goal_dock::render_overlay(frame, app, area),
        SurfaceLayer::Approval => {
            if let Some(modal) = app.approval_modal() {
                approval::render_approval_modal(frame, modal, area);
            }
        }
        SurfaceLayer::Worktree => worktree::render_worktree_merge_confirmation(frame, app, area),
        SurfaceLayer::Help => help::render_help(frame, app, area),
        SurfaceLayer::Main
        | SurfaceLayer::History
        | SurfaceLayer::Board
        | SurfaceLayer::Browser
        | SurfaceLayer::Activity
        | SurfaceLayer::AskForm => {}
    }
}

fn render_exclusive_surface(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    layer: SurfaceLayer,
    compact: bool,
) {
    match layer {
        SurfaceLayer::History => history::render_history_surface(frame, app, area),
        SurfaceLayer::Board => {
            if let Some(board) = app.task_board_surface() {
                crate::app::surfaces::board::render_task_board(frame, board, area);
            }
        }
        SurfaceLayer::Browser => {
            let state = app.browser_state().clone();
            if let Some(browser) = app.browser_surface() {
                crate::app::surfaces::browser::render_browser_surface(frame, &state, browser, area);
            }
        }
        SurfaceLayer::Activity => {
            if let Some(overlay) = app.transcript_overlay() {
                overlay::render_transcript_overlay(frame, overlay, area);
            }
        }
        _ => return,
    }
    finish_frame(frame, area, compact, area);
}

fn finish_frame(frame: &mut Frame<'_>, area: Rect, compact: bool, truncation_area: Rect) {
    degrade_frames(frame, area);
    if compact {
        render_compact_truncation_indicator(frame, truncation_area);
    }
}

fn compact_constraints(
    height: u16,
    session_tabs_height: u16,
    requested_status_height: u16,
    goal_dock_height: u16,
    requested_composer_height: u16,
    footer_height: u16,
) -> Vec<Constraint> {
    let body_height =
        height.saturating_sub(1 + session_tabs_height + footer_height + goal_dock_height);
    let composer_height =
        requested_composer_height.min(body_height.saturating_sub(COMPACT_MIN_TRANSCRIPT_HEIGHT));
    let status_height = requested_status_height
        .min(body_height.saturating_sub(composer_height + COMPACT_MIN_TRANSCRIPT_HEIGHT));
    let remaining_height = body_height.saturating_sub(composer_height + status_height);
    vec![
        Constraint::Length(1),
        Constraint::Length(session_tabs_height),
        Constraint::Length(remaining_height),
        Constraint::Length(status_height),
        Constraint::Length(goal_dock_height),
        Constraint::Length(composer_height),
        Constraint::Length(footer_height),
    ]
}

fn render_compact_truncation_indicator(frame: &mut Frame<'_>, area: Rect) {
    if area.is_empty() {
        return;
    }
    let indicator = Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width,
        height: 1,
    };
    Paragraph::new("… content truncated").render(indicator, frame.buffer_mut());
}

fn degrade_frames(frame: &mut Frame<'_>, area: Rect) {
    degrade_frame_buffer(frame.buffer_mut(), area);
}

fn degrade_frame_buffer(buffer: &mut ratatui::buffer::Buffer, area: Rect) {
    if area.width <= BORDERLESS_MODAL_MAX_WIDTH {
        remove_frame_borders(buffer, area);
    }
    if area.width < ASCII_FRAME_MAX_WIDTH {
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let symbol = buffer[(x, y)].symbol();
                let replacement = ascii_frame_replacement(symbol);
                if let Some(replacement) = replacement {
                    buffer[(x, y)].set_symbol(replacement);
                }
            }
        }
    }
}

fn remove_frame_borders(buffer: &mut ratatui::buffer::Buffer, area: Rect) {
    let mut borders = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if !matches!(buffer[(x, y)].symbol(), "┌" | "╭") {
                continue;
            }
            let Some(right) = ((x + 1)..area.right())
                .find(|right| matches!(buffer[(*right, y)].symbol(), "┐" | "╮"))
            else {
                continue;
            };
            let Some(bottom) = ((y + 1)..area.bottom()).find(|bottom| {
                matches!(buffer[(x, *bottom)].symbol(), "└" | "╰")
                    && matches!(buffer[(right, *bottom)].symbol(), "┘" | "╯")
            }) else {
                continue;
            };
            borders.push((x, y, right, bottom));
        }
    }
    for (left, top, right, bottom) in borders {
        for x in left..=right {
            buffer[(x, top)].set_symbol(" ");
            buffer[(x, bottom)].set_symbol(" ");
        }
        for y in top..=bottom {
            buffer[(left, y)].set_symbol(" ");
            buffer[(right, y)].set_symbol(" ");
        }
    }
}

fn ascii_frame_replacement(symbol: &str) -> Option<&'static str> {
    if symbol.is_empty() || !BOX_DRAWING_GLYPHS.contains(symbol) {
        return None;
    }
    Some(match symbol {
        "─" | "━" | "┄" | "┅" | "┈" | "┉" | "╌" | "╍" | "═" | "╴" | "╶" | "╸" | "╺" | "╼" | "╾" => {
            "-"
        }
        "│" | "┃" | "┆" | "┇" | "┊" | "┋" | "╎" | "╏" | "║" | "╵" | "╷" | "╹" | "╻" | "╽" | "╿" => {
            "|"
        }
        _ => "+",
    })
}

fn render_composer_region(frame: &mut Frame<'_>, app: &App, area: Rect, layer: SurfaceLayer) {
    if layer == SurfaceLayer::AskForm {
        if let Some(form) = app.ask_questions_form() {
            ask::render_ask_form(frame, form, area);
        }
    } else {
        composer::render_composer(frame, app, area);
    }
}

fn dialog_area(area: Rect, composer_area: Rect) -> Rect {
    let above = composer_area.y.saturating_sub(area.y);
    if above >= DIALOG_MIN_ROWS_ABOVE_COMPOSER {
        Rect {
            height: above,
            ..area
        }
    } else {
        area
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn popup_anchored_above(bounds: Rect, anchor_y: u16, width: u16, height: u16) -> Rect {
    let width = width.min(bounds.width);
    let height = height.min(bounds.height);
    let max_y = bounds
        .y
        .saturating_add(bounds.height.saturating_sub(height));
    let y = anchor_y.saturating_sub(height).clamp(bounds.y, max_y);
    Rect {
        x: bounds
            .x
            .saturating_add(bounds.width.saturating_sub(width) / 2),
        y,
        width,
        height,
    }
}
