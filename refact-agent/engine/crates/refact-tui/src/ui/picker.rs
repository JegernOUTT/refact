use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use ratatui::Frame;

use crate::app::ProjectPickerState;
use crate::client::ProjectEntry;
use crate::key_hint;
use crate::pickers::{PickerKind, PickerState};
use crate::style::accent_style;
use crate::ui::menu::{
    self, ColumnWidthConfig, ColumnWidthMode, GenericDisplayRow, ScrollState, MAX_POPUP_ROWS,
};

pub(crate) fn render_project_picker(
    frame: &mut Frame<'_>,
    picker: &ProjectPickerState,
    area: Rect,
) {
    let filtered = picker.filtered_projects();
    let rows = filtered
        .iter()
        .enumerate()
        .map(|(idx, project)| project_row(idx, picker.selected, project))
        .collect::<Vec<_>>();
    let width = area.width.saturating_sub(8).min(80).max(1);
    let height = popup_height_for_rows(&rows, picker.selected, width, area.height);
    let popup = super::centered(area, width, height);
    frame.render_widget(Clear, popup);
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    render_picker_content(
        frame,
        inner,
        Line::from(format!("projects: {}", picker.filter)),
        rows,
        picker.selected,
        menu::standard_popup_hint_line(),
        "No projects match",
    );
}

fn project_row(idx: usize, selected: usize, project: &ProjectEntry) -> GenericDisplayRow {
    let mut prefix = vec![Span::raw(cursor_prefix(idx, selected))];
    if project.pinned {
        prefix.push(Span::styled("★ ", accent_style()));
    }
    GenericDisplayRow {
        name: project.slug.clone(),
        name_style: Some(Style::default().add_modifier(Modifier::BOLD)),
        name_prefix_spans: prefix,
        description: Some(project.root.display().to_string()),
        ..Default::default()
    }
}

