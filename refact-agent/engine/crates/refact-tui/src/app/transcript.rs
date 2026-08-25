use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use serde_json::{json, Value};

use base64::Engine;

use crate::history::cells::{
    synthesize_goal_content, synthesize_plan_content, GoalCellData, HistoryCellKind, PlanCellData,
};
use crate::tools::{ToolCard, ToolStatus};
use crate::terminal_image::InlineImage;

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptItem {
    User(String),
    Assistant(String),
    Reasoning(String, bool),
    ContentBlock {
        summary: String,
        body: String,
        collapsed: bool,
        expandable: bool,
    },
    Image {
        placeholder: String,
        data: Vec<u8>,
        mime: String,
    },
    Tool(ToolCard),
    Plan(PlanCellData),
    Goal(GoalCellData),
    PlanStream(Vec<crate::vendored::terminal_hyperlinks::HyperlinkLine>),
    Diff(String),
    Notice(String),
    Info(Vec<String>),
    SystemFact(Vec<String>),
    Status(session::StatusSnapshot, TuiTheme),
    Approval(ApprovalModalState, Option<ToolStatus>),
    Session {
        title: String,
        subtitle: Option<String>,
    },
}

impl TranscriptItem {
    pub(super) fn keeps_live(&self) -> bool {
        matches!(self, Self::Tool(card) if card.status.is_active())
            || matches!(self, Self::Approval(_, None))
            || matches!(self, Self::Plan(_))
            || matches!(self, Self::Goal(_))
            || matches!(self, Self::PlanStream(_))
    }

    pub(super) fn can_enter_history(&self) -> bool {
        !matches!(self, Self::Assistant(text) if text.is_empty())
    }
}

pub(super) fn image_items(message: &TranscriptMessage) -> Vec<TranscriptItem> {
    message
        .images
        .iter()
        .filter_map(|image| {
            let data = base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .ok()?;
            let placeholder = format!("[image: {}, {} bytes]", image.mime, data.len());
            Some(TranscriptItem::Image {
                placeholder,
                data,
                mime: image.mime.clone(),
            })
        })
        .collect()
}

fn assistant_content_without_images(content: &str, images: &[TranscriptItem]) -> String {
    let placeholders = images
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::Image { placeholder, .. } => Some(placeholder.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    content
        .lines()
        .filter(|line| !placeholders.contains(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn inline_images_for_visible_transcript(
    items: &[TranscriptItem],
    protocol: Option<crate::terminal_probe::ImageProtocol>,
    positions: &[ratatui::layout::Position],
) -> Vec<InlineImage> {
    let Some(protocol) = protocol else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::Image { data, mime, .. } => Some((data, mime)),
            _ => None,
        })
        .zip(positions.iter().copied())
        .map(|((data, mime), position)| {
            InlineImage::new(protocol, data.clone(), mime.clone(), position)
        })
        .collect()
}

impl App {
    pub(super) fn open_latest_image_action(&mut self) -> AppAction {
        let image = image_items_for_transcript(&self.transcript)
            .last()
            .cloned()
            .or_else(|| {
                self.history.latest_image_data().map(|(data, mime)| {
                    InlineImage::new(
                        crate::terminal_probe::ImageProtocol::Kitty,
                        data,
                        mime,
                        ratatui::layout::Position::ORIGIN,
                    )
                })
            });
        let Some(image) = image else {
            return AppAction::None;
        };
        AppAction::OpenImageExternally { image }
    }
}

fn image_items_for_transcript(items: &[TranscriptItem]) -> Vec<InlineImage> {
    items
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::Image { data, mime, .. } => Some(InlineImage::new(
                crate::terminal_probe::ImageProtocol::Kitty,
                data.clone(),
                mime.clone(),
                ratatui::layout::Position::ORIGIN,
            )),
            _ => None,
        })
        .collect()
}

pub(super) fn thinking_block_items(message: &TranscriptMessage) -> Vec<TranscriptItem> {
    message
        .thinking_blocks
        .iter()
        .map(thinking_block_item)
        .collect()
}

