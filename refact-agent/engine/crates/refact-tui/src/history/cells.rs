use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::TranscriptItem;
use crate::approvals::{render_modal_lines, ApprovalModalState};
use crate::render::{color_enabled_from_env, render_unified_diff, MarkdownRenderer};
use crate::tools::{ToolCard, ToolStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryCellKind {
    User,
    Assistant,
    Reasoning,
    Notice,
    Tool,
    Citation,
    ServerContentBlock,
    Diff,
    Approval,
    Session,
    Plan,
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
}

impl UserCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for UserCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::User
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            "you",
            Style::default()
                .fg(Color::Blue)
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
            "server content",
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlanCell {
    text: String,
}

impl PlanCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for PlanCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Plan
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![role_line(
            "plan",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(renderer.render(&self.text));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&("plan", &self.text))
    }
}

pub fn cell_from_transcript_item(item: &TranscriptItem, selected: bool) -> Box<dyn HistoryCell> {
    match item {
        TranscriptItem::User(text) => Box::new(UserCell::new(text.clone())),
        TranscriptItem::Assistant(text) => Box::new(AssistantCell::new(text.clone())),
        TranscriptItem::Reasoning(text, collapsed) => {
            Box::new(ReasoningCell::new(text.clone(), *collapsed))
        }
        TranscriptItem::Tool(card) => Box::new(ToolCallCell::new(card.clone(), selected)),
        TranscriptItem::Citation(text) => Box::new(CitationCell::new(text.clone())),
        TranscriptItem::ServerContentBlock(text) => {
            Box::new(ServerContentBlockCell::new(text.clone()))
        }
        TranscriptItem::Diff(text) => Box::new(DiffCell::new(text.clone())),
        TranscriptItem::Notice(text) => Box::new(NoticeCell::new(text.clone())),
        TranscriptItem::Approval(state, outcome) => {
            Box::new(ApprovalCell::new(state.clone(), *outcome))
        }
        TranscriptItem::Session { title, subtitle } => {
            Box::new(SessionCell::new(title.clone(), subtitle.clone()))
        }
        TranscriptItem::Plan(text) => Box::new(PlanCell::new(text.clone())),
    }
}

pub fn render_transcript_item_lines(
    item: &TranscriptItem,
    width: usize,
    selected: bool,
) -> Vec<Line<'static>> {
    cell_from_transcript_item(item, selected).render(width)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::PauseReason;
    use serde_json::json;

    fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
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
        }])
    }

    #[test]
    fn user_cell_snapshot() {
        let cell = UserCell::new("hello **there**");
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
            text(&ServerContentBlockCell::new("{\"type\":\"web_search_call\"}").render(40)),
            "server content\n{\"type\":\"web_search_call\"}\n"
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
        let cell = PlanCell::new("## Plan\n- do it");
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("plan\n"));
        assert!(rendered.contains("Plan"));
        assert!(rendered.contains("do it"));
    }

    #[test]
    fn tool_cell_snapshot() {
        let mut card = ToolCard::from_tool_call(&json!({
            "id": "call-1",
            "function": {"name": "shell", "arguments": "{\"cmd\":\"echo hi\"}"}
        }))
        .with_result("line 1", ToolStatus::Success);
        card.duration_ms = Some(1200);
        card.expanded = true;
        let cell = ToolCallCell::new(card, true);
        assert!(cell.is_final());
        assert!(text(&cell.render(80)).contains("tool selected\n▾ ✅ shell"));
        assert!(text(&cell.render(80)).contains("line 1"));
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
