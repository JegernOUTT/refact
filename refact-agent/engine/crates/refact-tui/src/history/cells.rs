use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::app::TranscriptItem;
use crate::approvals::{render_modal_lines, ApprovalModalState};
use crate::render::{color_enabled_from_env, is_unified_diff, render_unified_diff, MarkdownRenderer};
use crate::tools::{ToolCard, ToolStatus};

const COLLAPSED_OUTPUT_LINES: usize = 12;
const EXPANDED_OUTPUT_LINES: usize = 200;
const PLAN_SYNTHESIS_SEPARATOR: &str = "\n\n---\n\n## Plan updates\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryCellKind {
    User,
    Assistant,
    Reasoning,
    Notice,
    Info,
    Tool,
    Exec,
    Diff,
    Plan,
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

pub trait HistoryCell: HistoryCellClone + std::fmt::Debug + Send + Sync {
    fn kind(&self) -> HistoryCellKind;
    fn render(&self, width: usize) -> Vec<Line<'static>>;
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserCell {
    text: String,
    selected: bool,
}

impl UserCell {
    pub fn new(text: impl Into<String>, selected: bool) -> Self {
        Self {
            text: text.into(),
            selected,
        }
    }
}

impl HistoryCell for UserCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::User
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            if self.selected { "you selected" } else { "you" },
            Style::default()
                .fg(if self.selected {
                    Color::Cyan
                } else {
                    Color::Blue
                })
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(renderer.render(&self.text));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text, self.selected))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssistantCell {
    text: String,
}

impl AssistantCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for AssistantCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Assistant
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            "assistant",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(renderer.render(&self.text));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReasoningCell {
    text: String,
    collapsed: bool,
}

impl ReasoningCell {
    pub fn new(text: impl Into<String>, collapsed: bool) -> Self {
        Self {
            text: text.into(),
            collapsed,
        }
    }

    pub fn update_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }
}

impl HistoryCell for ReasoningCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Reasoning
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let label = if self.collapsed {
            "reasoning collapsed"
        } else {
            "reasoning"
        };
        let mut lines = vec![role_line(label, Style::default().fg(Color::DarkGray))];
        if !self.collapsed {
            lines.extend(renderer.render(&self.text));
        }
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text, self.collapsed))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoticeCell {
    text: String,
}

impl NoticeCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for NoticeCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Notice
    }

    fn render(&self, _width: usize) -> Vec<Line<'static>> {
        finish(vec![Line::from(Span::styled(
            self.text.clone(),
            Style::default().fg(Color::DarkGray),
        ))])
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfoCell {
    lines: Vec<String>,
}

impl InfoCell {
    pub fn new(lines: Vec<String>) -> Self {
        Self { lines }
    }
}

impl HistoryCell for InfoCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Info
    }

    fn render(&self, _width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (idx, text) in self.lines.iter().enumerate() {
            let style = if idx == 0 {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(Span::styled(text.clone(), style)));
        }
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.lines))
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
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line("citation", Style::default().fg(Color::Cyan))];
        lines.extend(renderer.render(&self.text));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

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
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            server_content_label(&self.text),
            Style::default().fg(Color::Magenta),
        )];
        lines.extend(renderer.render(&self.text));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffCell {
    text: String,
}

impl DiffCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for DiffCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Diff
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![role_line(
            "diff",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(render_unified_diff(
            &self.text,
            Some(width.saturating_sub(2).max(8)),
            color_enabled_from_env(),
        ));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallCell {
    card: ToolCard,
    selected: bool,
}

impl ToolCallCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }

    pub fn card(&self) -> &ToolCard {
        &self.card
    }
}

impl HistoryCell for ToolCallCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Tool
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let label = if self.selected {
            "tool selected"
        } else {
            "tool"
        };
        let mut lines = vec![role_line(
            label,
            Style::default().fg(if self.selected {
                Color::Cyan
            } else {
                Color::Yellow
            }),
        )];
        lines.extend(self.card.render_lines(width));
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
pub struct ExecToolCell {
    card: ToolCard,
    selected: bool,
}

