use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use serde_json::Value;

use crate::app::TranscriptItem;
use crate::approvals::{render_modal_lines, ApprovalModalState};
use crate::render::wrapping::{adaptive_wrap_lines, line_width, RtOptions};
use crate::render::{color_enabled_from_env, is_unified_diff, render_unified_diff, MarkdownRenderer};
use crate::text_safety::{compact_tool_preview, sanitize_json_strings, sanitize_tool_text};
use crate::tools::{ToolCard, ToolStatus};
use crate::vendored::terminal_hyperlinks::{
    plain_hyperlink_lines, prefix_hyperlink_lines, HyperlinkLine,
};

const COLLAPSED_OUTPUT_LINES: usize = 12;
const EXPANDED_OUTPUT_LINES: usize = 200;
const PLAN_SYNTHESIS_SEPARATOR: &str = "\n\n---\n\n## Plan updates\n\n";
const GOAL_SYNTHESIS_SEPARATOR: &str = "\n\n---\n\n## Goal updates\n\n";

mod approval;
mod exec;
mod messages;
mod notices;
mod patches;
mod plans;
mod request_input;
mod search;
mod server;
mod session;

pub use approval::{ApprovalCell, ApprovalOutcome};
pub use exec::{ExecToolCell, SubchatCell, ToolCallCell};
pub use messages::{AssistantCell, AssistantStreamCell, ReasoningCell, UserCell};
pub use notices::{EventCell, EventCellData, InfoCell, NoticeCell, StatusCell};
pub use patches::{DiffCell, DiffToolCell};
pub use plans::{GoalCell, GoalCellData, PlanCell, PlanCellData, PlanStreamCell};
pub use request_input::RequestInputToolCell;
pub use search::SearchToolCell;
pub use server::{CitationCell, ServerContentBlockCell, ServerToolCell};
pub use session::SessionCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryCellKind {
    User,
    Assistant,
    Reasoning,
    Notice,
    Info,
    Tool,
    Subchat,
    Exec,
    Diff,
    Plan,
    Goal,
    Citation,
    ServerContentBlock,
    Search,
    RequestInput,
    Event,
    Approval,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCellType {
    Exec,
    Diff,
    Server,
    Search,
    RequestInput,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryRenderMode {
    Rich,
    Raw,
}

pub trait HistoryCell: HistoryCellClone + std::fmt::Debug + Send + Sync {
    fn kind(&self) -> HistoryCellKind;
    fn render(&self, width: usize) -> Vec<Line<'static>>;
    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.render(width))
    }
    fn display_hyperlink_lines(&self, width: usize) -> Vec<HyperlinkLine> {
        self.render_with_links(width)
    }
    fn desired_height(&self, width: usize) -> usize {
        if width == 0 {
            return 0;
        }
        let width_u16 = width.min(u16::MAX as usize) as u16;
        Paragraph::new(Text::from(self.render(width)))
            .wrap(Wrap { trim: false })
            .line_count(width_u16)
    }
    fn transcript_lines(&self, width: usize) -> Vec<Line<'static>> {
        self.render(width)
    }
    fn is_stream_continuation(&self) -> bool {
        false
    }
    fn transcript_animation_tick(&self) -> Option<u64> {
        None
    }
    fn is_final(&self) -> bool {
        true
    }
    fn revision(&self) -> u64;
}

pub trait HistoryCellClone {
    fn clone_box(&self) -> Box<dyn HistoryCell>;
}

