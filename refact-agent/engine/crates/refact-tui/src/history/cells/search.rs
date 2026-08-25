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

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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
        self.card.status.is_final()
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

fn search_header_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    let header = tool_family(&card.name).search_header(card.status.is_active());
    let detail = search_detail(card);
    let line = if detail.is_empty() {
        Line::from(bold_span(header))
    } else {
        let separator = if card.status.is_active() {
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
            "• Searched code for needle\n▾ ✅ succeeded search_pattern · needle · 1.2s\n  └ src/main.rs:1: needle\n"
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
            "• Searching code needle\n▾ ⏳ running search_pattern · needle\n"
        );
    }

    #[test]
    fn search_headers_are_family_aware() {
        let cases = [
            ("web_search", "Searching the web", "Searched the web"),
            ("web", "Fetching the web", "Fetched the web"),
            ("search_pattern", "Searching code", "Searched code"),
            ("cat", "Searching files", "Searched files"),
            (
                "tree",
                "Inspecting the file tree",
                "Inspected the file tree",
            ),
            (
                "doc_get",
                "Searching documentation",
                "Searched documentation",
            ),
            (
                "task_mem_search",
                "Searching knowledge",
                "Searched knowledge",
            ),
            ("knowledge", "Searching knowledge", "Searched knowledge"),
            ("unknown_search", "Searching", "Searched"),
        ];

        for (name, active, complete) in cases {
            let mut card = tool_card(name, json!({"query": "needle"}), "");
            card.status = ToolStatus::Running;
            assert!(
                text(&search_header_lines(&card, 80)).contains(active),
                "{name}"
            );

            card.status = ToolStatus::Succeeded;
            assert!(
                text(&search_header_lines(&card, 80)).contains(complete),
                "{name}"
            );
        }
    }
}