pub(super) fn citation_item(citation: &Value) -> TranscriptItem {
    let title = citation
        .get("title")
        .or_else(|| citation.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let url = citation
        .get("url")
        .or_else(|| citation.get("uri"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let summary = match (title, url) {
        (Some(title), _) => format!("Citation: {}", sanitize_tool_inline(title)),
        (None, Some(url)) => format!("Citation: {}", sanitize_tool_inline(url)),
        (None, None) => "Citation".to_string(),
    };
    let mut body = Vec::new();
    if let Some(title) = title {
        body.push(format!("Title: {}", sanitize_tool_text(title)));
    }
    if let Some(url) = url {
        body.push(format!("URL: {}", sanitize_tool_text(url)));
    }
    if let Some(text) = citation
        .get("text")
        .or_else(|| citation.get("content"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        body.push(format!("Excerpt: {}", sanitize_tool_text(text)));
    }
    TranscriptItem::ContentBlock {
        summary,
        body: body.join("\n"),
        collapsed: true,
        expandable: !body.is_empty(),
    }
}

pub(super) fn server_content_block_item(block: &Value) -> TranscriptItem {
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("server content");
    let status = block
        .get("status")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let summary = match status {
        Some(status) => format!(
            "Server: {} · {}",
            sanitize_tool_inline(kind),
            sanitize_tool_inline(status)
        ),
        None => format!("Server: {}", sanitize_tool_inline(kind)),
    };
    let body =
        serde_json::to_string_pretty(block).unwrap_or_else(|_| value_to_compact_string(block));
    TranscriptItem::ContentBlock {
        summary,
        body: sanitize_tool_text(body),
        collapsed: true,
        expandable: true,
    }
}

fn thinking_block_item(block: &Value) -> TranscriptItem {
    if block.get("type").and_then(Value::as_str) == Some("redacted_thinking") {
        return TranscriptItem::ContentBlock {
            summary: "🧠 [reasoning redacted by provider]".to_string(),
            body: String::new(),
            collapsed: true,
            expandable: false,
        };
    }
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("reasoning");
    let body = block
        .get("thinking")
        .or_else(|| block.get("text"))
        .or_else(|| block.get("content"))
        .or_else(|| block.get("summary"))
        .and_then(thinking_block_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(provider reasoning metadata)".to_string());
    TranscriptItem::ContentBlock {
        summary: format!("🧠 {}", sanitize_tool_inline(kind)),
        body: sanitize_tool_text(body),
        collapsed: true,
        expandable: true,
    }
}

fn thinking_block_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Null => None,
        value => Some(value_to_compact_string(value)),
    }
}

impl App {
    pub fn transcript_item_selected(&self, visible_index: usize, item: &TranscriptItem) -> bool {
        if self.selected_tool_index == Some(visible_index) {
            return true;
        }
        if !matches!(item, TranscriptItem::User(_)) {
            return false;
        }
        let Some(selected_index) = self.selected_backtrack_index else {
            return false;
        };
        let target_ordinal = self
            .transcript_state
            .messages()
            .iter()
            .take(selected_index.saturating_add(1))
            .filter(|message| message.role == TranscriptRole::User)
            .count();
        if target_ordinal == 0 {
            return false;
        }
        self.transcript
            .iter()
            .take(visible_index.saturating_add(1))
            .filter(|item| matches!(item, TranscriptItem::User(_)))
            .count()
            == target_ordinal
    }

    pub(super) fn copy_last_assistant_message(&mut self) -> AppAction {
        self.composer.clear();
        let Some(text) = self.last_assistant_rendered_plain_text(100) else {
            self.add_notice("No assistant message to copy");
            return AppAction::None;
        };
        AppAction::CopyToClipboard {
            text,
            source: ClipboardCopySource::LastAssistant,
        }
    }

    pub(super) fn last_assistant_rendered_plain_text(&self, width: usize) -> Option<String> {
        let message = self
            .transcript_state
            .messages()
            .iter()
            .rev()
            .find(|message| {
                message.role == TranscriptRole::Assistant && !message.content.is_empty()
            })?;
        let lines = crate::render::MarkdownRenderer::plain(Some(width)).render(&message.content);
        Some(
            lines
                .into_iter()
                .map(|line| line_to_plain_string(&line))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub(super) fn record_clipboard_result(
        &mut self,
        source: ClipboardCopySource,
        result: std::io::Result<crate::clipboard::ClipboardCopyReport>,
    ) {
        match result {
            Ok(report) => {
                let label = match source {
                    ClipboardCopySource::LastAssistant => "assistant message",
                    ClipboardCopySource::OverlayVisible => "visible overlay text",
                };
                if report.truncated {
                    self.add_notice(format!(
                        "Copied {label} to terminal clipboard via OSC52 (truncated to {} of {} bytes)",
                        report.copied_bytes, report.original_bytes
                    ));
                } else {
                    self.add_notice(format!(
                        "Copied {label} to terminal clipboard via OSC52 ({} bytes)",
                        report.copied_bytes
                    ));
                }
            }
            Err(error) => self.add_notice(format!("Clipboard copy failed: {error}")),
        }
    }
}

impl App {
    pub(super) fn append_assistant(&mut self, text: &str) {
        self.stream_controller.push_sanitized_delta(text);
        self.sync_assistant_stream_item();
    }

    pub(super) fn append_plan_stream(&mut self, key: String, text: &str) {
        if !self.record_state_history_key(key) && self.plan_stream_controller.is_some() {
            self.sync_plan_stream_item();
            return;
        }
        if self.plan_stream_controller.is_none() {
            let mut controller = PlanStreamController::new(None, std::path::Path::new("."));
            controller.set_render_mode(self.history_render_mode);
            self.plan_stream_controller = Some(controller);
        }
        if let Some(controller) = &mut self.plan_stream_controller {
            controller.push_sanitized_delta(text);
        }
        self.sync_plan_stream_item();
    }

    pub(super) fn sync_plan_stream_item(&mut self) {
        let Some(controller) = self.plan_stream_controller.as_mut() else {
            return;
        };
        let lines = controller.visible_display_lines();
        self.sync_plan_stream_lines(lines);
    }

    pub(super) fn sync_plan_stream_tail_item(&mut self) {
        let Some(controller) = self.plan_stream_controller.as_mut() else {
            return;
        };
        let lines = controller.current_tail_display_lines();
        self.sync_plan_stream_lines(lines);
    }

    pub(super) fn sync_plan_stream_lines(
        &mut self,
        lines: Vec<crate::vendored::terminal_hyperlinks::HyperlinkLine>,
    ) {
        match self.transcript.last_mut() {
            Some(TranscriptItem::PlanStream(_)) if lines.is_empty() => {
                self.transcript.pop();
            }
            Some(TranscriptItem::PlanStream(value)) => *value = lines,
            _ if !lines.is_empty() => self.push_live_item(TranscriptItem::PlanStream(lines)),
            _ => {}
        }
    }

    pub(super) fn sync_assistant_stream_item(&mut self) {
        let visible = self.stream_controller.visible();
        if visible.is_empty() {
            return;
        }
        match self.transcript.last_mut() {
            Some(TranscriptItem::Assistant(value)) => *value = visible,
            _ => self.push_live_item(TranscriptItem::Assistant(visible)),
        }
    }

    pub(super) fn sync_assistant_stream_tail_item(&mut self) {
        let tail = self.stream_controller.live();
        match self.transcript.last_mut() {
            Some(TranscriptItem::Assistant(_)) if tail.is_empty() => {
                self.transcript.pop();
            }
            Some(TranscriptItem::Assistant(value)) => *value = tail,
            _ if !tail.is_empty() => self.push_live_item(TranscriptItem::Assistant(tail)),
            _ => {}
        }
    }

    pub(super) fn push_state_history_item(&mut self, key: String, item: TranscriptItem) {
        if !self.record_state_history_key(key) {
            return;
        }
        self.push_recorded_state_history_item(item);
    }

    pub(super) fn push_state_reasoning_item(
        &mut self,
        key: String,
        text: String,
        collapsed: bool,
        replace_live: bool,
    ) {
        if !self.record_state_history_key(key) {
            return;
        }
        if replace_live {
            self.replace_reasoning_stream_with_final(text, collapsed);
        } else {
            self.push_recorded_state_history_item(TranscriptItem::Reasoning(text, collapsed));
        }
    }

    pub(super) fn record_state_history_key(&mut self, key: String) -> bool {
        if self
            .rendered_state_keys
            .get(self.rendered_state_cursor)
            .is_some_and(|existing| existing == &key)
        {
            self.rendered_state_cursor += 1;
            return false;
        }
        if state_key_has_stable_identity(&key)
            && self.rendered_state_keys[..self.rendered_state_cursor]
                .iter()
                .any(|existing| existing == &key)
        {
            return false;
        }
        self.rendered_state_keys
            .truncate(self.rendered_state_cursor);
        self.rendered_state_keys.push(key);
        self.rendered_state_cursor += 1;
        true
    }

    pub(super) fn push_recorded_state_history_item(&mut self, item: TranscriptItem) {
        match item {
            TranscriptItem::Assistant(text) if self.assistant_stream_active() => {
                self.replace_assistant_stream_with_final(text);
            }
            item => self.push_history_item(item),
        }
    }

    pub(super) fn assistant_stream_active(&self) -> bool {
        !self.stream_controller.visible().is_empty()
            || self.stream_controller.has_live_tail()
            || self.stream_controller.stable_lines_ready()
    }

    pub(super) fn replace_assistant_stream_with_final(&mut self, text: String) {
        self.stream_controller.clear();
        self.stream_chunking_policy.reset();
        if self.native_scrollback {
            self.transcript
                .retain(|item| !matches!(item, TranscriptItem::Assistant(_)));
            if self
                .history
                .remove_non_final_cells(HistoryCellKind::Assistant)
                > 0
            {
                self.resize_reflow.schedule_immediate();
            }
            self.history.enqueue(TranscriptItem::Assistant(text));
            return;
        }
        if let Some(existing) = self
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Assistant(existing) => Some(existing),
                _ => None,
            })
        {
            *existing = text;
        } else {
            self.push_history_item(TranscriptItem::Assistant(text));
        }
    }

    pub(super) fn replace_reasoning_stream_with_final(&mut self, text: String, collapsed: bool) {
        let collapsed = self
            .transcript
            .iter()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Reasoning(_, collapsed) => Some(*collapsed),
                _ => None,
            })
            .unwrap_or(collapsed);
        self.reasoning_stream_active = false;
        if self.native_scrollback {
            self.transcript
                .retain(|item| !matches!(item, TranscriptItem::Reasoning(_, _)));
            if self
                .history
                .remove_non_final_cells(HistoryCellKind::Reasoning)
                > 0
            {
                self.resize_reflow.schedule_immediate();
            }
            self.history
                .enqueue(TranscriptItem::Reasoning(text, collapsed));
            return;
        }
        if let Some((existing, existing_collapsed)) =
            self.transcript
                .iter_mut()
                .rev()
                .find_map(|item| match item {
                    TranscriptItem::Reasoning(existing, existing_collapsed) => {
                        Some((existing, existing_collapsed))
                    }
                    _ => None,
                })
        {
            *existing = text;
            *existing_collapsed = collapsed;
        } else {
            self.push_history_item(TranscriptItem::Reasoning(text, collapsed));
        }
    }

    pub(super) fn replace_live_region_from_snapshot(&mut self, next_keys: &[String]) {
        self.transcript.clear();
        self.clear_stream_controllers();
        if self.rendered_state_keys != next_keys {
            let inserted = self.native_scrollback && self.history.inserted_cell_count() > 0;
            self.history.clear_pending();
            self.rendered_state_keys.clear();
            if inserted {
                self.resize_reflow.schedule_immediate();
            }
        }
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
    }

    pub(super) fn mark_rendered_state_from_messages(&mut self) {
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        let messages = self.transcript_state.messages().to_vec();
        for message in &messages {
            for key in rendered_state_keys_for_message(message) {
                self.record_state_history_key(key);
            }
        }
        self.rendered_state_keys
            .truncate(self.rendered_state_cursor);
    }

    pub(super) fn push_history_item(&mut self, item: TranscriptItem) {
        if !item.can_enter_history() {
            return;
        }
        if self.native_scrollback && !item.keeps_live() {
            self.history.enqueue(item);
        } else {
            self.push_live_item(item);
        }
    }

    pub(super) fn push_live_item(&mut self, item: TranscriptItem) {
        self.transcript.push(item);
        self.enforce_live_transcript_limit();
    }

    pub(super) fn enforce_live_transcript_limit(&mut self) {
        if self.transcript.len() <= LIVE_TRANSCRIPT_ITEM_LIMIT {
            return;
        }
        let has_notice = matches!(
            self.transcript.first(),
            Some(TranscriptItem::Notice(text)) if text == LIVE_TRANSCRIPT_RETENTION_NOTICE
        );
        let target_len = LIVE_TRANSCRIPT_ITEM_LIMIT.saturating_sub(usize::from(!has_notice));
        let remove_count = self.transcript.len().saturating_sub(target_len);
        let mut remove_indices = self
            .transcript
            .iter()
            .enumerate()
            .filter(|(idx, item)| {
                !item.keeps_live()
                    && !(*idx == 0
                        && matches!(
                            item,
                            TranscriptItem::Notice(text) if text == LIVE_TRANSCRIPT_RETENTION_NOTICE
                        ))
            })
            .map(|(idx, _)| idx)
            .take(remove_count)
            .collect::<Vec<_>>();
        if remove_indices.len() < remove_count {
            let missing = remove_count - remove_indices.len();
            let existing = remove_indices.clone();
            let additional = self
                .transcript
                .iter()
                .enumerate()
                .filter(|(idx, _)| existing.binary_search(idx).is_err())
                .map(|(idx, _)| idx)
                .take(missing)
                .collect::<Vec<_>>();
            remove_indices.extend(additional);
            remove_indices.sort_unstable();
        }
        if remove_indices.is_empty() {
            return;
        }
        self.selected_tool_index = self.selected_tool_index.and_then(|selected| {
            if remove_indices.binary_search(&selected).is_ok() {
                None
            } else {
                Some(selected - remove_indices.iter().filter(|idx| **idx < selected).count())
            }
        });
        self.selected_backtrack_index = self.selected_backtrack_index.and_then(|selected| {
            if remove_indices.binary_search(&selected).is_ok() {
                None
            } else {
                Some(selected - remove_indices.iter().filter(|idx| **idx < selected).count())
            }
        });
        for idx in remove_indices.into_iter().rev() {
            self.transcript.remove(idx);
        }
        if !has_notice {
            self.transcript.insert(
                0,
                TranscriptItem::Notice(LIVE_TRANSCRIPT_RETENTION_NOTICE.to_string()),
            );
            if let Some(selected) = self.selected_tool_index.as_mut() {
                *selected += 1;
            }
        }
    }

    pub(super) fn finalized_assistant_message(
        &self,
        message_id: Option<&str>,
    ) -> Option<&TranscriptMessage> {
        let normalized_id = message_id.filter(|value| !value.is_empty());
        if let Some(id) = normalized_id {
            return self
                .transcript_state
                .messages()
                .iter()
                .rev()
                .find(|message| {
                    message.role == TranscriptRole::Assistant
                        && message.message_id.as_deref() == Some(id)
                        && message.stream_finished
                });
        }
        self.transcript_state
            .messages()
            .iter()
            .rev()
            .find(|message| message.role == TranscriptRole::Assistant && message.stream_finished)
    }

    pub(super) fn run_stream_commit_tick(&mut self) {
        self.tick_working_indicator();
        let output = run_commit_tick(
            &mut self.stream_chunking_policy,
            Some(&mut self.stream_controller),
            self.plan_stream_controller.as_mut(),
            CommitTickScope::AnyMode,
            Instant::now(),
        );
        if output.cells.is_empty() {
            return;
        }
        if self.native_scrollback {
            for cell in output.cells {
                self.history.enqueue_cell(cell);
            }
            self.sync_assistant_stream_tail_item();
            self.sync_plan_stream_tail_item();
        } else {
            self.sync_assistant_stream_item();
            self.sync_plan_stream_item();
        }
    }

    pub(super) fn finalize_assistant_stream(&mut self) -> Option<String> {
        let final_content = self.stream_controller.finalize();
        if final_content.is_empty() {
            return None;
        }
        if self.native_scrollback {
            self.transcript
                .retain(|item| !matches!(item, TranscriptItem::Assistant(_)));
            if self
                .history
                .remove_non_final_cells(HistoryCellKind::Assistant)
                > 0
            {
                self.resize_reflow.schedule_immediate();
            }
            self.history
                .enqueue(TranscriptItem::Assistant(final_content.clone()));
        } else {
            match self.transcript.last_mut() {
                Some(TranscriptItem::Assistant(value)) => *value = final_content.clone(),
                _ => self
                    .transcript
                    .push(TranscriptItem::Assistant(final_content.clone())),
            }
        }
        Some(final_content)
    }

    pub(super) fn finalize_plan_stream(&mut self) -> Option<String> {
        let source = self
            .plan_stream_controller
            .as_mut()
            .and_then(PlanStreamController::finalize);
        if source.is_some() {
            self.plan_stream_controller = None;
            self.transcript
                .retain(|item| !matches!(item, TranscriptItem::PlanStream(_)));
            if self.native_scrollback
                && self.history.remove_non_final_cells(HistoryCellKind::Plan) > 0
            {
                self.resize_reflow.schedule_immediate();
            }
        }
        source
    }

    pub(super) fn append_reasoning(&mut self, text: &str) {
        self.reasoning_stream_active = true;
        match self.transcript.last_mut() {
            Some(TranscriptItem::Reasoning(value, _)) => value.push_str(text),
            _ => self.push_live_item(TranscriptItem::Reasoning(text.to_string(), true)),
        }
    }
    pub(super) fn add_notice(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.transcript_state.push_notice(text.clone());
        self.push_history_item(TranscriptItem::Notice(text));
    }

    pub(super) fn show_diff_result(&mut self, diff: String) {
        if diff.trim().is_empty() {
            self.add_notice("No git diff for the current project");
        } else {
            self.push_history_item(TranscriptItem::Diff(diff));
        }
    }

    pub(super) fn replace_with_notice(&mut self, text: String) {
        self.cancel_backtrack();
        self.transcript_state.reset();
        self.transcript_state.push_notice(text.clone());
        self.transcript.clear();
        self.history.clear_pending();
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.push_history_item(TranscriptItem::Notice(text));
    }

    pub(super) fn replace_with_session(&mut self, title: String, subtitle: Option<String>) {
        self.cancel_backtrack();
        self.transcript_state.reset();
        self.transcript.clear();
        self.history.clear_pending();
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.push_state_history_item(
            session_header_key(&title, subtitle.as_deref().unwrap_or_default()),
            TranscriptItem::Session { title, subtitle },
        );
    }

    pub(super) fn session_header_title(&self) -> String {
        self.session_title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "New chat".to_string())
    }

    pub(super) fn push_session_header(&mut self) {
        let title = self.session_header_title();
        let subtitle = self.session_header_subtitle();
        self.push_state_history_item(
            session_header_key(&title, &subtitle),
            TranscriptItem::Session {
                title,
                subtitle: Some(subtitle),
            },
        );
    }

    pub(super) fn session_header_item(&self) -> TranscriptItem {
        TranscriptItem::Session {
            title: self.session_header_title(),
            subtitle: Some(self.session_header_subtitle()),
        }
    }

    pub(super) fn rebuild_render_transcript_from_state(&mut self) {
        self.transcript.clear();
        self.history.clear_pending();
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
        if !self.native_scrollback {
            self.rendered_state_keys.clear();
        }
        let messages = self.transcript_state.messages().to_vec();
        for message in &messages {
            self.append_render_message(message);
        }
        if self.native_scrollback {
            self.rendered_state_keys
                .truncate(self.rendered_state_cursor);
        }
        self.sync_backtrack_selection_after_rebuild();
    }

    pub(super) fn append_render_message(&mut self, message: &TranscriptMessage) {
        match &message.role {
            TranscriptRole::User => {
                if !message.content.is_empty() {
                    self.push_state_history_item(
                        render_message_key(message, "user", 0),
                        TranscriptItem::User(message.content.clone()),
                    );
                }
            }
            TranscriptRole::Assistant => {
                let mut part = 0usize;
                let images = image_items(message);
                let content = assistant_content_without_images(&message.content, &images);
                if !message.reasoning.is_empty() {
                    if message.stream_finished {
                        self.push_state_reasoning_item(
                            render_message_key(message, "reasoning", part),
                            message.reasoning.clone(),
                            true,
                            self.reasoning_stream_active,
                        );
                    } else {
                        self.reasoning_stream_active = true;
                        self.push_live_item(TranscriptItem::Reasoning(
                            message.reasoning.clone(),
                            true,
                        ));
                    }
                    part += 1;
                }
                for item in thinking_block_items(message) {
                    self.push_state_history_item(
                        render_message_key(message, "thinking", part),
                        item,
                    );
                    part += 1;
                }
                if !content.is_empty() {
                    if message.stream_finished {
                        self.push_state_history_item(
                            render_message_key(message, "assistant", part),
                            TranscriptItem::Assistant(content.clone()),
                        );
                    } else {
                        self.stream_controller.replace_sanitized_committed(&content);
                        self.transcript.push(TranscriptItem::Assistant(content));
                    }
                }
                part += 1;
                for item in images {
                    self.push_state_history_item(render_message_key(message, "image", part), item);
                    part += 1;
                }
                for citation in &message.citations {
                    self.push_state_history_item(
                        render_message_key(message, "citation", part),
                        citation_item(citation),
                    );
                    part += 1;
                }
                for block in &message.server_content_blocks {
                    self.push_state_history_item(
                        render_message_key(message, "server", part),
                        server_content_block_item(block),
                    );
                    part += 1;
                }
                for tool in &message.tool_calls {
                    self.push_tool_call(tool);
                }
            }
            TranscriptRole::Tool | TranscriptRole::Diff => self.push_state_tool_result(message),
            TranscriptRole::ClientLocalNotice => {
                self.push_state_history_item(
                    render_message_key(message, "notice", 0),
                    TranscriptItem::Notice(message.content.clone()),
                );
            }
            TranscriptRole::System => self.push_state_system_fact(message),
            TranscriptRole::ContextFile
            | TranscriptRole::PlainText
            | TranscriptRole::CdInstruction => {
                self.push_state_info_message(message, message.role.as_str());
            }
            TranscriptRole::CompressionReport => {
                self.push_state_info_message(message, "Compression report");
            }
            TranscriptRole::Error => self.push_state_error_message(message),
            TranscriptRole::Unknown { role, raw } => {
                self.push_state_unknown_message(message, role, raw);
            }
            TranscriptRole::Plan => {
                if self.plan_stream_active() {
                    self.append_plan_stream(
                        render_message_key(message, "plan", 0),
                        &message.content,
                    );
                } else {
                    self.upsert_current_plan_item(render_message_key(message, "plan", 0));
                }
            }
            TranscriptRole::Goal => {
                self.upsert_current_goal_item(render_message_key(message, "goal", 0));
            }
            TranscriptRole::Event => {
                if is_plan_delta_message(message) {
                    if self.plan_stream_active() {
                        self.append_plan_stream(
                            render_message_key(message, "plan_delta", 0),
                            &message.content,
                        );
                    } else {
                        self.upsert_current_plan_item(render_message_key(message, "plan_delta", 0));
                    }
                } else if is_goal_delta_message(message) {
                    self.upsert_current_goal_item(render_message_key(message, "goal_delta", 0));
                } else {
                    self.push_internal_event(message);
                }
            }
        }
    }

    fn plan_stream_active(&self) -> bool {
        self.transcript_state.messages().iter().any(|message| {
            (message.role == TranscriptRole::Plan || is_plan_delta_message(message))
                && !message.stream_finished
        })
    }

    pub(super) fn push_state_info_message(&mut self, message: &TranscriptMessage, label: &str) {
        self.push_state_history_item(
            render_message_key(message, label, 0),
            TranscriptItem::Info(vec![
                label.to_string(),
                visible_message_content(message, "(empty)"),
            ]),
        );
    }

    pub(super) fn push_state_system_fact(&mut self, message: &TranscriptMessage) {
        self.push_state_history_item(
            render_message_key(message, "System", 0),
            TranscriptItem::SystemFact(vec![
                "System".to_string(),
                visible_message_content(message, "(empty)"),
            ]),
        );
    }

    pub(super) fn push_state_error_message(&mut self, message: &TranscriptMessage) {
        self.push_state_history_item(
            render_message_key(message, "error", 0),
            TranscriptItem::Notice(format!(
                "Error: {}",
                visible_message_content(message, "Unknown error")
            )),
        );
    }

    pub(super) fn push_state_unknown_message(
        &mut self,
        message: &TranscriptMessage,
        role: &str,
        raw: &Value,
    ) {
        self.push_state_history_item(
            render_message_key(message, "unknown", 0),
            TranscriptItem::Info(vec![
                format!("Unknown role: {role}"),
                visible_message_content(message, "(empty)"),
                format!("Raw: {}", collapsed_unknown_payload(raw)),
            ]),
        );
    }

    pub(super) fn sync_backtrack_selection_after_rebuild(&mut self) {
        let Some(target) = self.backtrack_target.clone() else {
            return;
        };
        if self
            .transcript_state
            .messages()
            .get(target.index)
            .is_some_and(|message| target.matches(message))
        {
            self.selected_backtrack_index = Some(target.index);
        } else {
            self.clear_backtrack_selection();
        }
    }

    pub(super) fn upsert_current_plan_item(&mut self, key: String) {
        let Some(plan) = current_plan_cell_data(self.transcript_state.messages()) else {
            return;
        };
        self.finalize_plan_stream();
        if !self.record_state_history_key(key) {
            return;
        }
        if let Some(existing) = self.transcript.iter_mut().find_map(|item| match item {
            TranscriptItem::Plan(existing) => Some(existing),
            _ => None,
        }) {
            *existing = plan;
        } else {
            self.push_history_item(TranscriptItem::Plan(plan));
        }
    }

    pub(super) fn upsert_current_goal_item(&mut self, key: String) {
        let Some(goal) = current_goal_cell_data(self.transcript_state.messages()) else {
            return;
        };
        if !self.record_state_history_key(key) {
            return;
        }
        if let Some(existing) = self.transcript.iter_mut().find_map(|item| match item {
            TranscriptItem::Goal(existing) => Some(existing),
            _ => None,
        }) {
            *existing = goal;
        } else {
            self.push_history_item(TranscriptItem::Goal(goal));
        }
    }

    pub(super) fn push_internal_event(&mut self, message: &TranscriptMessage) {
        if !self.record_state_history_key(render_message_key(message, "event", 0)) {
            return;
        }
        let (subkind, source, payload) = event_metadata(message);
        if subkind == "mode_switch" && Self::settings_surface_enabled() {
            self.push_history_item(TranscriptItem::Info(mode_transition_banner(
                &payload,
                &self.mode_records,
            )));
        }
        self.events_pane.push_event(DaemonEventRecord {
            ts_ms: Some(now_ms()),
            kind: format!("chat.{subkind}"),
            project_id: self.current_project_id().map(str::to_string),
            payload: json!({
                "source": source,
                "content": message.content,
                "payload": payload,
            }),
        });
    }
}