pub fn render_modal_picker(
    frame: &mut Frame<'_>,
    picker: &PickerState,
    area: Rect,
    composer: Rect,
) {
    if matches!(
        picker.kind,
        PickerKind::SlashCommand | PickerKind::FileMention
    ) {
        render_composer_popup(frame, picker, area, composer);
        return;
    }

    let filtered = picker.filtered_items();
    let rows = filtered
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let name = if picker.is_multi() {
                let checkbox = if picker.is_selected(&item.id) {
                    "☑"
                } else {
                    "☐"
                };
                format!("{checkbox} {}", item.title)
            } else {
                item.title.clone()
            };
            GenericDisplayRow {
                name,
                name_prefix_spans: vec![Span::raw(cursor_prefix(idx, picker.selected))],
                description: (!item.description.is_empty()).then(|| item.description.clone()),
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();
    let width = area.width.saturating_sub(8).min(86).max(1);
    let height = popup_height_for_rows(&rows, picker.selected, width, area.height);
    let max_y = area.y.saturating_add(area.height.saturating_sub(height));
    let wanted_y = composer.y.saturating_sub(height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: wanted_y.min(max_y),
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    let title = if picker.is_multi() {
        format!(
            "{}: {} selected · {}",
            picker.title(),
            picker.selected_count(),
            picker.filter
        )
    } else {
        format!("{}: {}", picker.title(), picker.filter)
    };
    let footer = if picker.is_multi() {
        multi_picker_hint_line()
    } else {
        menu::standard_popup_hint_line()
    };
    render_picker_content(
        frame,
        inner,
        Line::from(title),
        rows,
        picker.selected,
        footer,
        "No entries match",
    );
}

fn render_composer_popup(frame: &mut Frame<'_>, picker: &PickerState, area: Rect, composer: Rect) {
    let rows = composer_rows(picker);
    let visible_rows = rows.len().max(1).min(MAX_POPUP_ROWS);
    let width = area.width.saturating_sub(8).min(86).max(1);
    let height = visible_rows
        .saturating_add(2)
        .min(area.height.saturating_sub(2).max(1) as usize) as u16;
    let max_y = area.y.saturating_add(area.height.saturating_sub(height));
    let wanted_y = composer.y.saturating_sub(height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: wanted_y.min(max_y),
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    let mut state = ScrollState {
        selected_idx: (!rows.is_empty())
            .then_some(picker.selected.min(rows.len().saturating_sub(1))),
        scroll_top: 0,
    };
    state.clamp_selection(rows.len());
    state.ensure_visible(rows.len(), visible_rows);
    let empty_message = composer_empty_message(picker);
    match picker.kind {
        PickerKind::SlashCommand => {
            menu::render_rows_single_line_with_config(
                inner,
                frame.buffer_mut(),
                &rows,
                &state,
                MAX_POPUP_ROWS,
                empty_message,
                ColumnWidthConfig::new(ColumnWidthMode::AutoAllRows, None),
            );
        }
        PickerKind::FileMention => {
            menu::render_rows_single_line(
                inner,
                frame.buffer_mut(),
                &rows,
                &state,
                MAX_POPUP_ROWS,
                empty_message,
            );
        }
        _ => {}
    }
}

fn composer_rows(picker: &PickerState) -> Vec<GenericDisplayRow> {
    match picker.kind {
        PickerKind::SlashCommand => picker
            .filtered_items()
            .into_iter()
            .map(|item| {
                let name = slash_command_name(&item.title, &item.id);
                GenericDisplayRow {
                    match_indices: match_indices_for_filter(&name, &picker.filter),
                    name,
                    description: (!item.description.is_empty()).then_some(item.description),
                    ..Default::default()
                }
            })
            .collect(),
        PickerKind::FileMention if file_picker_is_loading(picker) => Vec::new(),
        PickerKind::FileMention => picker
            .filtered_items()
            .into_iter()
            .filter(|item| !item.id.is_empty())
            .map(|item| GenericDisplayRow {
                name: if item.id.is_empty() {
                    item.title
                } else {
                    item.id
                },
                ..Default::default()
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn composer_empty_message(picker: &PickerState) -> &'static str {
    match picker.kind {
        PickerKind::FileMention if file_picker_is_loading(picker) => "loading...",
        _ => "no matches",
    }
}

fn file_picker_is_loading(picker: &PickerState) -> bool {
    picker.kind == PickerKind::FileMention
        && picker.items().len() == 1
        && picker.items()[0].id.is_empty()
        && picker.items()[0]
            .title
            .to_ascii_lowercase()
            .contains("loading")
}

fn slash_command_name(title: &str, id: &str) -> String {
    let name = if title.trim().is_empty() { id } else { title };
    if name.starts_with('/') {
        name.to_string()
    } else {
        format!("/{name}")
    }
}

fn match_indices_for_filter(name: &str, filter: &str) -> Option<Vec<usize>> {
    let needle = filter.trim().trim_start_matches('/').to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    let haystack = name.to_ascii_lowercase();
    if let Some(start) = haystack.find(&needle) {
        let start_chars = haystack[..start].chars().count();
        return Some((start_chars..start_chars + needle.chars().count()).collect());
    }
    let trimmed = name.trim_start_matches('/').to_ascii_lowercase();
    let Some(start) = trimmed.find(&needle) else {
        return fuzzy_match_indices(name, &needle);
    };
    let slash_offset = usize::from(name.starts_with('/'));
    Some((slash_offset + start..slash_offset + start + needle.chars().count()).collect())
}

fn fuzzy_match_indices(name: &str, needle: &str) -> Option<Vec<usize>> {
    let mut wanted = needle.chars();
    let mut current = wanted.next()?;
    let mut indices = Vec::new();
    for (idx, ch) in name.chars().enumerate() {
        if ch.to_ascii_lowercase() == current {
            indices.push(idx);
            match wanted.next() {
                Some(next) => current = next,
                None => return Some(indices),
            }
        }
    }
    None
}

fn popup_height_for_rows(
    rows: &[GenericDisplayRow],
    selected: usize,
    width: u16,
    available_height: u16,
) -> u16 {
    let mut state = ScrollState {
        selected_idx: (!rows.is_empty()).then_some(selected.min(rows.len().saturating_sub(1))),
        scroll_top: 0,
    };
    let visible_rows = rows.len().max(1).min(MAX_POPUP_ROWS);
    state.clamp_selection(rows.len());
    state.ensure_visible(rows.len(), visible_rows);
    let content_width = width.saturating_sub(4).max(1);
    let rows_height = menu::measure_rows_height(rows, &state, visible_rows, content_width)
        .max(1)
        .min(MAX_POPUP_ROWS as u16);
    rows_height
        .saturating_add(4)
        .min(available_height.saturating_sub(2).max(1))
}

fn cursor_prefix(idx: usize, selected: usize) -> String {
    if idx == selected {
        "› ".to_string()
    } else {
        "  ".to_string()
    }
}

fn render_picker_content(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    rows: Vec<GenericDisplayRow>,
    selected: usize,
    footer: Line<'static>,
    empty_message: &str,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let title_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    title.bold().render(title_area, frame.buffer_mut());

    let footer_height = u16::from(area.height > 1);
    let rows_area = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1 + footer_height),
    };
    let mut state = ScrollState {
        selected_idx: (!rows.is_empty()).then_some(selected.min(rows.len().saturating_sub(1))),
        scroll_top: 0,
    };
    let visible_rows = rows_area.height.max(1).min(MAX_POPUP_ROWS as u16) as usize;
    state.clamp_selection(rows.len());
    state.ensure_visible(rows.len(), visible_rows);
    menu::render_rows(
        rows_area,
        frame.buffer_mut(),
        &rows,
        &state,
        visible_rows,
        empty_message,
    );

    if footer_height > 0 {
        let footer_area = Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: area.width,
            height: 1,
        };
        footer.dim().render(footer_area, frame.buffer_mut());
    }
}

fn multi_picker_hint_line() -> Line<'static> {
    Line::from(vec![
        "Press ".into(),
        key_hint::plain("Space"),
        " to toggle; ".into(),
        key_hint::plain("Enter"),
        " to confirm or ".into(),
        key_hint::plain("Esc"),
        " to go back".into(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ProjectEntry;
    use crate::pickers::{PickerItem, PickerKind};
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    fn item(id: &str, title: &str, description: &str) -> PickerItem {
        PickerItem {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
        }
    }

    fn project_entry(slug: &str, root: &str, pinned: bool) -> ProjectEntry {
        ProjectEntry {
            id: slug.to_string(),
            slug: slug.to_string(),
            root: root.into(),
            pinned,
            last_active_ms: 0,
            settings: serde_json::Value::Null,
        }
    }

    #[test]
    fn project_rows_keep_daemon_fields_and_mark_pinned_projects() {
        let project = project_entry("demo", "/tmp/demo", true);

        let row = project_row(0, 0, &project);

        assert_eq!(row.name, "demo");
        assert_eq!(row.description.as_deref(), Some("/tmp/demo"));
        assert_eq!(row.name_prefix_spans[1].content.as_ref(), "★ ");
        assert_eq!(row.name_prefix_spans[1].style.fg, Some(Color::Cyan));
        assert!(row
            .name_style
            .unwrap()
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn modal_picker_renders_deboxed_accent_row_and_footer() {
        let picker = PickerState::new(
            PickerKind::Model,
            vec![item("a", "Alpha", "fast"), item("b", "Beta", "careful")],
        );
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_modal_picker(frame, &picker, frame.area(), Rect::new(0, 18, 60, 2));
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("models:"));
        assert!(text.contains("Alpha"));
        assert!(text.contains("Press Enter to confirm or Esc to go back"));
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
    fn multi_picker_renders_checked_state_and_toggle_hint() {
        let picker = PickerState::multi_with_selected(
            PickerKind::Permissions,
            vec![item("a", "Alpha", "fast"), item("b", "Beta", "careful")],
            vec!["b".to_string()],
        );
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_modal_picker(frame, &picker, frame.area(), Rect::new(0, 18, 60, 2));
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("permissions: 1 selected"));
        assert!(text.contains("☑ Beta"));
        assert!(text.contains("Press Space to toggle; Enter to confirm"));
    }

    #[test]
    fn composer_slash_popup_renders_rows_without_title_or_footer() {
        let mut picker = PickerState::new(
            PickerKind::SlashCommand,
            vec![
                item("model", "/model", "switch model"),
                item("mode", "/mode", "switch mode"),
            ],
        );
        picker.set_filter("mo");
        picker.selected = 1;
        let backend = TestBackend::new(60, 14);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_modal_picker(frame, &picker, frame.area(), Rect::new(0, 12, 60, 2));
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("/model"));
        assert!(text.contains("switch model"));
        assert!(!text.contains("commands:"));
        assert!(!text.contains("Press Enter"));
        assert!(!text.contains("┌"));
        let model_x = text.find("/model").expect("slash command rendered");
        assert!(buffer.content()[model_x + 1]
            .style()
            .add_modifier
            .contains(Modifier::BOLD));
        let selected_x = text
            .rfind("/mode")
            .expect("selected slash command rendered");
        assert_eq!(
            buffer.content()[selected_x + 1].style().fg,
            Some(Color::Cyan)
        );
        assert!(buffer.content()[selected_x + 1]
            .style()
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn composer_file_popup_uses_plain_paths_and_no_description() {
        let picker = PickerState::new(
            PickerKind::FileMention,
            vec![item("src/lib.rs", "src/lib.rs", "file mention")],
        );
        let backend = TestBackend::new(60, 14);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_modal_picker(frame, &picker, frame.area(), Rect::new(0, 12, 60, 2));
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains("file mention"));
        assert!(!text.contains("files:"));
    }

    #[test]
    fn composer_file_popup_empty_message_tracks_loading_state() {
        let loading = PickerState::new(
            PickerKind::FileMention,
            vec![item(
                "",
                "Loading file mentions…",
                "via /v1/at-command-completion",
            )],
        );
        let empty = PickerState::new(PickerKind::FileMention, Vec::new());

        assert_eq!(composer_empty_message(&loading), "loading...");
        assert!(composer_rows(&loading).is_empty());
        assert_eq!(composer_empty_message(&empty), "no matches");
        assert!(composer_rows(&empty).is_empty());
    }

    #[test]
    fn composer_file_popup_caps_visible_rows() {
        let picker = PickerState::new(
            PickerKind::FileMention,
            (0..12)
                .map(|idx| {
                    item(
                        &format!("src/file_{idx}.rs"),
                        &format!("src/file_{idx}.rs"),
                        "",
                    )
                })
                .collect(),
        );
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_modal_picker(frame, &picker, frame.area(), Rect::new(0, 18, 80, 2));
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("src/file_0.rs"));
        assert!(text.contains("src/file_7.rs"));
        assert!(!text.contains("src/file_8.rs"));
    }
}
