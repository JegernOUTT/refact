use super::*;

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
        HistoryCellKind::ContentBlock
    }

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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
        self.card.status.is_final()
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

fn server_call_header_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    let invocation = server_invocation_line(card);
    let header = if card.status.is_active() {
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
    if tool_family(&card.name).is_mcp() {
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
    fn server_tool_cell_snapshot() {
        let card = tool_card(
            "mcp_call",
            json!({"tool_name": "mcp_github_get_file_contents", "args": {"owner": "me", "repo": "r"}}),
            "README contents",
        );
        let rendered = text(&ServerToolCell::new(card, false).render(80));
        assert_eq!(
            rendered,
            "✅ Called mcp.github_get_file_contents({\"owner\":\"me\",\"repo\":\"r\"})\n▾ ✅ succeeded mcp_call · 1.2s\n  └ README contents\n"
        );
    }
}