impl App {
    pub(super) fn refresh_session_header_item(&mut self) {
        let include_header = self.show_session_header || self.session_title.is_some();
        if !include_header {
            return;
        }
        let title = self.session_header_title();
        let subtitle = self.session_header_subtitle();
        let key = session_header_key(&title, &subtitle);
        self.sync_session_header_render_key(&key);
        let next = TranscriptItem::Session {
            title,
            subtitle: Some(subtitle),
        };
        if let Some(existing) = self
            .transcript
            .iter_mut()
            .find(|item| matches!(item, TranscriptItem::Session { .. }))
        {
            *existing = next;
        } else if self.native_scrollback {
            let changed = self.history.replace_first_item_kind(&next);
            match changed {
                Some(true) => self.resize_reflow.schedule_immediate(),
                None => {
                    self.history.enqueue(next);
                }
                Some(false) => {}
            }
        } else {
            self.transcript.insert(0, next);
        }
    }

    pub(super) fn sync_session_header_render_key(&mut self, key: &str) {
        if let Some(existing) = self
            .rendered_state_keys
            .iter_mut()
            .find(|existing| existing.starts_with("session:header:0:"))
        {
            *existing = key.to_string();
        }
    }

    pub(super) fn rebuild_remote_transcript_from_state(&mut self) {
        let include_header = self.show_session_header || self.session_title.is_some();
        if self.native_scrollback {
            self.rendered_state_keys.clear();
        }
        self.plan_stream_controller = None;
        self.rebuild_render_transcript_from_state();
        if self.native_scrollback && self.history.inserted_cell_count() > 0 {
            self.resize_reflow.schedule_immediate();
        }
        if include_header && !self.native_scrollback {
            self.transcript.insert(0, self.session_header_item());
        }
    }
}