impl ExecToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for ExecToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Exec
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![role_line(
            if self.selected {
                "exec selected"
            } else {
                "exec"
            },
            Style::default().fg(Color::Cyan),
        )];
        let mut meta = Vec::new();
        if let Some(exit_code) = exit_code_from_result(&self.card.result) {
            meta.push(format!("exit {exit_code}"));
        }
        if let Some(duration_ms) = self.card.duration_ms {
            meta.push(format_duration(duration_ms));
        }
        lines.push(tool_summary_line(
            &self.card,
            command_label(&self.card),
            meta.join(" · "),
        ));
        if self.card.expanded {
            lines.extend(output_lines(
                &self.card.result,
                width,
                EXPANDED_OUTPUT_LINES,
                false,
            ));
        } else if !self.card.result.is_empty() {
            lines.extend(output_lines(
                &self.card.result,
                width,
                COLLAPSED_OUTPUT_LINES,
                true,
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
pub struct DiffToolCell {
    card: ToolCard,
    selected: bool,
}

impl DiffToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for DiffToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Diff
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let source = diff_source(&self.card);
        let stats = diff_file_stats(&source);
        let mut lines = vec![role_line(
            if self.selected {
                "diff selected"
            } else {
                "diff"
            },
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )];
        lines.push(tool_summary_line(
            &self.card,
            diff_summary(&stats),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        for stat in &stats {
            lines.push(Line::from(vec![
                Span::styled("Δ ", Style::default().fg(Color::Blue)),
                Span::raw(stat.path.clone()),
                Span::styled(
                    format!(" +{}", stat.added),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(
                    format!(" -{}", stat.deleted),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }
        if self.card.expanded {
            if is_unified_diff(&source) {
                lines.extend(
                    render_unified_diff(
                        &source,
                        Some(width.saturating_sub(2).max(8)),
                        color_enabled_from_env(),
                    )
                    .into_iter()
                    .take(EXPANDED_OUTPUT_LINES),
                );
            } else {
                lines.extend(output_lines(&source, width, EXPANDED_OUTPUT_LINES, false));
            }
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
        let mut lines = vec![role_line(
            if self.selected {
                "server tool selected"
            } else {
                "server tool"
            },
            Style::default().fg(Color::Magenta),
        )];
        lines.push(tool_summary_line(
            &self.card,
            self.card.name.clone(),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        if self.card.expanded && !self.card.result.is_empty() {
            lines.extend(output_lines(
                &self.card.result,
                width,
                EXPANDED_OUTPUT_LINES,
                false,
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
        let mut lines = vec![role_line(
            if self.selected {
                "search selected"
            } else {
                "search"
            },
            Style::default().fg(Color::Cyan),
        )];
        lines.push(tool_summary_line(
            &self.card,
            search_label(&self.card),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        if self.card.expanded && !self.card.result.is_empty() {
            lines.extend(output_lines(
                &self.card.result,
                width,
                EXPANDED_OUTPUT_LINES,
                false,
            ));
        } else if !self.card.result.is_empty() {
            lines.extend(output_lines(
                &self.card.result,
                width,
                COLLAPSED_OUTPUT_LINES,
                true,
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
pub struct RequestInputToolCell {
    card: ToolCard,
    selected: bool,
}

impl RequestInputToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for RequestInputToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::RequestInput
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            if self.selected {
                "input request selected"
            } else {
                "input request"
            },
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )];
        lines.push(tool_summary_line(
            &self.card,
            request_input_label(&self.card),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        if self.card.expanded {
            if let Some(prompt) = argument_value(
                &self.card,
                &["question", "questions", "prompt", "message", "title"],
            ) {
                lines.extend(renderer.render(&prompt));
            }
            if !self.card.result.is_empty() {
                lines.extend(output_lines(
                    &self.card.result,
                    width,
                    EXPANDED_OUTPUT_LINES,
                    false,
                ));
            }
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
}

impl HistoryCell for PlanCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Plan
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let update_label = if self.data.delta_count == 1 {
            "1 update".to_string()
        } else {
            format!("{} updates", self.data.delta_count)
        };
        let mut lines = vec![role_line(
            format!(
                "plan · {} · v{} · {}",
                self.data.mode, self.data.version, update_label
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(renderer.render(&self.data.content));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.data))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventCellData {
    pub subkind: String,
    pub source: String,
    pub content: String,
}

impl EventCellData {
    pub fn new(
        subkind: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            subkind: subkind.into(),
            source: source.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventCell {
    data: EventCellData,
}

impl EventCell {
    pub fn new(data: EventCellData) -> Self {
        Self { data }
    }
}

impl HistoryCell for EventCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Event
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            format!("event · {} · {}", self.data.subkind, self.data.source),
            Style::default().fg(Color::DarkGray),
        )];
        lines.extend(renderer.render(&self.data.content));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.data))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalOutcome {
    ApprovedOnce,
    ApprovedForChat,
    Denied,
}

impl ApprovalOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::ApprovedOnce => "approved once",
            Self::ApprovedForChat => "approved for chat",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCell {
    state: ApprovalModalState,
    outcome: Option<ApprovalOutcome>,
}

impl ApprovalCell {
    pub fn new(state: ApprovalModalState, outcome: Option<ApprovalOutcome>) -> Self {
        Self { state, outcome }
    }

    pub fn set_outcome(&mut self, outcome: ApprovalOutcome) {
        self.outcome = Some(outcome);
    }
}

impl HistoryCell for ApprovalCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Approval
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = render_modal_lines(&self.state, width);
        if let Some(outcome) = self.outcome {
            lines.push(Line::from(Span::styled(
                format!("approval {}", outcome.label()),
                Style::default().fg(Color::DarkGray),
            )));
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.outcome.is_some()
    }

    fn revision(&self) -> u64 {
        let reasons = self
            .state
            .reasons()
            .iter()
            .map(|reason| {
                (
                    reason.reason_type.as_str(),
                    reason.tool_name.as_str(),
                    reason.command.as_str(),
                    reason.rule.as_str(),
                    reason.tool_call_id.as_str(),
                    reason.integr_config_path.as_deref().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        revision(&(
            self.kind(),
            self.state.full_args(),
            self.state.pending_after(),
            reasons,
            self.outcome,
        ))
    }
}

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
        let mut lines = vec![Line::from(Span::styled(
            self.title.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))];
        if let Some(subtitle) = &self.subtitle {
            lines.push(Line::from(Span::styled(
                subtitle.clone(),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(Span::styled(
            "─".repeat(width.max(1).min(120)),
            Style::default().fg(Color::DarkGray),
        )));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.title, &self.subtitle))
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
        TranscriptItem::Citation(text) => Box::new(CitationCell::new(text.clone())),
        TranscriptItem::ServerContentBlock(text) => {
            Box::new(ServerContentBlockCell::new(text.clone()))
        }
        TranscriptItem::Diff(text) => Box::new(DiffCell::new(text.clone())),
        TranscriptItem::Notice(text) => Box::new(NoticeCell::new(text.clone())),
        TranscriptItem::Info(lines) => Box::new(InfoCell::new(lines.clone())),
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
    match value {
        Value::String(value) => value.clone(),
        value => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
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
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
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
mod tests {
    use super::*;
    use crate::approvals::PauseReason;
    use crate::render::wrapping::line_to_plain;
    use serde_json::json;

    fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(line_to_plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn approval_state() -> ApprovalModalState {
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

    fn tool_card(name: &str, args: Value, result: &str) -> ToolCard {
        let mut card = ToolCard::from_tool_call(&json!({
            "id": format!("call-{name}"),
            "function": {"name": name, "arguments": args.to_string()}
        }))
        .with_result(result, ToolStatus::Success);
        card.duration_ms = Some(1200);
        card.expanded = true;
        card
    }

    #[test]
    fn user_cell_snapshot() {
        let cell = UserCell::new("hello **there**", false);
        assert_eq!(text(&cell.render(40)), "you\nhello there\n");
    }

    #[test]
    fn assistant_cell_snapshot() {
        let cell = AssistantCell::new("| A | B |\n|---|---|\n| one | two |");
        assert_eq!(
            text(&cell.render(40)),
            "assistant\nA   │ B  \n━━━━━━━━━\none │ two\n"
        );
    }

    #[test]
    fn reasoning_cell_snapshot_and_update_preserves_collapse() {
        let mut cell = ReasoningCell::new("hidden plan", false);
        assert_eq!(text(&cell.render(40)), "reasoning\nhidden plan\n");
        cell.update_text("updated plan");
        assert!(!cell.collapsed());
        cell.set_collapsed(true);
        assert_eq!(text(&cell.render(40)), "reasoning collapsed\n");
    }

    #[test]
    fn notice_cell_snapshot() {
        let cell = NoticeCell::new("SSE disconnected");
        assert_eq!(text(&cell.render(40)), "SSE disconnected\n");
    }

    #[test]
    fn citation_and_server_cells_snapshot() {
        assert_eq!(
            text(&CitationCell::new("{\"title\":\"README\"}").render(40)),
            "citation\n{\"title\":\"README\"}\n"
        );
        assert_eq!(
            text(&ServerContentBlockCell::new("{\"type\":\"web_search_call\",\"status\":\"completed\"}").render(80)),
            "server content · web_search_call · completed\n{\"type\":\"web_search_call\",\"status\":\"completed\"}\n"
        );
    }

    #[test]
    fn diff_cell_reuses_unified_diff_renderer() {
        let cell = DiffCell::new("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new");
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("diff\n"));
        assert!(rendered.contains("- old"));
        assert!(rendered.contains("+ new"));
    }

    #[test]
    fn plan_cell_renders_markdown_plan() {
        let cell = PlanCell::new(PlanCellData::new("## Plan\n- do it", "agent", 1, 0));
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("plan · agent · v1"));
        assert!(rendered.contains("Plan"));
        assert!(rendered.contains("do it"));
    }

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
    fn unknown_tool_cell_keeps_generic_card() {
        let card = tool_card("totally_unknown", json!({"x": 1}), "line 1");
        let cell = ToolCallCell::new(card, true);
        assert!(cell.is_final());
        assert!(text(&cell.render(80)).contains("tool selected\n▾ ✅ totally_unknown"));
        assert!(text(&cell.render(80)).contains("line 1"));
    }

    #[test]
    fn exec_cell_snapshot_extracts_command_exit_and_output() {
        let card = tool_card(
            "shell",
            json!({"command": "echo hi"}),
            "stdout:\nhi\n\nThe command was running 0.120s, finished with exit code 0",
        );
        let cell = cell_from_tool_card(card, true);
        assert_eq!(cell.kind(), HistoryCellKind::Exec);
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("exec selected\n▾ ✅ $ echo hi · exit 0 · 1.2s"));
        assert!(rendered.contains("stdout:\nhi"));
    }

    #[test]
    fn exec_cell_truncates_collapsed_output_with_expand_hint() {
        let mut card = tool_card(
            "process_read",
            json!({"process_id": "exec_1"}),
            &(0..15)
                .map(|idx| format!("line {idx}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        card.expanded = false;
        let rendered = text(&ExecToolCell::new(card, false).render(80));
        assert!(rendered.contains("… 3 more lines (expand)"));
    }

    #[test]
    fn diff_cell_snapshot_reuses_unified_diff_renderer() {
        let card = tool_card(
            "apply_patch",
            json!({}),
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new",
        );
        let cell = cell_from_tool_card(card, false);
        assert_eq!(cell.kind(), HistoryCellKind::Diff);
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("diff\n▾ ✅ 1 file · +1 -1 · 1.2s"));
        assert!(rendered.contains("Δ x +1 -1"));
        assert!(rendered.contains("- old"));
        assert!(rendered.contains("+ new"));
    }

    #[test]
    fn search_cell_snapshot() {
        let card = tool_card(
            "search_pattern",
            json!({"pattern": "needle", "scope": "src"}),
            "src/main.rs:1: needle",
        );
        let rendered = text(&SearchToolCell::new(card, false).render(80));
        assert!(rendered.contains("search\n▾ ✅ search_pattern · needle · 1.2s"));
        assert!(rendered.contains("src/main.rs:1: needle"));
    }

    #[test]
    fn request_input_cell_snapshot() {
        let card = tool_card(
            "ask_questions",
            json!({"question": "Which file should I edit?"}),
            "waiting for user input",
        );
        let rendered = text(&RequestInputToolCell::new(card, true).render(80));
        assert!(rendered.contains("input request selected"));
        assert!(rendered.contains("Which file should I edit?"));
        assert!(rendered.contains("waiting for user input"));
    }

    #[test]
    fn plan_cell_snapshot_merges_deltas() {
        let content = synthesize_plan_content(
            "## Plan\n- base",
            &["first update".to_string(), "second update".to_string()],
        );
        let cell = PlanCell::new(PlanCellData::new(content, "agent", 2, 2));
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("plan · agent · v2 · 2 updates"));
        assert!(rendered.contains("Plan updates"));
        assert!(rendered.contains("second update"));
    }

    #[test]
    fn event_cell_snapshot() {
        let cell = EventCell::new(EventCellData::new(
            "process_completed",
            "exec.registry",
            "Process exited with code 0",
        ));
        assert_eq!(
            text(&cell.render(80)),
            "event · process_completed · exec.registry\nProcess exited with code 0\n"
        );
    }

    #[test]
    fn approval_cell_snapshot() {
        let mut cell = ApprovalCell::new(approval_state(), None);
        assert!(!cell.is_final());
        assert!(text(&cell.render(80)).contains("Approval required"));
        cell.set_outcome(ApprovalOutcome::ApprovedOnce);
        assert!(cell.is_final());
        assert!(text(&cell.render(80)).contains("approval approved once"));
    }

    #[test]
    fn session_cell_snapshot() {
        let cell = SessionCell::new("New chat started", Some("agent mode".to_string()));
        assert_eq!(
            text(&cell.render(12)),
            "New chat started\nagent mode\n────────────\n"
        );
    }
}
