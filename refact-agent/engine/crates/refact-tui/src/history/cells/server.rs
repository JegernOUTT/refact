use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerContentBlockCell {
    text: String,
}

impl ServerContentBlockCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for ServerContentBlockCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::ServerContentBlock
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width.saturating_sub(4).max(1)));
        let mut lines = prefixed_wrapped_line(
            Line::from(bold_span(server_content_label(&self.text))),
            width,
            Line::from(dim_span("• ")),
            Line::from("  "),
        );
        lines.extend(prefix_lines(
            renderer.render(&self.text),
            dim_span("  └ "),
            dim_span("    "),
        ));
        finish(lines)
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let renderer = MarkdownRenderer::new(Some(width.saturating_sub(4).max(1)));
        let mut lines = plain_hyperlink_lines(prefixed_wrapped_line(
            Line::from(bold_span(server_content_label(&self.text))),
            width,
            Line::from(dim_span("• ")),
            Line::from("  "),
        ));
        lines.extend(prefix_link_lines(
            renderer.render_with_links(&self.text),
            dim_span("  └ "),
            dim_span("    "),
        ));
        finish_links(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerToolCell {
    card: ToolCard,
    selected: bool,
}

impl ServerToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for ServerToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::ServerContentBlock
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = server_call_header_lines(&self.card, width);
        lines.push(tool_summary_line(
            &self.card,
            self.card.name.clone(),
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CitationCell {
    text: String,
}

impl CitationCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for CitationCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Citation
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width.saturating_sub(4).max(1)));
        finish(prefix_lines(
            renderer.render(&self.text),
            dim_span("• "),
            dim_span("  "),
        ))
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let renderer = MarkdownRenderer::new(Some(width.saturating_sub(4).max(1)));
        finish_links(prefix_link_lines(
            renderer.render_with_links(&self.text),
            dim_span("• "),
            dim_span("  "),
        ))
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

fn server_call_header_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    let invocation = server_invocation_line(card);
    let header = if card.status == ToolStatus::Running {
        "Calling"
    } else {
        "Called"
    };
    let mut compact = Line::from(vec![
        tool_status_bullet(card.status),
        Span::raw(" "),
        bold_span(header),
        Span::raw(" "),
    ]);
    let reserved = line_width(&compact);
    let inline = line_width(&invocation) <= width.saturating_sub(reserved);
    if inline {
        compact.spans.extend(invocation.spans);
        vec![compact]
    } else {
        let mut lines = vec![Line::from(vec![
            tool_status_bullet(card.status),
            Span::raw(" "),
            bold_span(header),
        ])];
        lines.extend(prefixed_wrapped_line(
            invocation,
            width,
            Line::from(dim_span("  └ ")),
            Line::from("    "),
        ));
        lines
    }
}

fn server_invocation_line(card: &ToolCard) -> Line<'static> {
    let invocation = server_invocation(card);
    Line::from(vec![
        cyan_span(invocation.server),
        Span::raw("."),
        cyan_span(invocation.tool),
        Span::raw("("),
        dim_span(invocation.args),
        Span::raw(")"),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServerInvocation {
    server: String,
    tool: String,
    args: String,
}

fn server_invocation(card: &ToolCard) -> ServerInvocation {
    let parsed = serde_json::from_str::<Value>(&card.args).ok();
    if card.name == "mcp_call" {
        let tool_name = parsed
            .as_ref()
            .and_then(|value| value.get("tool_name"))
            .map(value_to_string)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "tool".to_string());
        return ServerInvocation {
            server: "mcp".to_string(),
            tool: tool_name.trim_start_matches("mcp_").to_string(),
            args: mcp_args_string(parsed.as_ref()),
        };
    }
    let args = parsed
        .as_ref()
        .map(value_to_string)
        .filter(|value| value != "{}")
        .unwrap_or_default();
    ServerInvocation {
        server: "server".to_string(),
        tool: card.name.clone(),
        args,
    }
}

fn mcp_args_string(value: Option<&Value>) -> String {
    let Some(Value::Object(obj)) = value else {
        return String::new();
    };
    if let Some(args) = obj.get("args") {
        return value_to_string(args);
    }
    let flattened = obj
        .iter()
        .filter(|(key, _)| key.as_str() != "tool_name")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    if flattened.is_empty() {
        String::new()
    } else {
        value_to_string(&Value::Object(flattened))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{text, tool_card};
    use serde_json::json;

    #[test]
    fn server_content_block_cell_snapshot() {
        assert_eq!(
            text(&ServerContentBlockCell::new("{\"type\":\"web_search_call\",\"status\":\"completed\"}").render(80)),
            "• server content · web_search_call · completed\n  └ {\"type\":\"web_search_call\",\"status\":\"completed\"}\n"
        );
    }

    #[test]
    fn server_tool_cell_snapshot() {
        let card = tool_card(
            "mcp_call",
            json!({"tool_name": "mcp_github_get_file_contents", "args": {"owner": "me", "repo": "r"}}),
            "README contents",
        );
        let rendered = text(&ServerToolCell::new(card, false).render(80));
        assert_eq!(
            rendered,
            "• Called mcp.github_get_file_contents({\"owner\":\"me\",\"repo\":\"r\"})\n▾ ✅ mcp_call · 1.2s\n  └ README contents\n"
        );
    }

    #[test]
    fn citation_cell_snapshot() {
        assert_eq!(
            text(&CitationCell::new("{\"title\":\"README\"}").render(40)),
            "• {\"title\":\"README\"}\n"
        );
    }
}