impl<T> HistoryCellClone for T
where
    T: HistoryCell + Clone + 'static,
{
    fn clone_box(&self) -> Box<dyn HistoryCell> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn HistoryCell> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

pub fn raw_lines_from_source(source: &str) -> Vec<Line<'static>> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut parts = source.split('\n').collect::<Vec<_>>();
    if source.ends_with('\n') {
        parts.pop();
    }
    parts
        .into_iter()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

pub fn plain_lines(lines: impl IntoIterator<Item = Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let text = line
                .spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>();
            Line::from(text)
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct PrefixedWrappedHistoryCell {
    text: Text<'static>,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
}

impl PrefixedWrappedHistoryCell {
    pub fn new(
        text: impl Into<Text<'static>>,
        initial_prefix: impl Into<Line<'static>>,
        subsequent_prefix: impl Into<Line<'static>>,
    ) -> Self {
        Self {
            text: text.into(),
            initial_prefix: initial_prefix.into(),
            subsequent_prefix: subsequent_prefix.into(),
        }
    }
}

impl HistoryCell for PrefixedWrappedHistoryCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Info
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        if width == 0 {
            return Vec::new();
        }
        let opts = RtOptions::new(width)
            .initial_indent(self.initial_prefix.clone())
            .subsequent_indent(self.subsequent_prefix.clone());
        adaptive_wrap_lines(self.text.clone().lines, opts)
    }

    fn transcript_lines(&self, _width: usize) -> Vec<Line<'static>> {
        plain_lines(self.text.clone().lines)
    }

    fn revision(&self) -> u64 {
        revision(&(
            self.kind(),
            format!("{:?}", self.text),
            format!("{:?}", self.initial_prefix),
            format!("{:?}", self.subsequent_prefix),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct CompositeHistoryCell {
    parts: Vec<Box<dyn HistoryCell>>,
}

impl CompositeHistoryCell {
    pub fn new(parts: Vec<Box<dyn HistoryCell>>) -> Self {
        Self { parts }
    }
}

impl HistoryCell for CompositeHistoryCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Info
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut first = true;
        for part in &self.parts {
            let mut lines = part.render(width);
            if !lines.is_empty() {
                if !first {
                    out.push(Line::from(""));
                }
                out.append(&mut lines);
                first = false;
            }
        }
        out
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let mut out = Vec::new();
        let mut first = true;
        for part in &self.parts {
            let mut lines = part.render_with_links(width);
            if !lines.is_empty() {
                if !first {
                    out.push(HyperlinkLine::new(Line::from("")));
                }
                out.append(&mut lines);
                first = false;
            }
        }
        out
    }

    fn display_hyperlink_lines(&self, width: usize) -> Vec<HyperlinkLine> {
        let mut out = Vec::new();
        let mut first = true;
        for part in &self.parts {
            let mut lines = part.display_hyperlink_lines(width);
            if !lines.is_empty() {
                if !first {
                    out.push(HyperlinkLine::new(Line::from("")));
                }
                out.append(&mut lines);
                first = false;
            }
        }
        out
    }

    fn transcript_lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut first = true;
        for part in &self.parts {
            let mut lines = part.transcript_lines(width);
            if !lines.is_empty() {
                if !first {
                    out.push(Line::from(""));
                }
                out.append(&mut lines);
                first = false;
            }
        }
        out
    }

    fn is_final(&self) -> bool {
        self.parts.iter().all(|part| part.is_final())
    }

    fn revision(&self) -> u64 {
        let revisions = self
            .parts
            .iter()
            .map(|part| (part.kind(), part.revision()))
            .collect::<Vec<_>>();
        revision(&(self.kind(), revisions))
    }
}

pub fn cell_from_transcript_item(item: &TranscriptItem, selected: bool) -> Box<dyn HistoryCell> {
    match item {
        TranscriptItem::User(text) => Box::new(UserCell::new(text.clone(), selected)),
        TranscriptItem::Assistant(text) => Box::new(AssistantCell::new(text.clone())),
        TranscriptItem::Reasoning(text, collapsed) => {
            Box::new(ReasoningCell::new(text.clone(), *collapsed))
        }
        TranscriptItem::Tool(card) => cell_from_tool_card(card.clone(), selected),
        TranscriptItem::Plan(data) => Box::new(PlanCell::new(data.clone())),
        TranscriptItem::Goal(data) => Box::new(GoalCell::new(data.clone())),
        TranscriptItem::PlanStream(lines) => Box::new(PlanStreamCell::new(lines.clone(), false)),
        TranscriptItem::Citation(text) => Box::new(CitationCell::new(text.clone())),
        TranscriptItem::ServerContentBlock(text) => {
            Box::new(ServerContentBlockCell::new(text.clone()))
        }
        TranscriptItem::Diff(text) => Box::new(DiffCell::new(text.clone())),
        TranscriptItem::Notice(text) => Box::new(NoticeCell::new(text.clone())),
        TranscriptItem::Info(lines) => Box::new(InfoCell::new(lines.clone())),
        TranscriptItem::Status(snapshot, theme) => {
            Box::new(StatusCell::new(snapshot.clone(), theme.clone()))
        }
        TranscriptItem::Approval(state, outcome) => {
            Box::new(ApprovalCell::new(state.clone(), *outcome))
        }
        TranscriptItem::Session { title, subtitle } => {
            Box::new(SessionCell::new(title.clone(), subtitle.clone()))
        }
    }
}

