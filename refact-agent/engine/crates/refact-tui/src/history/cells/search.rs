use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchToolCell {
    card: ToolCard,
    selected: bool,
}

impl SearchToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for SearchToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Search
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = search_header_lines(&self.card, width);
        lines.push(tool_summary_line(
            &self.card,
            search_label(&self.card),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        lines.extend(subchat_lines(&self.card, width));
        if self.card.expanded && !self.card.result.is_empty() {
            lines.extend(prefix_lines(
                output_lines(&self.card.result, width, EXPANDED_OUTPUT_LINES, false),
                dim_span("  └ "),
                dim_span("    "),
            ));
        } else if !self.card.result.is_empty() {
            lines.extend(prefix_lines(
                output_lines(&self.card.result, width, COLLAPSED_OUTPUT_LINES, true),
                dim_span("  └ "),
                dim_span("    "),
            ));
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.card.status != ToolStatus::Running
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

fn search_header_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    let header = if card.status == ToolStatus::Running {
        "Searching the web"
    } else {
        "Searched the web"
    };
    let detail = search_detail(card);
    let line = if detail.is_empty() {
        Line::from(bold_span(header))
    } else {
        let separator = if card.status == ToolStatus::Running {
            " "
        } else {
            " for "
        };
        Line::from(vec![
            bold_span(header),
            Span::raw(separator),
            Span::raw(detail),
        ])
    };
    prefixed_wrapped_line(
        line,
        width,
        Line::from(vec![dim_span("•"), Span::raw(" ")]),
        Line::from("  "),
    )
}

fn search_detail(card: &ToolCard) -> String {
    argument_value(
        card,
        &["query", "pattern", "search_key", "symbols", "path", "scope"],
    )
    .unwrap_or_else(|| card.args_preview.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{text, tool_card};
    use serde_json::json;

    #[test]
    fn search_cell_snapshot() {
        let card = tool_card(
            "search_pattern",
            json!({"pattern": "needle", "scope": "src"}),
            "src/main.rs:1: needle",
        );
        let rendered = text(&SearchToolCell::new(card, false).render(80));
        assert_eq!(
            rendered,
            "• Searched the web for needle\n▾ ✅ search_pattern · needle · 1.2s\n  └ src/main.rs:1: needle\n"
        );
    }

    #[test]
    fn search_cell_running_uses_live_header() {
        let mut card = tool_card("search_pattern", json!({"pattern": "needle"}), "");
        card.status = ToolStatus::Running;
        card.duration_ms = None;
        let rendered = text(&SearchToolCell::new(card, false).render(80));
        assert_eq!(
            rendered,
            "• Searching the web needle\n▾ ⏳ search_pattern · needle\n"
        );
    }
}
