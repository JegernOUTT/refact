use super::*;
use crate::render::wrapping::take_prefix_by_width;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionCell {
    title: String,
    subtitle: Option<String>,
}

impl SessionCell {
    pub fn new(title: impl Into<String>, subtitle: Option<String>) -> Self {
        Self {
            title: title.into(),
            subtitle,
        }
    }
}

impl HistoryCell for SessionCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Session
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        finish(vec![session_separator_line(
            width.max(1),
            session_label(&self.title, self.subtitle.as_deref()),
        )])
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.title, &self.subtitle))
    }
}

fn session_label(title: &str, subtitle: Option<&str>) -> String {
    match subtitle.filter(|value| !value.trim().is_empty()) {
        Some(subtitle) => format!("{title} · {subtitle}"),
        None => title.to_string(),
    }
}

fn session_separator_line(width: usize, label: String) -> Line<'static> {
    let label = label.trim();
    let raw = if label.is_empty() {
        "─".repeat(width)
    } else {
        let candidate = format!("─ {label} ─");
        let (prefix, _, used_width) = take_prefix_by_width(&candidate, width);
        format!("{prefix}{}", "─".repeat(width.saturating_sub(used_width)))
    };
    Line::from(Span::styled(
        raw,
        Style::default().add_modifier(Modifier::DIM),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::text;

    #[test]
    fn session_cell_snapshot() {
        let cell = SessionCell::new("New chat started", Some("agent mode".to_string()));
        assert_eq!(
            text(&cell.render(36)),
            "─ New chat started · agent mode ────\n"
        );
    }

    #[test]
    fn session_cell_truncates_separator_to_width() {
        let cell = SessionCell::new("New chat started", Some("agent mode".to_string()));
        assert_eq!(text(&cell.render(12)), "─ New chat s\n");
    }

    #[test]
    fn session_cell_renders_empty_separator() {
        let cell = SessionCell::new("", None);
        assert_eq!(text(&cell.render(6)), "──────\n");
    }
}
