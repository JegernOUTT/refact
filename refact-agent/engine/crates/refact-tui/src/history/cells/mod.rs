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
use crate::theme::{ThemeRole, TuiTheme};
use crate::tools::{ToolCard, ToolStatus};
use crate::vendored::terminal_hyperlinks::{plain_hyperlink_lines, HyperlinkLine};

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
mod tool_family;

pub use approval::ApprovalCell;
pub use exec::{ExecToolCell, SubchatCell, ToolCallCell};
pub use messages::{AssistantCell, AssistantStreamCell, ContentBlockCell, ReasoningCell, UserCell};
pub use notices::{EventCell, EventCellData, InfoCell, NoticeCell, StatusCell};
pub use patches::{DiffCell, DiffToolCell};
pub use plans::{GoalCell, GoalCellData, PlanCell, PlanCellData, PlanStreamCell};
pub use request_input::RequestInputToolCell;
pub use search::SearchToolCell;
pub use server::ServerToolCell;
pub use session::SessionCell;
pub use tool_family::{tool_display_name, tool_family, ToolFamily};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryCellKind {
    User,
    Assistant,
    Reasoning,
    ContentBlock,
    Notice,
    Info,
    Tool,
    Subchat,
    Exec,
    Diff,
    Plan,
    Goal,
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
    fn render_raw(&self, width: usize) -> Vec<Line<'static>>;
    fn render(&self, width: usize) -> Vec<Line<'static>> {
        self.render_raw(width)
    }
    fn render_with_theme(&self, width: usize, theme: &TuiTheme) -> Vec<Line<'static>> {
        resolve_cell_lines_with_color_enabled(
            self.render_raw(width),
            theme,
            true,
            color_enabled_from_env(),
        )
    }
    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.render_raw(width))
    }
    fn render_with_links_with_theme(&self, width: usize, theme: &TuiTheme) -> Vec<HyperlinkLine> {
        resolve_hyperlink_lines_with_color_enabled(
            self.render_with_links(width),
            theme,
            true,
            color_enabled_from_env(),
        )
    }
    fn display_hyperlink_lines(&self, width: usize) -> Vec<HyperlinkLine> {
        self.render_with_links(width)
    }
    fn display_hyperlink_lines_with_theme(
        &self,
        width: usize,
        theme: &TuiTheme,
    ) -> Vec<HyperlinkLine> {
        let color_enabled = color_enabled_from_env();
        if color_enabled && *theme == TuiTheme::default() {
            return self.display_hyperlink_lines(width);
        }
        resolve_hyperlink_lines_with_color_enabled(
            self.display_hyperlink_lines(width),
            theme,
            true,
            color_enabled,
        )
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

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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
        TranscriptItem::ContentBlock {
            summary,
            body,
            collapsed,
            expandable,
        } => Box::new(ContentBlockCell::new(
            summary.clone(),
            body.clone(),
            *collapsed,
            *expandable,
        )),
        TranscriptItem::Tool(card) => cell_from_tool_card(card.clone(), selected),
        TranscriptItem::Plan(data) => Box::new(PlanCell::new(data.clone())),
        TranscriptItem::Goal(data) => Box::new(GoalCell::new(data.clone())),
        TranscriptItem::PlanStream(lines) => Box::new(PlanStreamCell::new(lines.clone(), false)),
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

pub fn render_transcript_item_lines_with_theme(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    cell_from_transcript_item(item, selected).render_with_theme(width, theme)
}

pub fn render_transcript_item_hyperlink_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<HyperlinkLine> {
    cell_from_transcript_item(item, selected).render_with_links(width)
}

pub fn render_transcript_item_hyperlink_lines_with_theme(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
    theme: &TuiTheme,
) -> Vec<HyperlinkLine> {
    cell_from_transcript_item(item, selected).render_with_links_with_theme(width, theme)
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
    tool_family(name).cell_type()
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
        Span::styled(marker, default_theme_style(ThemeRole::Accent)),
        Span::raw(" "),
        Span::styled(card.status.visual(), status_style(card.status)),
        Span::raw(" "),
        Span::styled(title, default_theme_style(ThemeRole::Text)),
    ];
    if !meta.is_empty() {
        spans.push(Span::styled(
            format!(" · {meta}"),
            default_theme_style(ThemeRole::Muted),
        ));
    }
    Line::from(spans)
}