pub fn render_transcript_item_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<Line<'static>> {
    cell_from_transcript_item(item, selected).render(width)
}

pub fn render_transcript_item_hyperlink_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<HyperlinkLine> {
    cell_from_transcript_item(item, selected).render_with_links(width)
}

pub fn cell_from_tool_card(card: ToolCard, selected: bool) -> Box<dyn HistoryCell> {
    match tool_cell_type_for(&card.name) {
        ToolCellType::Exec => Box::new(ExecToolCell::new(card, selected)),
        ToolCellType::Diff => Box::new(DiffToolCell::new(card, selected)),
        ToolCellType::Server => Box::new(ServerToolCell::new(card, selected)),
        ToolCellType::Search => Box::new(SearchToolCell::new(card, selected)),
        ToolCellType::RequestInput => Box::new(RequestInputToolCell::new(card, selected)),
        ToolCellType::Generic => Box::new(ToolCallCell::new(card, selected)),
    }
}

pub fn tool_cell_type_for(name: &str) -> ToolCellType {
    if is_process_tool(name) || name == "shell" {
        ToolCellType::Exec
    } else if is_diff_tool(name) {
        ToolCellType::Diff
    } else if is_server_tool(name) {
        ToolCellType::Server
    } else if is_search_tool(name) {
        ToolCellType::Search
    } else if is_request_input_tool(name) {
        ToolCellType::RequestInput
    } else {
        ToolCellType::Generic
    }
}

pub fn synthesize_plan_content(base: &str, deltas: &[String]) -> String {
    if deltas.is_empty() {
        base.to_string()
    } else {
        format!("{base}{PLAN_SYNTHESIS_SEPARATOR}{}", deltas.join("\n\n"))
    }
}

pub fn synthesize_goal_content(base: &str, deltas: &[String]) -> String {
    if deltas.is_empty() {
        base.to_string()
    } else {
        format!("{base}{GOAL_SYNTHESIS_SEPARATOR}{}", deltas.join("\n\n"))
    }
}

fn is_process_tool(name: &str) -> bool {
    name.starts_with("process_")
}

fn is_diff_tool(name: &str) -> bool {
    matches!(
        name,
        "patch"
            | "apply_patch"
            | "text_edit"
            | "create_textdoc"
            | "update_textdoc"
            | "replace_textdoc"
            | "update_textdoc_regex"
            | "update_textdoc_by_lines"
            | "update_textdoc_anchored"
            | "undo_textdoc"
            | "rm"
            | "mv"
    )
}

fn is_server_tool(name: &str) -> bool {
    name.starts_with("srvtoolu_")
        || matches!(
            name,
            "web_search_call"
                | "file_search_call"
                | "code_interpreter_call"
                | "mcp_call"
                | "local_shell_call"
                | "image_generation_call"
                | "computer_use_call"
                | "web_fetch"
                | "web_search"
                | "code_execution"
        )
}

fn is_search_tool(name: &str) -> bool {
    matches!(
        name,
        "knowledge"
            | "search"
            | "search_pattern"
            | "search_symbol_definition"
            | "tree"
            | "cat"
            | "doc_list"
            | "doc_get"
            | "vecdb_search"
    )
}

fn is_request_input_tool(name: &str) -> bool {
    matches!(
        name,
        "ask_questions" | "request_user_input" | "request-user-input" | "agent_ask_planner"
    )
}

fn role_line(label: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(Span::styled(label.into(), style))
}

fn finish(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines.push(Line::default());
    lines
}

fn finish_links(mut lines: Vec<HyperlinkLine>) -> Vec<HyperlinkLine> {
    lines.push(HyperlinkLine::new(Line::default()));
    lines
}

