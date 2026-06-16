use super::*;

use crate::style::proposed_plan_style;
use crate::vendored::terminal_hyperlinks::prefix_hyperlink_lines;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlanCellData {
    pub content: String,
    pub mode: String,
    pub version: u32,
    pub delta_count: usize,
}

impl PlanCellData {
    pub fn new(
        content: impl Into<String>,
        mode: impl Into<String>,
        version: u32,
        delta_count: usize,
    ) -> Self {
        Self {
            content: content.into(),
            mode: mode.into(),
            version,
            delta_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlanCell {
    data: PlanCellData,
}

impl PlanCell {
    pub fn new(data: PlanCellData) -> Self {
        Self { data }
    }

    fn title(&self) -> &'static str {
        "Proposed Plan"
    }

    fn metadata(&self) -> String {
        format!(
            "plan · {} · v{} · {}",
            self.data.mode,
            self.data.version,
            update_label(self.data.delta_count)
        )
    }
}

impl HistoryCell for PlanCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Plan
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        self.render_with_links(width)
            .into_iter()
            .map(|line| line.line)
            .collect()
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let mut lines = vec![HyperlinkLine::new(plan_header_line(self.title()))];
        let mut card = vec![HyperlinkLine::new(Line::from(" "))];
        card.push(HyperlinkLine::new(Line::from(Span::styled(
            self.metadata(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))));
        card.push(HyperlinkLine::new(Line::from(" ")));

        let renderer = MarkdownRenderer::new(Some(width.saturating_sub(2).max(1)));
        card.extend(prefix_hyperlink_lines(
            renderer.render_with_links(&self.data.content),
            Span::raw("  "),
            Span::raw("  "),
        ));
        card.push(HyperlinkLine::new(Line::from(" ")));

        let plan_style = proposed_plan_style();
        lines.extend(card.into_iter().map(|line| line.style(plan_style)));
        finish_links(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.data))
    }
}

fn update_label(delta_count: usize) -> String {
    if delta_count == 1 {
        "1 update".to_string()
    } else {
        format!("{delta_count} updates")
    }
}

fn plan_header_line(title: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("• ", Style::default().add_modifier(Modifier::DIM)),
        Span::styled(
            title.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::text;

    #[test]
    fn plan_cell_renders_markdown_plan() {
        let cell = PlanCell::new(PlanCellData::new("## Plan\n- do it", "agent", 1, 0));
        let lines = cell.render(80);

        assert_eq!(
            text(&lines),
            "• Proposed Plan\n \nplan · agent · v1 · 0 updates\n \n  ## Plan\n  \n  - do it\n \n"
        );
        assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::DIM));
        assert!(lines[0].spans[1]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(lines[2].spans[0].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(lines[1].style, proposed_plan_style());
        assert_eq!(lines[2].style, proposed_plan_style());
    }

    #[test]
    fn plan_cell_snapshot_merges_deltas() {
        let content = synthesize_plan_content(
            "## Plan\n- base",
            &["first update".to_string(), "second update".to_string()],
        );
        let cell = PlanCell::new(PlanCellData::new(content, "agent", 2, 2));

        assert_eq!(
            text(&cell.render(80)),
            "• Proposed Plan\n \nplan · agent · v2 · 2 updates\n \n  ## Plan\n  \n  - base\n  \n  ———\n  \n  ## Plan updates\n  \n  first update\n  \n  second update\n \n"
        );
    }
}