fn status_style(status: ToolStatus) -> Style {
    match status {
        ToolStatus::Queued | ToolStatus::Cancelled => default_theme_style(ThemeRole::Muted),
        ToolStatus::AwaitingApproval | ToolStatus::Running => {
            default_theme_style(ThemeRole::Warning)
        }
        ToolStatus::ApprovedOnce | ToolStatus::ApprovedForChat => {
            default_theme_style(ThemeRole::Accent)
        }
        ToolStatus::Succeeded => default_theme_style(ThemeRole::Success),
        ToolStatus::Failed | ToolStatus::Denied => default_theme_style(ThemeRole::Error),
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
    Span::styled(text.into(), default_theme_style(ThemeRole::Highlight))
}

fn tool_status_bullet(status: ToolStatus) -> Span<'static> {
    match status {
        ToolStatus::Queued | ToolStatus::Cancelled => dim_span(status.icon()),
        ToolStatus::AwaitingApproval | ToolStatus::Running => {
            Span::styled(status.icon(), default_theme_style(ThemeRole::Warning))
        }
        ToolStatus::ApprovedOnce | ToolStatus::ApprovedForChat => {
            Span::styled(status.icon(), default_theme_style(ThemeRole::Accent))
        }
        ToolStatus::Succeeded => Span::styled(
            status.icon(),
            default_theme_style(ThemeRole::Success).add_modifier(Modifier::BOLD),
        ),
        ToolStatus::Failed | ToolStatus::Denied => Span::styled(
            status.icon(),
            default_theme_style(ThemeRole::Error).add_modifier(Modifier::BOLD),
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
        .unwrap_or_else(|| format!("{}({})", tool_display_name(&card.name), card.args_preview))
}

fn search_label(card: &ToolCard) -> String {
    argument_value(
        card,
        &["pattern", "query", "search_key", "symbols", "path", "scope"],
    )
    .map(|query| format!("{} · {query}", tool_display_name(&card.name)))
    .unwrap_or_else(|| format!("{}({})", tool_display_name(&card.name), card.args_preview))
}

fn request_input_label(card: &ToolCard) -> String {
    argument_value(card, &["question", "prompt", "message", "title"])
        .unwrap_or_else(|| format!("{}({})", tool_display_name(&card.name), card.args_preview))
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
            default_theme_style(ThemeRole::Muted),
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
            default_theme_style(ThemeRole::Muted),
        )));
    }
    lines
}

fn output_style(line: &str) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        default_theme_style(ThemeRole::Success)
    } else if line.starts_with('-') && !line.starts_with("---") {
        default_theme_style(ThemeRole::Error)
    } else if line.starts_with("@@") {
        default_theme_style(ThemeRole::Accent).add_modifier(Modifier::BOLD)
    } else if line.starts_with("stderr") || line.contains("error") || line.contains("failed") {
        default_theme_style(ThemeRole::Error)
    } else {
        default_theme_style(ThemeRole::Text)
    }
}

fn default_theme_style(role: ThemeRole) -> Style {
    let mut style = Style::default();
    style.fg = TuiTheme::dark().style(role).fg;
    style
}

fn resolve_cell_lines_with_color_enabled(
    lines: Vec<Line<'static>>,
    theme: &TuiTheme,
    resolve_fallback_colors: bool,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|mut line| {
            line.style =
                resolve_cell_style(line.style, theme, resolve_fallback_colors, color_enabled);
            line.spans = line
                .spans
                .into_iter()
                .map(|mut span| {
                    span.style = resolve_cell_style(
                        span.style,
                        theme,
                        resolve_fallback_colors,
                        color_enabled,
                    );
                    span
                })
                .collect();
            line
        })
        .collect()
}

fn resolve_hyperlink_lines_with_color_enabled(
    lines: Vec<HyperlinkLine>,
    theme: &TuiTheme,
    resolve_fallback_colors: bool,
    color_enabled: bool,
) -> Vec<HyperlinkLine> {
    lines
        .into_iter()
        .map(|mut line| {
            line.line = resolve_cell_lines_with_color_enabled(
                vec![line.line],
                theme,
                resolve_fallback_colors,
                color_enabled,
            )
            .pop()
            .unwrap_or_default();
            line
        })
        .collect()
}

