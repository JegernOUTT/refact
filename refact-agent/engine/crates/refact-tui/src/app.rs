use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::approvals::{ApprovalKeyAction, ApprovalModalState};
use crate::client::{
    ChatEvent, ChatSeqDecision, ChatSeqTracker, DaemonClient, OpenProjectResponse, ProjectEntry,
    ToolDecision, WorkerInfo,
};
use crate::events_pane::{DaemonEventRecord, EventsPaneState};
use crate::pickers::{model_items_from_caps, mode_items_from_response, PickerKind, PickerState};
use crate::terminal::TerminalSession;
use crate::tools::{now_ms, ToolCard, ToolStatus};
use crate::vendored::markdown_stream::MarkdownStreamCollector;

const PATCH_LIKE_FUNCTIONS: &[&str] = &[
    "patch",
    "text_edit",
    "create_textdoc",
    "update_textdoc",
    "replace_textdoc",
    "update_textdoc_regex",
    "update_textdoc_by_lines",
    "update_textdoc_anchored",
    "apply_patch",
    "undo_textdoc",
];
const CHAT_RECONNECT_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("daemon client error: {0}")]
    Client(#[from] crate::client::ClientError),
    #[error("terminal error: {0}")]
    Terminal(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiOptions {
    pub daemon_url: Option<String>,
    pub project_hint: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptItem {
    User(String),
    Assistant(String),
    Reasoning(String, bool),
    Tool(ToolCard),
    Notice(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Generating,
    ExecutingTools,
    Paused,
    Error,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Idle => "idle",
            SessionState::Generating => "generating",
            SessionState::ExecutingTools => "tools",
            SessionState::Paused => "paused",
            SessionState::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    Chat,
    ProjectPicker,
}

#[derive(Debug, Clone)]
pub struct ProjectPickerState {
    projects: Vec<ProjectEntry>,
    pub filter: String,
    pub selected: usize,
}

impl ProjectPickerState {
    fn new(projects: Vec<ProjectEntry>) -> Self {
        Self {
            projects,
            filter: String::new(),
            selected: 0,
        }
    }

    pub fn filtered_projects(&self) -> Vec<ProjectEntry> {
        if self.filter.trim().is_empty() {
            return self.projects.clone();
        }
        let needle = self.filter.to_ascii_lowercase();
        self.projects
            .iter()
            .filter(|project| {
                project.slug.to_ascii_lowercase().contains(&needle)
                    || project
                        .root
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .contains(&needle)
            })
            .cloned()
            .collect()
    }

    fn selected_project(&self) -> Option<ProjectEntry> {
        self.filtered_projects().get(self.selected).cloned()
    }

    fn clamp_selection(&mut self) {
        let len = self.filtered_projects().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct App {
    transcript: Vec<TranscriptItem>,
    composer: String,
    composer_mode: ComposerMode,
    picker: ProjectPickerState,
    modal_picker: Option<PickerState>,
    approval_modal: Option<ApprovalModalState>,
    events_pane: EventsPaneState,
    current_project: Option<OpenProjectResponse>,
    chat_id: String,
    model: Option<String>,
    mode: Option<String>,
    pending_model: Option<String>,
    pending_mode: Option<String>,
    session_state: SessionState,
    daemon_online: bool,
    scroll_offset: usize,
    selected_tool_index: Option<usize>,
    help_open: bool,
    should_quit: bool,
    last_ctrl_c: Option<Instant>,
    stream_collector: MarkdownStreamCollector,
}

impl App {
    pub fn new(project: OpenProjectResponse) -> Self {
        Self {
            transcript: vec![TranscriptItem::Notice(format!(
                "Opened project {} at {}",
                project.slug,
                project.root.display()
            ))],
            composer: String::new(),
            composer_mode: ComposerMode::Chat,
            picker: ProjectPickerState::new(Vec::new()),
            modal_picker: None,
            approval_modal: None,
            events_pane: EventsPaneState::new(),
            current_project: Some(project),
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            mode: None,
            pending_model: None,
            pending_mode: None,
            session_state: SessionState::Idle,
            daemon_online: true,
            scroll_offset: 0,
            selected_tool_index: None,
            help_open: false,
            should_quit: false,
            last_ctrl_c: None,
            stream_collector: MarkdownStreamCollector::new(None, std::path::Path::new(".")),
        }
    }

    fn notice_only(notice: impl Into<String>) -> Self {
        Self {
            transcript: vec![TranscriptItem::Notice(notice.into())],
            composer: String::new(),
            composer_mode: ComposerMode::Chat,
            picker: ProjectPickerState::new(Vec::new()),
            modal_picker: None,
            approval_modal: None,
            events_pane: EventsPaneState::new(),
            current_project: None,
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            mode: None,
            pending_model: None,
            pending_mode: None,
            session_state: SessionState::Error,
            daemon_online: false,
            scroll_offset: 0,
            selected_tool_index: None,
            help_open: false,
            should_quit: false,
            last_ctrl_c: None,
            stream_collector: MarkdownStreamCollector::new(None, std::path::Path::new(".")),
        }
    }

    pub fn composer(&self) -> &str {
        &self.composer
    }

    pub fn composer_mode(&self) -> ComposerMode {
        self.composer_mode
    }

    pub fn current_project(&self) -> Option<&OpenProjectResponse> {
        self.current_project.as_ref()
    }

    pub fn current_project_id(&self) -> Option<&str> {
        self.current_project
            .as_ref()
            .map(|project| project.project_id.as_str())
    }

    pub fn current_worker(&self) -> Option<&WorkerInfo> {
        self.current_project
            .as_ref()
            .and_then(|project| project.worker.as_ref())
    }

    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    pub fn session_state(&self) -> SessionState {
        self.session_state
    }

    pub fn daemon_online(&self) -> bool {
        self.daemon_online
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn visible_transcript(&self) -> &[TranscriptItem] {
        &self.transcript
    }

    pub fn project_picker(&self) -> &ProjectPickerState {
        &self.picker
    }

    pub fn modal_picker(&self) -> Option<&PickerState> {
        self.modal_picker.as_ref()
    }

    pub fn approval_modal(&self) -> Option<&ApprovalModalState> {
        self.approval_modal.as_ref()
    }

    pub fn events_pane(&self) -> &EventsPaneState {
        &self.events_pane
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    pub fn selected_tool_index(&self) -> Option<usize> {
        self.selected_tool_index
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn composer_height(&self, width: u16) -> u16 {
        let width = width.saturating_sub(2).max(1) as usize;
        let lines = self
            .composer
            .lines()
            .map(|line| (unicode_width::UnicodeWidthStr::width(line) / width) + 1)
            .sum::<usize>()
            .max(1)
            .min(5);
        lines as u16 + 2
    }

    fn open_project_picker(&mut self, projects: Vec<ProjectEntry>) {
        self.picker = ProjectPickerState::new(projects);
        self.composer_mode = ComposerMode::ProjectPicker;
        self.modal_picker = None;
    }

    fn open_model_picker(&mut self, caps: Value) {
        let items = model_items_from_caps(&caps);
        if items.is_empty() {
            self.add_notice("No models returned by caps");
        } else {
            self.modal_picker = Some(PickerState::new(PickerKind::Model, items));
            self.composer_mode = ComposerMode::Chat;
        }
    }

    fn open_mode_picker(&mut self, modes: Value) {
        let items = mode_items_from_response(&modes);
        if items.is_empty() {
            self.add_notice("No modes returned by worker");
        } else {
            self.modal_picker = Some(PickerState::new(PickerKind::Mode, items));
            self.composer_mode = ComposerMode::Chat;
        }
    }

    fn set_project(&mut self, project: OpenProjectResponse) {
        self.current_project = Some(project.clone());
        self.chat_id = uuid::Uuid::new_v4().to_string();
        self.session_state = SessionState::Idle;
        self.transcript.clear();
        self.transcript.push(TranscriptItem::Notice(format!(
            "Switched to project {} at {}",
            project.slug,
            project.root.display()
        )));
        self.stream_collector.clear();
        self.composer_mode = ComposerMode::Chat;
        self.modal_picker = None;
        self.approval_modal = None;
        self.selected_tool_index = None;
    }

    fn new_chat(&mut self) {
        self.chat_id = uuid::Uuid::new_v4().to_string();
        self.transcript.clear();
        self.transcript
            .push(TranscriptItem::Notice("New chat started".to_string()));
        self.session_state = SessionState::Idle;
        self.stream_collector.clear();
        self.approval_modal = None;
        self.selected_tool_index = None;
    }

    fn submit_composer(&mut self) -> Option<(String, Value)> {
        let prompt = self.composer.trim().to_string();
        if prompt.is_empty() || self.current_project.is_none() {
            return None;
        }
        self.composer.clear();
        self.transcript.push(TranscriptItem::User(prompt.clone()));
        self.transcript
            .push(TranscriptItem::Assistant(String::new()));
        self.session_state = SessionState::Generating;
        self.stream_collector.clear();
        let params = self.take_pending_params();
        Some((prompt, params))
    }

    fn take_pending_params(&mut self) -> Value {
        let mut patch = Map::new();
        if let Some(model) = self.pending_model.take() {
            patch.insert("model".to_string(), Value::String(model.clone()));
            self.model = Some(model);
        }
        if let Some(mode) = self.pending_mode.take() {
            patch.insert("mode".to_string(), Value::String(mode.clone()));
            patch.insert("tool_use".to_string(), Value::String(mode.clone()));
            self.mode = Some(mode);
        }
        if self.mode.is_none() {
            patch.insert("mode".to_string(), Value::String("agent".to_string()));
            patch.insert("tool_use".to_string(), Value::String("agent".to_string()));
            self.mode = Some("agent".to_string());
        }
        Value::Object(patch)
    }

    fn append_assistant(&mut self, text: &str) {
        if !matches!(self.transcript.last(), Some(TranscriptItem::Assistant(_))) {
            self.transcript
                .push(TranscriptItem::Assistant(String::new()));
        }
        if let Some(TranscriptItem::Assistant(value)) = self.transcript.last_mut() {
            value.push_str(text);
        }
        self.stream_collector.push_delta(text);
        let _ = self.stream_collector.commit_complete_source();
    }

    fn append_reasoning(&mut self, text: &str) {
        match self.transcript.last_mut() {
            Some(TranscriptItem::Reasoning(value, _)) => value.push_str(text),
            _ => self
                .transcript
                .push(TranscriptItem::Reasoning(text.to_string(), true)),
        }
    }

    fn add_notice(&mut self, text: impl Into<String>) {
        self.transcript.push(TranscriptItem::Notice(text.into()));
    }

    fn handle_chat_event(&mut self, event: ChatEvent) {
        if event
            .chat_id
            .as_deref()
            .is_some_and(|chat_id| chat_id != self.chat_id)
        {
            return;
        }
        self.daemon_online = true;
        match event.kind.as_str() {
            "snapshot" => self.handle_snapshot(&event.raw),
            "stream_started" => {
                self.session_state = SessionState::Generating;
                if !matches!(self.transcript.last(), Some(TranscriptItem::Assistant(_))) {
                    self.transcript
                        .push(TranscriptItem::Assistant(String::new()));
                }
            }
            "stream_delta" => self.handle_stream_delta(&event.raw),
            "stream_finished" => {
                let tail = self.stream_collector.finalize_and_drain_source();
                let _ = tail;
                if self.session_state != SessionState::Paused {
                    self.session_state = SessionState::Idle;
                }
            }
            "runtime_updated" => self.handle_runtime_updated(&event.raw),
            "pause_required" => self.handle_pause_required(&event.raw),
            "pause_cleared" => self.approval_modal = None,
            "message_added" => self.handle_message_added(&event.raw),
            _ => {}
        }
    }

    fn handle_snapshot(&mut self, raw: &Value) {
        if let Some(thread) = raw.get("thread") {
            self.model = thread
                .get("model")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            self.mode = thread
                .get("mode")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        if let Some(messages) = raw.get("messages").and_then(Value::as_array) {
            self.rebuild_transcript(messages);
        }
        if let Some(runtime) = raw.get("runtime") {
            self.apply_runtime_state(runtime);
            if let Some(modal) = ApprovalModalState::from_event(runtime) {
                self.approval_modal = Some(modal);
            }
        }
    }

    fn rebuild_transcript(&mut self, messages: &[Value]) {
        self.transcript.clear();
        self.selected_tool_index = None;
        for message in messages {
            match message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "user" => {
                    if let Some(text) = content_text(message) {
                        self.transcript.push(TranscriptItem::User(text));
                    }
                }
                "assistant" => {
                    if let Some(text) = content_text(message) {
                        if !text.is_empty() {
                            self.transcript.push(TranscriptItem::Assistant(text));
                        }
                    }
                    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                        for call in calls {
                            self.push_tool_call(call);
                        }
                    }
                }
                "tool" => self.handle_tool_message(message),
                _ => {}
            }
        }
    }

    fn handle_runtime_updated(&mut self, raw: &Value) {
        self.apply_runtime_state(raw);
    }

    fn apply_runtime_state(&mut self, raw: &Value) {
        if raw
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|error| !error.is_empty())
        {
            self.session_state = SessionState::Error;
            return;
        }
        self.session_state = match raw.get("state").and_then(Value::as_str).unwrap_or_default() {
            "generating" => SessionState::Generating,
            "executing_tools" => SessionState::ExecutingTools,
            "paused" => SessionState::Paused,
            "error" => SessionState::Error,
            _ => SessionState::Idle,
        };
    }

    fn handle_pause_required(&mut self, raw: &Value) {
        self.session_state = SessionState::Paused;
        self.approval_modal = ApprovalModalState::from_event(raw);
        if self.approval_modal.is_none() {
            self.add_notice("Approval required but no tool metadata was provided");
        }
    }

    fn handle_stream_delta(&mut self, raw: &Value) {
        let Some(ops) = raw.get("ops").and_then(Value::as_array) else {
            return;
        };
        for op in ops {
            match op.get("op").and_then(Value::as_str).unwrap_or_default() {
                "append_content" => {
                    if let Some(text) = op.get("text").and_then(Value::as_str) {
                        self.append_assistant(text);
                    }
                }
                "append_reasoning" => {
                    if let Some(text) = op.get("text").and_then(Value::as_str) {
                        self.append_reasoning(text);
                    }
                }
                "set_tool_calls" => {
                    for tool in op
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        self.push_tool_call(tool);
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_message_added(&mut self, raw: &Value) {
        let Some(message) = raw.get("message").or_else(|| raw.get("msg")) else {
            return;
        };
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "tool" => self.handle_tool_message(message),
            "assistant" => {
                if let Some(text) = content_text(message) {
                    self.transcript.push(TranscriptItem::Assistant(text));
                }
            }
            "user" => {
                if let Some(text) = content_text(message) {
                    self.transcript.push(TranscriptItem::User(text));
                }
            }
            _ => {}
        }
    }

    fn handle_tool_message(&mut self, message: &Value) {
        let id = message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let result = content_text(message).unwrap_or_default();
        let failed = message
            .get("tool_failed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let completed_at_ms = now_ms();
        self.complete_tool(
            &id,
            result,
            if failed {
                ToolStatus::Error
            } else {
                ToolStatus::Success
            },
            completed_at_ms,
        );
    }

    fn push_tool_call(&mut self, tool: &Value) {
        let card = ToolCard::from_tool_call(tool);
        self.transcript.push(TranscriptItem::Tool(card));
        self.selected_tool_index = Some(self.transcript.len() - 1);
    }

    fn complete_tool(
        &mut self,
        id: &str,
        result: String,
        status: ToolStatus,
        completed_at_ms: u64,
    ) {
        for (idx, item) in self.transcript.iter_mut().enumerate().rev() {
            if let TranscriptItem::Tool(card) = item {
                if card.id == id || id.is_empty() {
                    card.result = result;
                    card.status = status;
                    card.duration_ms = Some(completed_at_ms.saturating_sub(card.started_at_ms));
                    self.selected_tool_index = Some(idx);
                    return;
                }
            }
        }
        let mut card = ToolCard::from_tool_call(&json!({"id": id, "name": "tool"}));
        card.result = result;
        card.status = status;
        card.duration_ms = Some(0);
        self.transcript.push(TranscriptItem::Tool(card));
        self.selected_tool_index = Some(self.transcript.len() - 1);
    }

    fn toggle_selected_tool(&mut self) -> bool {
        let Some(index) = self.selected_tool_index else {
            return false;
        };
        if let Some(TranscriptItem::Tool(card)) = self.transcript.get_mut(index) {
            card.toggle();
            true
        } else {
            false
        }
    }

    fn cycle_tool_selection(&mut self) {
        let indices = self
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| matches!(item, TranscriptItem::Tool(_)).then_some(idx))
            .collect::<Vec<_>>();
        if indices.is_empty() {
            self.selected_tool_index = None;
            return;
        }
        let next = match self.selected_tool_index {
            Some(current) => indices
                .iter()
                .position(|idx| *idx == current)
                .map(|pos| indices[(pos + 1) % indices.len()])
                .unwrap_or(indices[0]),
            None => indices[0],
        };
        self.selected_tool_index = Some(next);
    }

    fn set_workers(&mut self, workers: Vec<WorkerInfo>) {
        self.events_pane.set_workers(workers);
    }

    fn push_daemon_event(&mut self, event: DaemonEventRecord) {
        self.events_pane.push_event(event);
    }

    fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind != KeyEventKind::Press {
            return AppAction::None;
        }
        if self.help_open {
            self.help_open = false;
            return AppAction::None;
        }
        if let Some(action) = self.handle_approval_key(key) {
            return action;
        }
        if self.modal_picker.is_some() {
            return self.handle_modal_picker_key(key);
        }
        if self.composer_mode == ComposerMode::ProjectPicker {
            return self.handle_project_picker_key(key);
        }
        match key {
            KeyEvent {
                code: KeyCode::Char('?'),
                ..
            } => {
                self.help_open = true;
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::F(2),
                ..
            } => {
                self.events_pane.toggle();
                AppAction::RefreshWorkers
            }
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.should_quit = true;
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.new_chat();
                AppAction::SubscribeCurrent
            }
            KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => AppAction::LoadProjects,
            KeyEvent {
                code: KeyCode::Char('m'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => AppAction::LoadModels,
            KeyEvent {
                code: KeyCode::Char('o'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => AppAction::LoadModes,
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                for item in &mut self.transcript {
                    if let TranscriptItem::Reasoning(_, collapsed) = item {
                        *collapsed = !*collapsed;
                    }
                }
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.ctrl_c_action(),
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                if self.events_pane.open {
                    self.events_pane.open = false;
                    AppAction::None
                } else if matches!(
                    self.session_state,
                    SessionState::Generating | SessionState::ExecutingTools | SessionState::Paused
                ) {
                    self.session_state = SessionState::Idle;
                    self.add_notice("Cancel requested");
                    AppAction::Abort
                } else {
                    AppAction::None
                }
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                self.cycle_tool_selection();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers,
                ..
            } if self.composer.is_empty()
                && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
            {
                self.toggle_selected_tool();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::SHIFT,
                ..
            } => {
                self.composer.push('\n');
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } if self.composer.is_empty() && self.toggle_selected_tool() => AppAction::None,
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self
                .submit_composer()
                .map(|(prompt, params)| AppAction::SendMessage { prompt, params })
                .unwrap_or(AppAction::None),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.composer.pop();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => {
                self.scroll_offset = self.scroll_offset.saturating_add(5);
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => {
                self.scroll_offset = self.scroll_offset.saturating_sub(5);
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                self.composer.push(ch);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_approval_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let modal = self.approval_modal.as_mut()?;
        match modal.handle_key(key) {
            ApprovalKeyAction::None | ApprovalKeyAction::ToggleFullArgs => Some(AppAction::None),
            ApprovalKeyAction::ApproveOnce => {
                let decisions = modal.decisions(true);
                self.approval_modal = None;
                Some(AppAction::SendToolDecisions {
                    decisions,
                    patch: None,
                })
            }
            ApprovalKeyAction::ApproveForChat => {
                let patch = approval_patch(modal);
                let decisions = modal.decisions(true);
                self.approval_modal = None;
                Some(AppAction::SendToolDecisions {
                    decisions,
                    patch: Some(patch),
                })
            }
            ApprovalKeyAction::Deny => {
                let decisions = modal.decisions(false);
                self.approval_modal = None;
                Some(AppAction::SendToolDecisions {
                    decisions,
                    patch: None,
                })
            }
        }
    }

    fn ctrl_c_action(&mut self) -> AppAction {
        if matches!(
            self.session_state,
            SessionState::Generating | SessionState::ExecutingTools | SessionState::Paused
        ) {
            self.session_state = SessionState::Idle;
            self.approval_modal = None;
            self.add_notice("Cancel requested");
            self.last_ctrl_c = Some(Instant::now());
            return AppAction::Abort;
        }
        let now = Instant::now();
        if self
            .last_ctrl_c
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(900))
        {
            self.should_quit = true;
        } else {
            self.add_notice("Press Ctrl-C again to exit");
            self.last_ctrl_c = Some(now);
        }
        AppAction::None
    }

    fn handle_project_picker_key(&mut self, key: KeyEvent) -> AppAction {
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.composer_mode = ComposerMode::Chat;
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self
                .picker
                .selected_project()
                .map(|project| AppAction::OpenProject(project.root))
                .unwrap_or(AppAction::None),
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.picker.selected = self.picker.selected.saturating_sub(1);
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.picker.selected = self.picker.selected.saturating_add(1);
                self.picker.clamp_selection();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.picker.filter.pop();
                self.picker.clamp_selection();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                self.picker.filter.push(ch);
                self.picker.selected = 0;
                self.picker.clamp_selection();
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_modal_picker_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(picker) = self.modal_picker.as_mut() else {
            return AppAction::None;
        };
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.modal_picker = None;
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if let Some(item) = picker.selected_item() {
                    match picker.kind {
                        PickerKind::Model => {
                            self.pending_model = Some(item.id.clone());
                            self.model = Some(item.id.clone());
                            self.add_notice(format!(
                                "Model selected for next message: {}",
                                item.title
                            ));
                        }
                        PickerKind::Mode => {
                            self.pending_mode = Some(item.id.clone());
                            self.mode = Some(item.id.clone());
                            self.add_notice(format!(
                                "Mode selected for next message: {}",
                                item.title
                            ));
                        }
                    }
                }
                self.modal_picker = None;
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                picker.select_prev();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                picker.select_next();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                picker.pop_filter();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                picker.push_filter(ch);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    #[cfg(test)]
    pub fn test_set_approval(&mut self, modal: ApprovalModalState) {
        self.approval_modal = Some(modal);
        self.session_state = SessionState::Paused;
    }

    #[cfg(test)]
    pub fn test_push_tool(&mut self, card: ToolCard) {
        self.transcript.push(TranscriptItem::Tool(card));
        self.selected_tool_index = Some(self.transcript.len() - 1);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    None,
    LoadProjects,
    LoadModels,
    LoadModes,
    RefreshWorkers,
    OpenProject(PathBuf),
    SubscribeCurrent,
    SendMessage {
        prompt: String,
        params: Value,
    },
    SendToolDecisions {
        decisions: Vec<ToolDecision>,
        patch: Option<Value>,
    },
    Abort,
}

async fn show_startup_notice(message: String) -> Result<(), TuiError> {
    let mut app = App::notice_only(message);
    let mut terminal = TerminalSession::start()?;
    terminal
        .terminal_mut()
        .draw(|frame| crate::ui::render(frame, &app))?;
    let mut reader = EventStream::new();
    let started = Instant::now();
    loop {
        if started.elapsed() >= Duration::from_secs(8) {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(250), reader.next()).await {
            Ok(Some(Ok(Event::Key(key)))) if key.kind == KeyEventKind::Press => {
                app.handle_key(key);
                break;
            }
            Ok(Some(Ok(Event::Resize(_, _)))) => {
                terminal
                    .terminal_mut()
                    .draw(|frame| crate::ui::render(frame, &app))?;
            }
            Ok(Some(Ok(_))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => {}
        }
    }
    Ok(())
}

#[derive(Debug)]
enum RuntimeEvent {
    Input(Event),
    Tick,
    Chat { generation: u64, event: ChatEvent },
    ChatResubscribe { generation: u64, message: String },
    ChatDisconnected { generation: u64, message: String },
    InputError(String),
    DaemonEvent(DaemonEventRecord),
    DaemonEventsDisconnected(String),
    ProjectsLoaded(Result<Vec<ProjectEntry>, String>),
    ProjectOpened(Result<OpenProjectResponse, String>),
    ModelsLoaded(Result<Value, String>),
    ModesLoaded(Result<Value, String>),
    WorkersLoaded(Result<Vec<WorkerInfo>, String>),
    CommandFinished(Result<(), String>),
}

#[derive(Debug, Default)]
struct SubscriptionManager {
    generation: u64,
    current: Option<SubscriptionTask>,
}

#[derive(Debug)]
struct SubscriptionTask {
    generation: u64,
    project_id: String,
    chat_id: String,
    join: JoinHandle<()>,
}

impl SubscriptionManager {
    fn new() -> Self {
        Self::default()
    }

    fn subscribe(
        &mut self,
        client: DaemonClient,
        project_id: String,
        chat_id: String,
        tx: mpsc::Sender<RuntimeEvent>,
    ) {
        self.start(client, project_id, chat_id, tx, Duration::ZERO);
    }

    fn reconnect_current(&mut self, client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>) {
        let Some(current) = &self.current else {
            return;
        };
        self.start(
            client,
            current.project_id.clone(),
            current.chat_id.clone(),
            tx,
            CHAT_RECONNECT_BACKOFF,
        );
    }

    fn abort_current(&mut self) {
        if let Some(current) = self.current.take() {
            current.join.abort();
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.generation == generation)
    }

    fn apply_chat_event(&self, app: &mut App, generation: u64, event: ChatEvent) -> bool {
        if self.is_current(generation) {
            app.handle_chat_event(event);
            true
        } else {
            false
        }
    }

    fn start(
        &mut self,
        client: DaemonClient,
        project_id: String,
        chat_id: String,
        tx: mpsc::Sender<RuntimeEvent>,
        initial_delay: Duration,
    ) {
        self.abort_current();
        self.generation = self.generation.wrapping_add(1).max(1);
        let generation = self.generation;
        let join = spawn_subscription_task(
            client,
            project_id.clone(),
            chat_id.clone(),
            tx,
            generation,
            initial_delay,
        );
        self.current = Some(SubscriptionTask {
            generation,
            project_id,
            chat_id,
            join,
        });
    }
}

impl Drop for SubscriptionManager {
    fn drop(&mut self) {
        self.abort_current();
    }
}

pub async fn run(options: TuiOptions) -> Result<(), TuiError> {
    let endpoint = match crate::client::resolve_daemon_endpoint(
        options
            .daemon_url
            .or_else(|| std::env::var("REFACT_DAEMON_URL").ok()),
    ) {
        Ok(endpoint) => endpoint,
        Err(warning) => {
            let notice = warning.notice();
            let _ = show_startup_notice(notice.clone()).await;
            return Err(TuiError::Message(notice));
        }
    };
    let client = DaemonClient::from_endpoint(endpoint)?;
    let root = match options.project_hint {
        Some(path) => path,
        None => std::env::current_dir().map_err(|error| TuiError::Message(error.to_string()))?,
    };
    let project = client.open_project(&root).await?;
    let mut app = App::new(project);
    let mut terminal = TerminalSession::start()?;
    let (tx, mut rx) = mpsc::channel::<RuntimeEvent>(256);
    let mut subscriptions = SubscriptionManager::new();
    spawn_input_task(tx.clone());
    spawn_tick_task(tx.clone());
    spawn_daemon_events_task(client.clone(), tx.clone());
    spawn_worker_refresh_task(client.clone(), tx.clone());
    if let Some(project_id) = app.current_project_id() {
        subscriptions.subscribe(
            client.clone(),
            project_id.to_string(),
            app.chat_id().to_string(),
            tx.clone(),
        );
    }

    loop {
        terminal
            .terminal_mut()
            .draw(|frame| crate::ui::render(frame, &app))?;
        if app.should_quit() {
            break;
        }
        let Some(event) = rx.recv().await else {
            break;
        };
        match event {
            RuntimeEvent::Input(Event::Key(key)) => {
                let action = app.handle_key(key);
                run_action(&mut app, action, &client, &tx, &mut subscriptions);
            }
            RuntimeEvent::Input(Event::Resize(_, _)) | RuntimeEvent::Tick => {}
            RuntimeEvent::Input(_) => {}
            RuntimeEvent::Chat { generation, event } => {
                subscriptions.apply_chat_event(&mut app, generation, event);
            }
            RuntimeEvent::ChatResubscribe {
                generation,
                message,
            } => {
                if subscriptions.is_current(generation) {
                    app.add_notice(format!("SSE resync: {message}"));
                    subscriptions.reconnect_current(client.clone(), tx.clone());
                }
            }
            RuntimeEvent::ChatDisconnected {
                generation,
                message,
            } => {
                if !subscriptions.is_current(generation) {
                    continue;
                }
                app.daemon_online = false;
                app.add_notice(format!("SSE disconnected: {message}; reconnecting…"));
                subscriptions.reconnect_current(client.clone(), tx.clone());
            }
            RuntimeEvent::InputError(message) => app.add_notice(format!("Input error: {message}")),
            RuntimeEvent::DaemonEvent(event) => app.push_daemon_event(event),
            RuntimeEvent::DaemonEventsDisconnected(message) => {
                app.add_notice(format!(
                    "Daemon events disconnected: {message}; reconnecting…"
                ));
            }
            RuntimeEvent::ProjectsLoaded(Ok(projects)) => app.open_project_picker(projects),
            RuntimeEvent::ProjectsLoaded(Err(error)) => {
                app.add_notice(format!("Failed to list projects: {error}"))
            }
            RuntimeEvent::ProjectOpened(Ok(project)) => {
                app.set_project(project);
                if let Some(project_id) = app.current_project_id() {
                    subscriptions.subscribe(
                        client.clone(),
                        project_id.to_string(),
                        app.chat_id().to_string(),
                        tx.clone(),
                    );
                }
            }
            RuntimeEvent::ProjectOpened(Err(error)) => {
                app.add_notice(format!("Failed to open project: {error}"))
            }
            RuntimeEvent::ModelsLoaded(Ok(caps)) => app.open_model_picker(caps),
            RuntimeEvent::ModelsLoaded(Err(error)) => {
                app.add_notice(format!("Failed to load models: {error}"))
            }
            RuntimeEvent::ModesLoaded(Ok(modes)) => app.open_mode_picker(modes),
            RuntimeEvent::ModesLoaded(Err(error)) => {
                app.add_notice(format!("Failed to load modes: {error}"))
            }
            RuntimeEvent::WorkersLoaded(Ok(workers)) => app.set_workers(workers),
            RuntimeEvent::WorkersLoaded(Err(error)) => {
                if app.events_pane.open {
                    app.add_notice(format!("Failed to refresh workers: {error}"));
                }
            }
            RuntimeEvent::CommandFinished(Ok(())) => {}
            RuntimeEvent::CommandFinished(Err(error)) => {
                app.add_notice(format!("Command failed: {error}"))
            }
        }
    }
    Ok(())
}

fn run_action(
    app: &mut App,
    action: AppAction,
    client: &DaemonClient,
    tx: &mpsc::Sender<RuntimeEvent>,
    subscriptions: &mut SubscriptionManager,
) {
    match action {
        AppAction::None => {}
        AppAction::LoadProjects => {
            let client = client.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = client
                    .list_projects()
                    .await
                    .map_err(|error| error.to_string());
                let _ = tx.send(RuntimeEvent::ProjectsLoaded(result)).await;
            });
        }
        AppAction::LoadModels => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .get_caps(&project_id)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::ModelsLoaded(result)).await;
                });
            }
        }
        AppAction::LoadModes => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .get_chat_modes(&project_id)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::ModesLoaded(result)).await;
                });
            }
        }
        AppAction::RefreshWorkers => refresh_workers(client.clone(), tx.clone()),
        AppAction::OpenProject(root) => {
            let client = client.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = client
                    .open_project(&root)
                    .await
                    .map_err(|error| error.to_string());
                let _ = tx.send(RuntimeEvent::ProjectOpened(result)).await;
            });
        }
        AppAction::SubscribeCurrent => {
            if let Some(project_id) = app.current_project_id() {
                subscriptions.subscribe(
                    client.clone(),
                    project_id.to_string(),
                    app.chat_id().to_string(),
                    tx.clone(),
                );
            }
        }
        AppAction::SendMessage { prompt, params } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = async {
                        if params.as_object().is_some_and(|object| !object.is_empty()) {
                            client
                                .send_set_params(&project_id, &chat_id, params)
                                .await?;
                        }
                        client
                            .send_user_message(&project_id, &chat_id, &prompt)
                            .await
                    }
                    .await
                    .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::CommandFinished(result)).await;
                });
            }
        }
        AppAction::SendToolDecisions { decisions, patch } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = async {
                        if let Some(patch) = patch {
                            client.send_set_params(&project_id, &chat_id, patch).await?;
                        }
                        client
                            .send_tool_decisions(&project_id, &chat_id, decisions)
                            .await
                    }
                    .await
                    .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::CommandFinished(result)).await;
                });
            }
        }
        AppAction::Abort => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .send_abort(&project_id, &chat_id)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::CommandFinished(result)).await;
                });
            }
        }
    }
}