fn revision(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn tool_summary_line(card: &ToolCard, title: String, meta: String) -> Line<'static> {
    let marker = if card.expanded { "▾" } else { "▸" };
    let mut spans = vec![
        Span::styled(marker, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(card.status.icon(), status_style(card.status)),
        Span::raw(" "),
        Span::styled(title, Style::default().fg(Color::White)),
    ];
    if !meta.is_empty() {
        spans.push(Span::styled(
            format!(" · {meta}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn status_style(status: ToolStatus) -> Style {
    match status {
        ToolStatus::Running => Style::default().fg(Color::Yellow),
        ToolStatus::Success => Style::default().fg(Color::Green),
        ToolStatus::Error => Style::default().fg(Color::Red),
    }
}

fn dim_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn dim_span(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), dim_style())
}

fn bold_span(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().add_modifier(Modifier::BOLD))
}

fn cyan_span(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(Color::Cyan))
}

fn tool_status_bullet(status: ToolStatus) -> Span<'static> {
    match status {
        ToolStatus::Running => dim_span("•"),
        ToolStatus::Success => Span::styled(
            "•",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        ToolStatus::Error => Span::styled(
            "•",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    }
}

fn prefixed_wrapped_line(
    line: Line<'static>,
    width: usize,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
) -> Vec<Line<'static>> {
    PrefixedWrappedHistoryCell::new(Text::from(line), initial_prefix, subsequent_prefix)
        .render(width)
}

fn prefix_lines(
    lines: Vec<Line<'static>>,
    initial_prefix: Span<'static>,
    subsequent_prefix: Span<'static>,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mut spans = vec![if index == 0 {
                initial_prefix.clone()
            } else {
                subsequent_prefix.clone()
            }];
            spans.extend(line.spans);
            Line {
                spans,
                style: line.style,
                alignment: line.alignment,
            }
        })
        .collect()
}

fn prefix_link_lines(
    lines: Vec<HyperlinkLine>,
    initial_prefix: Span<'static>,
    subsequent_prefix: Span<'static>,
) -> Vec<HyperlinkLine> {
    prefix_hyperlink_lines(lines, initial_prefix, subsequent_prefix)
}

fn wrap_with_prefix(
    text: &str,
    width: usize,
    initial_prefix: Span<'static>,
    subsequent_prefix: Span<'static>,
    style: Style,
) -> Vec<Line<'static>> {
    prefixed_wrapped_line(
        Line::from(Span::styled(text.to_string(), style)),
        width,
        Line::from(initial_prefix),
        Line::from(subsequent_prefix),
    )
}

fn subchat_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    SubchatCell::new(card.clone()).render_inline(width)
}

fn command_label(card: &ToolCard) -> String {
    argument_value(card, &["command", "cmd"])
        .map(|command| format!("$ {command}"))
        .or_else(|| argument_value(card, &["process_id"]).map(|id| format!("process {id}")))
        .unwrap_or_else(|| format!("{}({})", card.name, card.args_preview))
}

fn search_label(card: &ToolCard) -> String {
    argument_value(
        card,
        &["pattern", "query", "search_key", "symbols", "path", "scope"],
    )
    .map(|query| format!("{} · {query}", card.name))
    .unwrap_or_else(|| format!("{}({})", card.name, card.args_preview))
}

fn request_input_label(card: &ToolCard) -> String {
    argument_value(card, &["question", "prompt", "message", "title"])
        .unwrap_or_else(|| format!("{}({})", card.name, card.args_preview))
}

fn argument_value(card: &ToolCard, keys: &[&str]) -> Option<String> {
    let value = serde_json::from_str::<Value>(&card.args).ok()?;
    keys.iter()
        .find_map(|key| value.get(*key).map(value_to_string))
}

fn value_to_string(value: &Value) -> String {
    let sanitized = sanitize_json_strings(value);
    match sanitized {
        Value::String(value) => value,
        value => serde_json::to_string(&value).unwrap_or_else(|_| value.to_string()),
    }
}

fn exit_code_from_result(result: &str) -> Option<String> {
    result.lines().find_map(|line| {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("exit_code:") {
            let value = value.trim();
            return (!value.is_empty() && value != "<none>").then(|| value.to_string());
        }
        trimmed
            .rsplit_once("exit code ")
            .map(|(_, code)| code.trim().to_string())
            .filter(|code| !code.is_empty())
    })
}