fn resolve_cell_style(
    mut style: Style,
    theme: &TuiTheme,
    resolve_fallback_colors: bool,
    color_enabled: bool,
) -> Style {
    if !color_enabled {
        style.fg = None;
        return style;
    }

    if !resolve_fallback_colors {
        return style;
    }

    let role = match style.fg {
        Some(Color::Black | Color::White | Color::Reset) => Some(ThemeRole::Text),
        Some(Color::Gray | Color::DarkGray) => Some(ThemeRole::Muted),
        Some(Color::Yellow | Color::LightYellow) => Some(ThemeRole::Warning),
        Some(Color::Red | Color::LightRed) => Some(ThemeRole::Error),
        Some(Color::Green | Color::LightGreen) => Some(ThemeRole::Success),
        Some(Color::Blue | Color::LightBlue | Color::Cyan | Color::LightCyan) => {
            Some(ThemeRole::Highlight)
        }
        Some(Color::Magenta | Color::LightMagenta) => Some(ThemeRole::Accent),
        Some(_) => None,
        None => None,
    };
    if let Some(role) = role {
        style.fg = theme.style(role).fg;
    }
    style
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
        .with_result(result, ToolStatus::Succeeded);
        card.duration_ms = Some(1200);
        card.expanded = true;
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_family_registry_classifies_tool_cells() {
        let cases = [
            ("shell", ToolFamily::Shell, ToolCellType::Exec),
            ("process_read", ToolFamily::Process, ToolCellType::Exec),
            ("apply_patch", ToolFamily::Diff, ToolCellType::Diff),
            ("web_search", ToolFamily::WebSearch, ToolCellType::Search),
            ("web", ToolFamily::WebFetch, ToolCellType::Search),
            (
                "search_pattern",
                ToolFamily::CodeSearch,
                ToolCellType::Search,
            ),
            ("cat", ToolFamily::FileSearch, ToolCellType::Search),
            ("tree", ToolFamily::TreeSearch, ToolCellType::Search),
            (
                "doc_list",
                ToolFamily::DocumentationSearch,
                ToolCellType::Search,
            ),
            (
                "knowledge",
                ToolFamily::KnowledgeSearch,
                ToolCellType::Search,
            ),
            (
                "ask_questions",
                ToolFamily::RequestInput,
                ToolCellType::RequestInput,
            ),
            ("web_search_call", ToolFamily::Server, ToolCellType::Server),
            (
                "totally_unknown",
                ToolFamily::Unknown,
                ToolCellType::Generic,
            ),
        ];

        for (name, family, cell_type) in cases {
            assert_eq!(tool_family(name), family, "{name}");
            assert_eq!(tool_cell_type_for(name), cell_type, "{name}");
        }
    }

    #[test]
    fn raw_lines_from_source_omits_trailing_empty_line() {
        assert_eq!(raw_lines_from_source("one\ntwo\n").len(), 2);
    }

    #[test]
    fn themes_change_transcript_colours_but_dark_preserves_them() {
        let item = TranscriptItem::Tool(test_support::tool_card(
            "shell",
            serde_json::json!({"command": "echo hi"}),
            "stdout: hi",
        ));
        let cell = cell_from_transcript_item(&item, false);
        let dark = resolve_cell_lines_with_color_enabled(
            cell.render_raw(80),
            &TuiTheme::dark(),
            true,
            true,
        );
        let light = resolve_cell_lines_with_color_enabled(
            cell.render_raw(80),
            &TuiTheme::light(),
            true,
            true,
        );

        let dark_colours = dark
            .iter()
            .flat_map(|line| {
                line.style
                    .fg
                    .into_iter()
                    .chain(line.spans.iter().filter_map(|span| span.style.fg))
            })
            .collect::<Vec<_>>();
        let light_colours = light
            .iter()
            .flat_map(|line| {
                line.style
                    .fg
                    .into_iter()
                    .chain(line.spans.iter().filter_map(|span| span.style.fg))
            })
            .collect::<Vec<_>>();

        assert!(dark_colours.contains(&TuiTheme::dark().style(ThemeRole::Highlight).fg.unwrap()));
        assert!(dark_colours.contains(&TuiTheme::dark().style(ThemeRole::Text).fg.unwrap()));
        assert!(light_colours.contains(&TuiTheme::light().style(ThemeRole::Highlight).fg.unwrap()));
        assert!(light_colours.contains(&TuiTheme::light().style(ThemeRole::Text).fg.unwrap()));
        assert_ne!(dark_colours, light_colours);
    }

    #[test]
    fn no_color_strips_transcript_cell_colours() {
        let item = TranscriptItem::Tool(test_support::tool_card(
            "shell",
            serde_json::json!({"command": "echo hi"}),
            "stderr: failed",
        ));
        let lines = resolve_cell_lines_with_color_enabled(
            cell_from_transcript_item(&item, false).render(80),
            &TuiTheme::dark(),
            true,
            false,
        );

        assert!(lines.iter().all(|line| {
            line.style.fg.is_none() && line.spans.iter().all(|span| span.style.fg.is_none())
        }));
    }
}