pub(super) fn current_plan_cell_data(messages: &[TranscriptMessage]) -> Option<PlanCellData> {
    let base = current_plan_message(messages)?;
    let deltas = messages
        .iter()
        .filter(|message| is_plan_delta_message(message))
        .map(|message| message.content.clone())
        .collect::<Vec<_>>();
    let content = synthesize_plan_content(&base.content, &deltas);
    let plan_meta = base.extra.get("plan").and_then(Value::as_object);
    let mode = plan_meta
        .and_then(|meta| meta.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("agent")
        .to_string();
    let version = plan_meta
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32;
    Some(PlanCellData::new(content, mode, version, deltas.len()))
}

pub(super) fn current_plan_message(messages: &[TranscriptMessage]) -> Option<&TranscriptMessage> {
    messages
        .iter()
        .filter(|message| message.role == TranscriptRole::Plan)
        .max_by_key(|message| {
            message
                .extra
                .get("plan")
                .and_then(|plan| plan.get("version"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
}

pub(super) fn current_goal_cell_data(messages: &[TranscriptMessage]) -> Option<GoalCellData> {
    let base = current_goal_message(messages)?;
    let deltas = messages
        .iter()
        .filter(|message| is_goal_delta_message(message))
        .map(|message| message.content.clone())
        .collect::<Vec<_>>();
    let content = synthesize_goal_content(&base.content, &deltas);
    let goal_meta = base.extra.get("goal").and_then(Value::as_object);
    let version = goal_meta
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32;
    Some(GoalCellData::new(content, version, deltas.len()))
}

pub(super) fn current_goal_message(messages: &[TranscriptMessage]) -> Option<&TranscriptMessage> {
    messages
        .iter()
        .filter(|message| message.role == TranscriptRole::Goal)
        .max_by_key(|message| {
            message
                .extra
                .get("goal")
                .and_then(|goal| goal.get("version"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
}

pub(super) fn is_plan_delta_message(message: &TranscriptMessage) -> bool {
    event_subkind(message) == Some("plan_delta")
}

pub(super) fn is_goal_delta_message(message: &TranscriptMessage) -> bool {
    event_subkind(message) == Some("goal_delta")
}

pub(super) fn event_metadata(message: &TranscriptMessage) -> (String, String, Value) {
    let event = message.extra.get("event").and_then(Value::as_object);
    let subkind = event
        .and_then(|event| event.get("subkind"))
        .and_then(Value::as_str)
        .unwrap_or("event")
        .to_string();
    let source = event
        .and_then(|event| event.get("source"))
        .and_then(Value::as_str)
        .unwrap_or("chat")
        .to_string();
    let payload = event
        .and_then(|event| event.get("payload"))
        .cloned()
        .unwrap_or(Value::Null);
    (subkind, source, payload)
}

pub(super) fn event_subkind(message: &TranscriptMessage) -> Option<&str> {
    message
        .extra
        .get("event")
        .and_then(|event| event.get("subkind"))
        .and_then(Value::as_str)
}

pub(super) fn mode_transition_banner(
    payload: &Value,
    mode_records: &[crate::pickers::ModePickerItem],
) -> Vec<String> {
    let from = payload
        .get("from")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let to = payload
        .get("to")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .map(sanitize_tool_inline);
    let from_record = mode_records.iter().find(|record| record.item.id == from);
    let to_record = mode_records.iter().find(|record| record.item.id == to);
    let from_title = from_record
        .map(|record| record.item.title.as_str())
        .unwrap_or(from);
    let to_title = to_record
        .map(|record| record.item.title.as_str())
        .unwrap_or(to);
    let mut lines = vec![match reason {
        Some(reason) => format!("Mode transition: {from_title} → {to_title} ({reason})"),
        None => format!("Mode transition: {from_title} → {to_title}"),
    }];
    let known = match (from_record, to_record) {
        (Some(from), Some(to)) => resolved_mode_delta(from, to),
        _ => Vec::new(),
    };
    if known.is_empty() {
        lines.push("Resolved picker records do not show a field-level delta.".to_string());
    } else {
        lines.push(format!("Resolved picker records: {}.", known.join("; ")));
    }
    lines.push(
        "Mode changes may alter the tool allow-list, system prompt, integration/MCP/subagent access, confirmation rules, thread defaults, and goal ownership. This event has no field-level diff, so unreported fields are unknown."
            .to_string(),
    );
    lines
}

fn resolved_mode_delta(
    from: &crate::pickers::ModePickerItem,
    to: &crate::pickers::ModePickerItem,
) -> Vec<String> {
    let mut delta = Vec::new();
    if let (Some(from), Some(to)) = (from.tools_count, to.tools_count) {
        if from != to {
            delta.push(format!("tool count {from} → {to}"));
        }
    }
    append_default_delta(
        &mut delta,
        "project info",
        from.thread_defaults.include_project_info,
        to.thread_defaults.include_project_info,
    );
    append_default_delta(
        &mut delta,
        "checkpoints",
        from.thread_defaults.checkpoints_enabled,
        to.thread_defaults.checkpoints_enabled,
    );
    append_default_delta(
        &mut delta,
        "file edits auto-approved",
        from.thread_defaults.auto_approve_editing_tools,
        to.thread_defaults.auto_approve_editing_tools,
    );
    append_default_delta(
        &mut delta,
        "dangerous commands auto-approved",
        from.thread_defaults.auto_approve_dangerous_commands,
        to.thread_defaults.auto_approve_dangerous_commands,
    );
    delta
}

fn append_default_delta(
    delta: &mut Vec<String>,
    label: &str,
    from: Option<bool>,
    to: Option<bool>,
) {
    if let (Some(from), Some(to)) = (from, to) {
        if from != to {
            delta.push(format!("{label} {} → {}", on_off(from), on_off(to)));
        }
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}
pub(super) fn value_to_compact_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

pub(super) fn visible_message_content(message: &TranscriptMessage, empty: &str) -> String {
    (!message.content.trim().is_empty())
        .then(|| message.content.clone())
        .unwrap_or_else(|| empty.to_string())
}

pub(super) fn collapsed_unknown_payload(raw: &Value) -> String {
    let compact = sanitize_tool_inline(value_to_compact_string(raw));
    truncate_graphemes(&compact, 160).0
}

pub(super) fn line_to_plain_string(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub(super) fn render_message_key(message: &TranscriptMessage, part: &str, index: usize) -> String {
    let id = message
        .message_id
        .as_deref()
        .or(message.tool_call_id.as_deref())
        .unwrap_or_default();
    let revision = render_message_revision(message, part, index);
    if id.is_empty() {
        format!(
            "{}:{}:{}:{:016x}",
            message.role.as_str(),
            part,
            index,
            revision
        )
    } else {
        format!("{}:{}:{}:{:016x}", id, part, index, revision)
    }
}

pub(super) fn state_key_has_stable_identity(key: &str) -> bool {
    let Some((identity, rest)) = key.split_once(':') else {
        return false;
    };
    if rest.split(':').count() != 3 {
        return false;
    }
    !matches!(
        identity,
        "user" | "assistant" | "tool" | "notice" | "plan" | "goal" | "event" | "session"
    )
}

const SESSION_HEADER_TIPS: &str = "Tips: type /help for shortcuts; Ctrl-C twice exits";

pub(super) fn session_header_subtitle(model: Option<&str>, project_root: Option<&Path>) -> String {
    let model = model
        .filter(|value| !value.trim().is_empty())
        .map(sanitize_tool_inline)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let directory = project_root
        .map(|path| sanitize_tool_inline(path.display().to_string()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    format!("model: {model} · /model to change\ndirectory: {directory}\n{SESSION_HEADER_TIPS}")
}

pub(super) fn session_header_key(title: &str, subtitle: &str) -> String {
    format!(
        "session:header:0:{:016x}",
        stable_revision(&(title, subtitle))
    )
}

pub(super) fn render_message_revision(
    message: &TranscriptMessage,
    part: &str,
    index: usize,
) -> u64 {
    match part {
        "user" | "assistant" | "notice" => stable_revision(&(
            message.role.as_str(),
            part,
            &message.content,
            message.stream_finished,
        )),
        "reasoning" => stable_revision(&(
            message.role.as_str(),
            part,
            &message.reasoning,
            message.stream_finished,
        )),
        "thinking" => stable_revision(&(
            message.role.as_str(),
            part,
            indexed_message_value(
                &message.thinking_blocks,
                index.saturating_sub(usize::from(!message.reasoning.is_empty())),
            ),
        )),
        "tool" => stable_revision(&(
            message.role.as_str(),
            part,
            &message.tool_call_id,
            &message.content,
            message.tool_failed,
            message.stream_finished,
        )),
        "image" => stable_revision(&(
            message.role.as_str(),
            part,
            message
                .images
                .get(index.saturating_sub(render_message_side_part_base(message)))
                .map(|image| (&image.mime, &image.data)),
        )),
        "citation" => stable_revision(&(
            message.role.as_str(),
            part,
            indexed_message_value(
                &message.citations,
                index.saturating_sub(render_message_side_part_base(message)),
            ),
        )),
        "server" => stable_revision(&(
            message.role.as_str(),
            part,
            indexed_message_value(
                &message.server_content_blocks,
                index
                    .saturating_sub(render_message_side_part_base(message))
                    .saturating_sub(message.citations.len()),
            ),
        )),
        _ => stable_revision(&(
            message.role.as_str(),
            part,
            &message.content,
            &message.reasoning,
            json_values_revision(&message.tool_calls),
            &message.tool_call_id,
            message.tool_failed,
            json_values_revision(&message.citations),
            json_values_revision(&message.thinking_blocks),
            json_values_revision(&message.server_content_blocks),
            message.stream_finished,
            value_to_compact_string(&Value::Object(message.extra.clone())),
        )),
    }
}

pub(super) fn render_message_side_part_base(message: &TranscriptMessage) -> usize {
    usize::from(!message.reasoning.is_empty()) + message.thinking_blocks.len() + 1
}

pub(super) fn indexed_message_value(values: &[Value], index: usize) -> String {
    values
        .get(index)
        .map(value_to_compact_string)
        .unwrap_or_default()
}

pub(super) fn json_values_revision(values: &[Value]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| format!("{values:?}"))
}

fn stable_revision<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
pub(super) fn rendered_state_keys_for_message(message: &TranscriptMessage) -> Vec<String> {
    match message.role {
        TranscriptRole::User => (!message.content.is_empty())
            .then(|| render_message_key(message, "user", 0))
            .into_iter()
            .collect(),
        TranscriptRole::Assistant => {
            let mut part = 0usize;
            let mut keys = Vec::new();
            let images = image_items(message);
            let content = assistant_content_without_images(&message.content, &images);
            if !message.reasoning.is_empty() && message.stream_finished {
                keys.push(render_message_key(message, "reasoning", part));
            }
            if !message.reasoning.is_empty() {
                part += 1;
            }
            for _ in &message.thinking_blocks {
                keys.push(render_message_key(message, "thinking", part));
                part += 1;
            }
            if message.stream_finished && !content.is_empty() {
                keys.push(render_message_key(message, "assistant", part));
            }
            part += 1;
            for _ in images {
                keys.push(render_message_key(message, "image", part));
                part += 1;
            }
            for _ in &message.citations {
                keys.push(render_message_key(message, "citation", part));
                part += 1;
            }
            for _ in &message.server_content_blocks {
                keys.push(render_message_key(message, "server", part));
                part += 1;
            }
            keys
        }
        TranscriptRole::Tool | TranscriptRole::Diff => vec![render_message_key(message, "tool", 0)],
        TranscriptRole::ClientLocalNotice => vec![render_message_key(message, "notice", 0)],
        TranscriptRole::System => vec![render_message_key(message, "System", 0)],
        TranscriptRole::ContextFile | TranscriptRole::PlainText | TranscriptRole::CdInstruction => {
            vec![render_message_key(message, message.role.as_str(), 0)]
        }
        TranscriptRole::CompressionReport => {
            vec![render_message_key(message, "Compression report", 0)]
        }
        TranscriptRole::Error => vec![render_message_key(message, "error", 0)],
        TranscriptRole::Unknown { .. } => vec![render_message_key(message, "unknown", 0)],
        TranscriptRole::Plan => vec![render_message_key(message, "plan", 0)],
        TranscriptRole::Goal => vec![render_message_key(message, "goal", 0)],
        TranscriptRole::Event => vec![render_message_key(
            message,
            if is_plan_delta_message(message) {
                "plan_delta"
            } else if is_goal_delta_message(message) {
                "goal_delta"
            } else {
                "event"
            },
            0,
        )],
    }
}

pub(super) fn finalized_assistant_content_part(message: &TranscriptMessage) -> usize {
    usize::from(!message.reasoning.is_empty()) + message.thinking_blocks.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_state_keys_track_visible_assistant_parts() {
        let message = TranscriptMessage::from_wire(&json!({
            "message_id": "assistant-1",
            "role": "assistant",
            "reasoning_content": "think",
            "content": "answer",
            "citations": [{"url": "https://example.com"}],
            "server_content_blocks": [{"type": "notice"}],
            "stream_finished": true,
        }));

        let keys = rendered_state_keys_for_message(&message);

        assert_eq!(keys.len(), 4);
        assert!(keys.iter().all(|key| key.starts_with("assistant-1:")));
    }

    #[test]
    fn inline_images_leave_textual_fallback_in_the_ratatui_cell() {
        let message = TranscriptMessage::from_wire(&json!({
            "role": "assistant",
            "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJDRA=="}}],
            "stream_finished": true,
        }));
        let images = image_items(&message);
        let lines = crate::history::render_transcript_item_lines(&images[0], 80, false);
        let raw = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(raw.contains("[image: image/png, 4 bytes]"));
        assert!(!raw.contains('\x1b'));
        assert!(inline_images_for_visible_transcript(&images, None, &[]).is_empty());
    }

    #[test]
    fn inline_images_keep_one_terminal_write_record_per_image() {
        let item = TranscriptItem::Image {
            placeholder: "[image: image/png, 4 bytes]".to_string(),
            data: b"ABCD".to_vec(),
            mime: "image/png".to_string(),
        };
        let images = inline_images_for_visible_transcript(
            &[item],
            Some(crate::terminal_probe::ImageProtocol::Kitty),
            &[ratatui::layout::Position { x: 4, y: 2 }],
        );

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].data, b"ABCD");
        assert_eq!(images[0].position, ratatui::layout::Position { x: 4, y: 2 });
    }

    #[test]
    fn image_render_keys_follow_content_before_citations() {
        let message = TranscriptMessage::from_wire(&json!({
            "message_id": "assistant-1",
            "role": "assistant",
            "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJDRA=="}}],
            "citations": [{"url": "https://example.com"}],
            "stream_finished": true,
        }));

        let keys = rendered_state_keys_for_message(&message);

        assert_eq!(keys.len(), 2);
        assert!(keys[0].contains(":image:"));
        assert!(keys[1].contains(":citation:"));
    }

    #[test]
    fn image_placeholder_moves_to_its_own_cell_without_duplication() {
        let message = TranscriptMessage::from_wire(&json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "before"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJDRA=="}},
                {"type": "text", "text": "after"}
            ],
            "stream_finished": true,
        }));
        let images = image_items(&message);

        assert_eq!(
            assistant_content_without_images(&message.content, &images),
            "before\nafter"
        );
    }

    #[test]
    fn fallback_action_targets_the_latest_transcript_image() {
        let mut app = App::notice_only("test");
        app.test_set_history_items(vec![
            TranscriptItem::Image {
                placeholder: "[image: image/png, 3 bytes]".to_string(),
                data: b"one".to_vec(),
                mime: "image/png".to_string(),
            },
            TranscriptItem::Image {
                placeholder: "[image: image/jpeg, 3 bytes]".to_string(),
                data: b"two".to_vec(),
                mime: "image/jpeg".to_string(),
            },
        ]);

        let AppAction::OpenImageExternally { image } = app.open_latest_image_action() else {
            panic!("latest image action was not produced");
        };
        assert_eq!(image.data, b"two");
        assert_eq!(image.mime, "image/jpeg");
    }

    #[test]
    fn fallback_action_reaches_images_in_native_scrollback() {
        let mut app = App::notice_only("test");
        app.set_native_scrollback(true);
        app.test_push_history_item(TranscriptItem::Image {
            placeholder: "[image: image/png, 3 bytes]".to_string(),
            data: b"png".to_vec(),
            mime: "image/png".to_string(),
        });

        let AppAction::OpenImageExternally { image } = app.open_latest_image_action() else {
            panic!("native scrollback image action was not produced");
        };
        assert_eq!(image.data, b"png");
    }

    #[test]
    fn thinking_blocks_add_individual_render_keys() {
        let message = TranscriptMessage::from_wire(&json!({
            "message_id": "assistant-1",
            "role": "assistant",
            "content": "answer",
            "thinking_blocks": [{"type": "thinking", "thinking": "private reasoning"}],
            "stream_finished": true,
        }));

        let keys = rendered_state_keys_for_message(&message);

        assert_eq!(keys.len(), 2);
        assert!(keys[0].contains(":thinking:"));
    }

    #[test]
    fn non_stable_render_keys_are_not_replay_identities() {
        assert!(!state_key_has_stable_identity(
            "assistant:assistant:0:0000000000000000"
        ));
        assert!(state_key_has_stable_identity(
            "message-1:assistant:0:0000000000000000"
        ));
    }

    #[test]
    fn system_messages_use_the_system_fact_channel() {
        let message = TranscriptMessage::from_wire(&json!({
            "message_id": "system-1",
            "role": "system",
            "content": "Server-maintained fact",
            "stream_finished": true,
        }));
        let mut app = App::notice_only("test");

        app.append_render_message(&message);

        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::SystemFact(lines))
                if lines == &["System".to_string(), "Server-maintained fact".to_string()]
        ));
    }

    #[test]
    fn mode_transition_banner_states_unknown_fields_without_a_diff() {
        let banner = mode_transition_banner(&json!({"from": "ask", "to": "agent"}), &[]);

        assert_eq!(banner[0], "Mode transition: ask → agent");
        assert_eq!(
            banner[1],
            "Resolved picker records do not show a field-level delta."
        );
        assert!(banner[2].contains("has no field-level diff"));
        assert!(banner[2].contains("unknown"));
        assert!(!banner[2].contains("changed"));
    }

    #[test]
    fn mode_transition_banner_reports_only_resolved_picker_deltas() {
        let records = vec![
            crate::pickers::ModePickerItem {
                item: crate::pickers::PickerItem {
                    id: "ask".to_string(),
                    title: "Ask".to_string(),
                    description: String::new(),
                },
                group: "Chat".to_string(),
                tags: vec!["chat".to_string()],
                order: 1,
                is_overlay: false,
                is_current: false,
                tools_count: Some(1),
                thread_defaults: crate::pickers::ModeThreadDefaults {
                    auto_approve_editing_tools: Some(false),
                    ..Default::default()
                },
            },
            crate::pickers::ModePickerItem {
                item: crate::pickers::PickerItem {
                    id: "agent".to_string(),
                    title: "Agent".to_string(),
                    description: String::new(),
                },
                group: "Tools".to_string(),
                tags: vec!["tools".to_string()],
                order: 2,
                is_overlay: false,
                is_current: false,
                tools_count: Some(8),
                thread_defaults: crate::pickers::ModeThreadDefaults {
                    auto_approve_editing_tools: Some(true),
                    ..Default::default()
                },
            },
        ];

        let banner = mode_transition_banner(
            &json!({"from": "ask", "to": "agent", "reason": "handoff"}),
            &records,
        );

        assert_eq!(banner[0], "Mode transition: Ask → Agent (handoff)");
        assert!(banner[1].contains("tool count 1 → 8"));
        assert!(banner[1].contains("file edits auto-approved off → on"));
        assert!(banner[2].contains("has no field-level diff"));
    }
}