fn spawn_input_task(tx: mpsc::Sender<RuntimeEvent>) {
    tokio::spawn(async move {
        let mut reader = EventStream::new();
        while let Some(event) = reader.next().await {
            match event {
                Ok(event) => {
                    if tx.send(RuntimeEvent::Input(event)).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    if tx
                        .send(RuntimeEvent::InputError(error.to_string()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
}

fn spawn_tick_task(tx: mpsc::Sender<RuntimeEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            if tx.send(RuntimeEvent::Tick).await.is_err() {
                break;
            }
        }
    });
}

fn spawn_worker_refresh_task(client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            refresh_workers(client.clone(), tx.clone());
            if tx.is_closed() {
                break;
            }
        }
    });
}

fn refresh_workers(client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>) {
    tokio::spawn(async move {
        let result = client
            .list_workers()
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(RuntimeEvent::WorkersLoaded(result)).await;
    });
}

fn spawn_daemon_events_task(client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>) {
    tokio::spawn(async move {
        loop {
            match client.subscribe_daemon_events().await {
                Ok(mut stream) => {
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(event) => {
                                if tx.send(RuntimeEvent::DaemonEvent(event)).await.is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                if tx
                                    .send(RuntimeEvent::DaemonEventsDisconnected(error.to_string()))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    if tx
                        .send(RuntimeEvent::DaemonEventsDisconnected(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn spawn_subscription_task(
    client: DaemonClient,
    project_id: String,
    chat_id: String,
    tx: mpsc::Sender<RuntimeEvent>,
    generation: u64,
    initial_delay: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !initial_delay.is_zero() {
            tokio::time::sleep(initial_delay).await;
        }
        match client.subscribe_chat(&project_id, &chat_id).await {
            Ok(mut stream) => {
                let mut seq = ChatSeqTracker::new();
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(event) => match seq.observe(&event) {
                            ChatSeqDecision::Apply => {
                                if tx
                                    .send(RuntimeEvent::Chat { generation, event })
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            ChatSeqDecision::Resubscribe(message) => {
                                let _ = tx
                                    .send(RuntimeEvent::ChatResubscribe {
                                        generation,
                                        message,
                                    })
                                    .await;
                                return;
                            }
                        },
                        Err(error) => {
                            let _ = tx
                                .send(RuntimeEvent::ChatDisconnected {
                                    generation,
                                    message: error.to_string(),
                                })
                                .await;
                            return;
                        }
                    }
                }
                let _ = tx
                    .send(RuntimeEvent::ChatDisconnected {
                        generation,
                        message: "stream ended".to_string(),
                    })
                    .await;
            }
            Err(error) => {
                let _ = tx
                    .send(RuntimeEvent::ChatDisconnected {
                        generation,
                        message: error.to_string(),
                    })
                    .await;
            }
        }
    })
}

fn content_text(message: &Value) -> Option<String> {
    match message.get("content")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        value => Some(value_to_compact_string(value)),
    }
}

fn value_to_compact_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn approval_patch(modal: &ApprovalModalState) -> Value {
    let all_patch_like = modal.reasons().iter().all(|reason| {
        PATCH_LIKE_FUNCTIONS
            .iter()
            .any(|name| *name == reason.tool_name)
    });
    if all_patch_like {
        json!({"auto_approve_editing_tools": true})
    } else {
        json!({"auto_approve_dangerous_commands": true})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::PauseReason;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: false,
            worker: None,
            cron_pending: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn chat_event(app: &App, seq: u64, kind: &str, raw: Value) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: Some(seq),
            kind: kind.to_string(),
            raw,
        }
    }

    fn apply_tracked(
        app: &mut App,
        tracker: &mut ChatSeqTracker,
        event: ChatEvent,
    ) -> ChatSeqDecision {
        let decision = tracker.observe(&event);
        if decision == ChatSeqDecision::Apply {
            app.handle_chat_event(event);
        }
        decision
    }

    #[test]
    fn app_submits_message_and_streams_answer() {
        let mut app = App::new(project());
        app.composer = "hello".to_string();
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "hello"
        ));
        assert_eq!(app.session_state(), SessionState::Generating);
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "append_content", "text": "hi"}]}),
        });
        assert!(
            matches!(app.visible_transcript().last(), Some(TranscriptItem::Assistant(text)) if text == "hi")
        );
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        assert_eq!(app.session_state(), SessionState::Idle);
    }

    #[test]
    fn app_cancel_and_double_ctrl_c_behaviour() {
        let mut app = App::new(project());
        app.session_state = SessionState::Generating;
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            AppAction::Abort
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(!app.should_quit());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(app.should_quit());
    }

    #[test]
    fn project_picker_filters_and_selects() {
        let mut picker = ProjectPickerState::new(vec![
            ProjectEntry {
                id: "a".to_string(),
                slug: "alpha".to_string(),
                root: PathBuf::from("/tmp/alpha"),
                pinned: false,
                last_active_ms: 0,
                settings: Value::Null,
            },
            ProjectEntry {
                id: "b".to_string(),
                slug: "beta".to_string(),
                root: PathBuf::from("/tmp/beta"),
                pinned: false,
                last_active_ms: 0,
                settings: Value::Null,
            },
        ]);
        picker.filter = "bet".to_string();
        assert_eq!(picker.filtered_projects()[0].slug, "beta");
    }

    #[test]
    fn synthetic_pause_approval_yields_decision_and_clears_modal() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "pause_required".to_string(),
            raw: json!({"reasons": [{"type": "confirmation", "tool_name": "shell", "command": "echo hi", "rule": "*", "tool_call_id": "call-1"}]}),
        });
        assert!(app.approval_modal().is_some());
        let action = app.handle_key(key(KeyCode::Char('y')));
        assert!(app.approval_modal().is_none());
        assert_eq!(
            action,
            AppAction::SendToolDecisions {
                decisions: vec![ToolDecision {
                    tool_call_id: "call-1".to_string(),
                    accepted: true,
                }],
                patch: None,
            }
        );
    }

    #[test]
    fn approve_for_chat_sets_patch_scope() {
        let mut app = App::new(project());
        app.test_set_approval(ApprovalModalState::new(vec![PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "update_textdoc".to_string(),
            command: "{}".to_string(),
            rule: "default".to_string(),
            tool_call_id: "call-1".to_string(),
            integr_config_path: None,
        }]));
        let action = app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(
            action,
            AppAction::SendToolDecisions {
                decisions: vec![ToolDecision {
                    tool_call_id: "call-1".to_string(),
                    accepted: true,
                }],
                patch: Some(json!({"auto_approve_editing_tools": true})),
            }
        );
    }

    #[test]
    fn picker_selection_applies_params_to_next_message() {
        let mut app = App::new(project());
        app.open_model_picker(json!({"chat_models": {"m1": {"name": "Model One"}}}));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        app.composer = "hello".to_string();
        let action = app.handle_key(key(KeyCode::Enter));
        match action {
            AppAction::SendMessage { params, .. } => {
                assert_eq!(params["model"], "m1");
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn seq_gap_requests_resubscribe_before_gap_delta_applies() {
        let mut app = App::new(project());
        let mut tracker = ChatSeqTracker::new();
        let snapshot = chat_event(
            &app,
            0,
            "snapshot",
            json!({"thread": {"model": "", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        );
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, snapshot),
            ChatSeqDecision::Apply
        );
        let started = chat_event(&app, 1, "stream_started", json!({}));
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, started),
            ChatSeqDecision::Apply
        );
        let kept = chat_event(
            &app,
            2,
            "stream_delta",
            json!({"ops": [{"op": "append_content", "text": "kept"}]}),
        );
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, kept),
            ChatSeqDecision::Apply
        );
        let dropped = chat_event(
            &app,
            4,
            "stream_delta",
            json!({"ops": [{"op": "append_content", "text": "dropped"}]}),
        );
        let decision = apply_tracked(&mut app, &mut tracker, dropped);
        assert!(matches!(decision, ChatSeqDecision::Resubscribe(_)));
        assert!(
            matches!(app.visible_transcript().last(), Some(TranscriptItem::Assistant(text)) if text == "kept")
        );
    }

    #[test]
    fn duplicate_seq_does_not_duplicate_transcript_content() {
        let mut app = App::new(project());
        let mut tracker = ChatSeqTracker::new();
        let snapshot = chat_event(
            &app,
            0,
            "snapshot",
            json!({"thread": {"model": "", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        );
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, snapshot),
            ChatSeqDecision::Apply
        );
        let started = chat_event(&app, 1, "stream_started", json!({}));
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, started),
            ChatSeqDecision::Apply
        );
        let delta = json!({"ops": [{"op": "append_content", "text": "once"}]});
        let first = chat_event(&app, 2, "stream_delta", delta.clone());
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, first),
            ChatSeqDecision::Apply
        );
        let duplicate = chat_event(&app, 2, "stream_delta", delta);
        let decision = apply_tracked(&mut app, &mut tracker, duplicate);
        assert!(matches!(decision, ChatSeqDecision::Resubscribe(_)));
        assert!(
            matches!(app.visible_transcript().last(), Some(TranscriptItem::Assistant(text)) if text == "once")
        );
    }

    #[tokio::test]
    async fn stale_subscription_generation_is_ignored() {
        let mut app = App::new(project());
        let mut subscriptions = SubscriptionManager::new();
        let join = tokio::spawn(async { futures::future::pending::<()>().await });
        subscriptions.current = Some(SubscriptionTask {
            generation: 2,
            project_id: "p2".to_string(),
            chat_id: app.chat_id().to_string(),
            join,
        });
        let stale = chat_event(
            &app,
            1,
            "stream_delta",
            json!({"ops": [{"op": "append_content", "text": "stale"}]}),
        );
        assert!(!subscriptions.apply_chat_event(&mut app, 1, stale));
        assert!(matches!(
            app.visible_transcript(),
            [TranscriptItem::Notice(_)]
        ));
        let fresh = chat_event(
            &app,
            1,
            "stream_delta",
            json!({"ops": [{"op": "append_content", "text": "fresh"}]}),
        );
        assert!(subscriptions.apply_chat_event(&mut app, 2, fresh));
        assert!(
            matches!(app.visible_transcript().last(), Some(TranscriptItem::Assistant(text)) if text == "fresh")
        );
    }

    #[tokio::test]
    async fn subscription_switch_aborts_prior_task() {
        let mut subscriptions = SubscriptionManager::new();
        subscriptions.generation = 1;
        let join = tokio::spawn(async { futures::future::pending::<()>().await });
        let abort = join.abort_handle();
        subscriptions.current = Some(SubscriptionTask {
            generation: 1,
            project_id: "p1".to_string(),
            chat_id: "c1".to_string(),
            join,
        });
        let client = DaemonClient::new("http://127.0.0.1:1", None).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        subscriptions.subscribe(client, "p2".to_string(), "c2".to_string(), tx);
        for _ in 0..10 {
            if abort.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(abort.is_finished());
        assert!(subscriptions.is_current(2));
        subscriptions.abort_current();
    }
}