fn output_lines(
    result: &str,
    width: usize,
    max_lines: usize,
    collapsed: bool,
) -> Vec<Line<'static>> {
    let result = sanitize_tool_text(result);
    let all_lines = result.lines().collect::<Vec<_>>();
    if all_lines.is_empty() {
        return vec![Line::from(Span::styled(
            "(no output)",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    let shown = all_lines.len().min(max_lines);
    let mut lines = all_lines
        .iter()
        .take(shown)
        .map(|line| {
            Line::from(Span::styled(
                compact_preview(line, width.saturating_sub(4).max(8)),
                output_style(line),
            ))
        })
        .collect::<Vec<_>>();
    if all_lines.len() > shown {
        let suffix = if collapsed { " (expand)" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("… {} more lines{suffix}", all_lines.len() - shown),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn output_style(line: &str) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("stderr") || line.contains("error") || line.contains("failed") {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::White)
    }
}

fn compact_preview(value: &str, max_chars: usize) -> String {
    compact_tool_preview(value, max_chars)
}

fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1000 {
        format!("{duration_ms}ms")
    } else {
        format!("{:.1}s", duration_ms as f64 / 1000.0)
    }
}

fn diff_source(card: &ToolCard) -> String {
    if is_unified_diff(&card.result) {
        return card.result.clone();
    }
    argument_value(card, &["patch", "diff"])
        .filter(|value| is_unified_diff(value))
        .unwrap_or_else(|| card.result.clone())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileDiffStat {
    path: String,
    added: usize,
    deleted: usize,
}

fn diff_file_stats(source: &str) -> Vec<FileDiffStat> {
    let mut stats = Vec::<FileDiffStat>::new();
    let mut current = None::<FileDiffStat>;
    for line in source.lines() {
        if let Some(path) = diff_git_path(line).or_else(|| plus_file_path(line)) {
            if let Some(stat) = current.take() {
                stats.push(stat);
            }
            current = Some(FileDiffStat {
                path,
                added: 0,
                deleted: 0,
            });
            continue;
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            current.get_or_insert_with(default_diff_stat).added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            current.get_or_insert_with(default_diff_stat).deleted += 1;
        }
    }
    if let Some(stat) = current {
        stats.push(stat);
    }
    if stats.is_empty() && !source.is_empty() {
        stats.push(default_diff_stat());
    }
    stats
}

fn default_diff_stat() -> FileDiffStat {
    FileDiffStat {
        path: "changes".to_string(),
        added: 0,
        deleted: 0,
    }
}

fn diff_git_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    rest.split_whitespace()
        .nth(1)
        .map(|path| path.trim_start_matches("b/").to_string())
        .filter(|path| !path.is_empty())
}

fn plus_file_path(line: &str) -> Option<String> {
    let path = line.strip_prefix("+++ ")?.trim();
    if path == "/dev/null" {
        return None;
    }
    Some(path.trim_start_matches("b/").to_string()).filter(|path| !path.is_empty())
}

fn diff_summary(stats: &[FileDiffStat]) -> String {
    let files = stats.len();
    let added = stats.iter().map(|stat| stat.added).sum::<usize>();
    let deleted = stats.iter().map(|stat| stat.deleted).sum::<usize>();
    let file_label = if files == 1 { "file" } else { "files" };
    format!("{} {file_label} · +{} -{}", files.max(1), added, deleted)
}

fn server_content_label(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            let kind = value.get("type").and_then(Value::as_str)?;
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if status.is_empty() {
                Some(format!("server content · {kind}"))
            } else {
                Some(format!("server content · {kind} · {status}"))
            }
        })
        .unwrap_or_else(|| "server content".to_string())
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;
    use crate::approvals::PauseReason;
    use crate::render::wrapping::line_to_plain;
    use serde_json::{json, Value};

    pub(super) fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(line_to_plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn approval_state() -> ApprovalModalState {
        ApprovalModalState::new(vec![PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: "echo hi".to_string(),
            rule: "default".to_string(),
            tool_call_id: "call-1".to_string(),
            integr_config_path: None,
            args: None,
            diff: None,
        }])
    }

    pub(super) fn tool_card(name: &str, args: Value, result: &str) -> ToolCard {
        let mut card = ToolCard::from_tool_call(&json!({
            "id": format!("call-{name}"),
            "function": {"name": name, "arguments": args.to_string()}
        }))
        .with_result(result, ToolStatus::Success);
        card.duration_ms = Some(1200);
        card.expanded = true;
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_table_maps_known_tool_names() {
        assert_eq!(tool_cell_type_for("shell"), ToolCellType::Exec);
        assert_eq!(tool_cell_type_for("process_read"), ToolCellType::Exec);
        assert_eq!(tool_cell_type_for("apply_patch"), ToolCellType::Diff);
        assert_eq!(tool_cell_type_for("search_pattern"), ToolCellType::Search);
        assert_eq!(
            tool_cell_type_for("ask_questions"),
            ToolCellType::RequestInput
        );
        assert_eq!(tool_cell_type_for("web_search_call"), ToolCellType::Server);
        assert_eq!(tool_cell_type_for("totally_unknown"), ToolCellType::Generic);
    }

    #[test]
    fn raw_lines_from_source_omits_trailing_empty_line() {
        assert_eq!(raw_lines_from_source("one\ntwo\n").len(), 2);
    }
}
