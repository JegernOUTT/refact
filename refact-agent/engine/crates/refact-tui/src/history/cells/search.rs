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
            rendered_duration(&self.card),
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
