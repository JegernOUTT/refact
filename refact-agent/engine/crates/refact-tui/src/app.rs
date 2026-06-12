use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::approvals::{ApprovalKeyAction, ApprovalModalState, ApprovalQueue};
use crate::client::{
    ChatEvent, ChatSeqDecision, ChatSeqTracker, DaemonClient, OpenProjectResponse, ProjectEntry,
    ToolDecision, WorkerInfo,
};
use crate::commands::{
    command_by_name, command_picker_items, workflow, CommandAction, CommandContext, CommandPicker,
    InfoTopic, LocalToggle,
};
use crate::composer::{load_history, save_history, ComposerState, EnterDecision};
use crate::events_pane::{DaemonEventRecord, EventsPaneState};
use crate::history::cells::ApprovalOutcome;
use crate::history::{insert_history, HistoryBuffer, HistoryInsertion};
use crate::pickers::{
    file_mention_items_from_completions, model_items_from_caps, mode_items_from_response,
    PickerAccept, PickerItem, PickerKind, PickerState,
};
use crate::protocol::{
    content_text, DeltaOp, SseEvent, TranscriptMessage, TranscriptRole, TranscriptState,
};
use crate::streaming::{run_commit_tick, StreamController};
use crate::terminal::TerminalSession;
use crate::tools::{now_ms, ToolCard, ToolStatus};

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
const ABORT_BEFORE_QUIT_TIMEOUT: Duration = Duration::from_secs(1);

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
    Citation(String),
    ServerContentBlock(String),
    Diff(String),
    Notice(String),
    Approval(ApprovalModalState, Option<ApprovalOutcome>),
    Session {
        title: String,
        subtitle: Option<String>,
    },
    Plan(String),
}

impl TranscriptItem {
    fn keeps_live(&self) -> bool {
        matches!(self, Self::Tool(card) if card.status == ToolStatus::Running)
            || matches!(self, Self::Approval(_, None))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Generating,
    ExecutingTools,
    Paused,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Online,
    Waking,
    Offline,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSummary {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl UsageSummary {
    fn from_value(value: &Value) -> Option<Self> {
        let prompt_tokens = token_count(value, &["prompt_tokens", "input_tokens", "prompt"]);
        let completion_tokens =
            token_count(value, &["completion_tokens", "output_tokens", "completion"]);
        let total_tokens = token_count(value, &["total_tokens", "total"])
            .or_else(|| prompt_tokens.zip(completion_tokens).map(|(a, b)| a + b));
        if prompt_tokens.is_none() && completion_tokens.is_none() && total_tokens.is_none() {
            None
        } else {
            Some(Self {
                prompt_tokens: prompt_tokens.unwrap_or_default(),
                completion_tokens: completion_tokens.unwrap_or_default(),
                total_tokens: total_tokens.unwrap_or_default(),
            })
        }
    }

    pub fn display(self) -> String {
        if self.total_tokens > 0 {
            format!("{} tok", self.total_tokens)
        } else {
            format!("{} in · {} out", self.prompt_tokens, self.completion_tokens)
        }
    }

    pub fn tokens_used(self) -> u64 {
        if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.prompt_tokens.saturating_add(self.completion_tokens)
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
    transcript_state: TranscriptState,
    composer: ComposerState,
    history_path: Option<PathBuf>,
    composer_mode: ComposerMode,
    picker: ProjectPickerState,
    modal_picker: Option<PickerState>,
    approval_queue: ApprovalQueue,
    pending_approval_clears: usize,
    events_pane: EventsPaneState,
    current_project: Option<OpenProjectResponse>,
    chat_id: String,
    model: Option<String>,
    mode: Option<String>,
    pending_model: Option<String>,
    pending_mode: Option<String>,
    session_state: SessionState,
    subscription_status: SubscriptionStatus,
    daemon_online: bool,
    retry_hint: Option<String>,
    model_context_windows: HashMap<String, u64>,
    default_context_window_tokens: Option<u64>,
    scroll_offset: usize,
    selected_tool_index: Option<usize>,
    help_open: bool,
    usage: Option<UsageSummary>,
    should_quit: bool,
    last_ctrl_c: Option<Instant>,
    stream_controller: StreamController,
    history: HistoryBuffer,
    native_scrollback: bool,
    rendered_message_count: usize,
    rendered_state_cursor: usize,
    rendered_state_keys: Vec<String>,
}

impl App {
    pub fn new(project: OpenProjectResponse) -> Self {
        Self::with_history_path(project, None)
    }

    pub fn with_history_path(project: OpenProjectResponse, history_path: Option<PathBuf>) -> Self {
        let history_entries = history_path
            .as_deref()
            .map(load_history)
            .unwrap_or_default();
        Self {
            transcript: vec![TranscriptItem::Notice(format!(
                "Opened project {} at {}",
                project.slug,
                project.root.display()
            ))],
            transcript_state: notice_transcript_state(format!(
                "Opened project {} at {}",
                project.slug,
                project.root.display()
            )),
            composer: ComposerState::new(history_entries),
            history_path,
            composer_mode: ComposerMode::Chat,
            picker: ProjectPickerState::new(Vec::new()),
            modal_picker: None,
            approval_queue: ApprovalQueue::new(),
            pending_approval_clears: 0,
            events_pane: EventsPaneState::new(),
            current_project: Some(project),
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            mode: None,
            pending_model: None,
            pending_mode: None,
            session_state: SessionState::Idle,
            subscription_status: SubscriptionStatus::Online,
            daemon_online: true,
            retry_hint: None,
            model_context_windows: HashMap::new(),
            default_context_window_tokens: None,
            scroll_offset: 0,
            selected_tool_index: None,
            help_open: false,
            usage: None,
            should_quit: false,
            last_ctrl_c: None,
            stream_controller: StreamController::new(None, std::path::Path::new(".")),
            history: HistoryBuffer::new(),
            native_scrollback: false,
            rendered_message_count: 0,
            rendered_state_cursor: 0,
            rendered_state_keys: Vec::new(),
        }
    }

    fn notice_only(notice: impl Into<String>) -> Self {
        let notice = notice.into();
        Self {
            transcript: vec![TranscriptItem::Notice(notice.clone())],
            transcript_state: notice_transcript_state(notice),
            composer: ComposerState::new(Vec::new()),
            history_path: None,
            composer_mode: ComposerMode::Chat,
            picker: ProjectPickerState::new(Vec::new()),
            modal_picker: None,
            approval_queue: ApprovalQueue::new(),
            pending_approval_clears: 0,
            events_pane: EventsPaneState::new(),
            current_project: None,
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            mode: None,
            pending_model: None,
            pending_mode: None,
            session_state: SessionState::Error,
            subscription_status: SubscriptionStatus::Offline,
            daemon_online: false,
            retry_hint: None,
            model_context_windows: HashMap::new(),
            default_context_window_tokens: None,
            scroll_offset: 0,
            selected_tool_index: None,
            help_open: false,
            usage: None,
            should_quit: false,
            last_ctrl_c: None,
            stream_controller: StreamController::new(None, std::path::Path::new(".")),
            history: HistoryBuffer::new(),
            native_scrollback: false,
            rendered_message_count: 0,
            rendered_state_cursor: 0,
            rendered_state_keys: Vec::new(),
        }
    }

    pub fn composer(&self) -> &str {
        self.composer.text()
    }

    pub fn composer_state(&self) -> &ComposerState {
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

    pub fn subscription_status(&self) -> SubscriptionStatus {
        self.subscription_status
    }

    pub fn usage(&self) -> Option<UsageSummary> {
        self.usage
    }

    pub fn context_window_tokens(&self) -> Option<u64> {
        self.model
            .as_deref()
            .and_then(|model| context_window_for_model(&self.model_context_windows, model))
            .or(self.default_context_window_tokens)
    }

    pub fn retry_hint(&self) -> Option<&str> {
        self.retry_hint.as_deref()
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

    pub fn pending_history_insertions(&mut self, width: u16) -> Vec<HistoryInsertion> {
        self.history.drain_pending(width)
    }

    pub fn history_pending_count(&self) -> usize {
        self.history.pending_cell_count()
    }

    pub fn flush_pending_paste(&mut self) -> bool {
        self.composer.flush_pending_paste(Instant::now())
    }

    pub fn set_native_scrollback(&mut self, enabled: bool) {
        if enabled && !self.native_scrollback {
            let old_selected = self.selected_tool_index;
            let old_items = std::mem::take(&mut self.transcript);
            self.selected_tool_index = None;
            for (old_idx, item) in old_items.into_iter().enumerate() {
                if item.keeps_live() {
                    let new_idx = self.transcript.len();
                    self.transcript.push(item);
                    if old_selected == Some(old_idx) {
                        self.selected_tool_index = Some(new_idx);
                    }
                } else {
                    self.history.enqueue(item);
                }
            }
        }
        self.native_scrollback = enabled;
    }

    pub fn native_scrollback(&self) -> bool {
        self.native_scrollback
    }

    pub fn begin_frame_render(&mut self) {
        self.rendered_message_count = 0;
    }

    pub fn note_rendered_messages(&mut self, count: usize) {
        self.rendered_message_count = self.rendered_message_count.saturating_add(count);
    }

    pub fn rendered_message_count(&self) -> usize {
        self.rendered_message_count
    }

    pub fn transcript_state(&self) -> &TranscriptState {
        &self.transcript_state
    }

    fn current_project_root(&self) -> Option<PathBuf> {
        self.current_project
            .as_ref()
            .map(|project| project.root.clone())
    }

    pub fn stream_has_committable_lines(&self) -> bool {
        self.stream_controller.stable_lines_ready()
    }

    pub fn active_stream_committed(&self) -> &str {
        self.stream_controller.committed()
    }

    pub fn active_stream_live(&self) -> String {
        self.stream_controller.live()
    }

    pub fn project_picker(&self) -> &ProjectPickerState {
        &self.picker
    }

    pub fn modal_picker(&self) -> Option<&PickerState> {
        self.modal_picker.as_ref()
    }

    pub fn approval_modal(&self) -> Option<&ApprovalModalState> {
        self.approval_queue.front()
    }

    #[cfg(test)]
    fn approval_pending_clear_count(&self) -> usize {
        self.pending_approval_clears
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
        self.composer.height(width.saturating_sub(2).max(1), 8)
    }

    fn open_project_picker(&mut self, projects: Vec<ProjectEntry>) {
        self.picker = ProjectPickerState::new(projects);
        self.composer_mode = ComposerMode::ProjectPicker;
        self.modal_picker = None;
    }

    fn open_model_picker(&mut self, caps: Value) {
        self.apply_caps(&caps);
        let items = model_items_from_caps(&caps);
        if items.is_empty() {
            self.add_notice("No models returned by caps");
        } else {
            self.modal_picker = Some(PickerState::new(PickerKind::Model, items));
            self.composer_mode = ComposerMode::Chat;
        }
    }

    pub fn apply_caps(&mut self, caps: &Value) {
        self.model_context_windows = model_context_windows(caps);
        self.default_context_window_tokens =
            default_context_window(caps, &self.model_context_windows);
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

    fn open_slash_command_picker(&mut self) {
        let items = command_picker_items(CommandContext {
            active_turn: self.is_chat_active(),
        });
        self.modal_picker = Some(PickerState::new(PickerKind::SlashCommand, items));
        self.composer_mode = ComposerMode::Chat;
    }

    fn open_file_mention_picker(&mut self, items: Vec<PickerItem>) {
        if items.is_empty() {
            self.modal_picker = Some(PickerState::new(
                PickerKind::FileMention,
                vec![PickerItem {
                    id: String::new(),
                    title: "No file completions loaded".to_string(),
                    description:
                        "Type after @ or use /mention again when backend search returns files"
                            .to_string(),
                }],
            ));
        } else {
            self.modal_picker = Some(PickerState::new(PickerKind::FileMention, items));
        }
        self.composer_mode = ComposerMode::Chat;
    }

    fn open_permissions_picker(&mut self) {
        self.modal_picker = Some(PickerState::multi(
            PickerKind::MultiSelect,
            vec![
                PickerItem {
                    id: "editing_tools".to_string(),
                    title: "Editing tools".to_string(),
                    description: "Patch and text-edit tools".to_string(),
                },
                PickerItem {
                    id: "dangerous_commands".to_string(),
                    title: "Dangerous commands".to_string(),
                    description: "Shell and external command approvals".to_string(),
                },
                PickerItem {
                    id: "network_tools".to_string(),
                    title: "Network tools".to_string(),
                    description: "Browser, fetch, and integration calls".to_string(),
                },
            ],
        ));
        self.composer_mode = ComposerMode::Chat;
    }

    fn update_slash_picker_filter(&mut self) {
        let Some(picker) = self.modal_picker.as_mut() else {
            return;
        };
        if picker.kind != PickerKind::SlashCommand {
            return;
        }
        let filter = self
            .composer
            .text()
            .strip_prefix('/')
            .unwrap_or_default()
            .to_string();
        picker.set_filter(filter);
    }

    fn start_file_mention_lookup(&mut self) -> AppAction {
        self.open_file_mention_picker(vec![PickerItem {
            id: String::new(),
            title: "Loading file mentions…".to_string(),
            description: "via /v1/at-command-completion".to_string(),
        }]);
        AppAction::LoadFileMentions {
            query: self.composer.text().to_string(),
            cursor: self.composer.cursor_char_offset(),
        }
    }

    fn submit_structured_prompt(&mut self, prompt: &str) -> AppAction {
        self.composer.set_text(prompt);
        self.submit_composer()
            .map(|(prompt, params)| AppAction::SendMessage { prompt, params })
            .unwrap_or(AppAction::None)
    }

    fn show_current_plan(&mut self) -> AppAction {
        self.composer.clear();
        match workflow::synthesize_current_plan(self.transcript_state.messages()) {
            Some(plan) => self.push_history_item(TranscriptItem::Plan(plan)),
            None => self.add_notice("No current plan is installed for this chat"),
        }
        AppAction::None
    }

    fn switch_to_agent_mode(&mut self) -> AppAction {
        self.composer.clear();
        self.mode = Some("agent".to_string());
        self.pending_mode = None;
        self.add_notice("Switching to Agent mode");
        AppAction::SetParams {
            patch: workflow::agent_mode_patch(),
        }
    }

    fn accept_modal_picker(&mut self, kind: PickerKind, accept: PickerAccept) -> AppAction {
        match (kind, accept) {
            (PickerKind::Model, PickerAccept::Single(Some(item))) => {
                self.pending_model = Some(item.id.clone());
                self.model = Some(item.id.clone());
                self.add_notice(format!("Model selected for next message: {}", item.title));
                AppAction::None
            }
            (PickerKind::Mode, PickerAccept::Single(Some(item))) => {
                self.pending_mode = Some(item.id.clone());
                self.mode = Some(item.id.clone());
                self.add_notice(format!("Mode selected for next message: {}", item.title));
                AppAction::None
            }
            (PickerKind::SlashCommand, PickerAccept::Single(Some(item))) => {
                self.execute_command_name(&item.id)
            }
            (PickerKind::FileMention, PickerAccept::Single(Some(item))) => {
                if item.id.is_empty() {
                    self.add_notice("File mention search did not return a selectable file");
                } else {
                    self.composer
                        .replace_current_token('@', &format!("@{} ", item.id));
                }
                AppAction::None
            }
            (PickerKind::MultiSelect, PickerAccept::Multi(items)) => {
                if items.is_empty() {
                    self.add_notice("No permissions selected");
                } else {
                    let selected = items
                        .iter()
                        .map(|item| item.title.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.add_notice(format!(
                        "Permissions picker selected {selected}; backend wiring is not yet implemented"
                    ));
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn execute_command_name(&mut self, name: &str) -> AppAction {
        let Some(command) = command_by_name(name) else {
            self.add_notice(format!("/{name} is not registered"));
            return AppAction::None;
        };
        match command.action {
            CommandAction::SendPrompt { prompt } => {
                self.composer.set_text(prompt);
                self.submit_composer()
                    .map(|(prompt, params)| AppAction::SendMessage { prompt, params })
                    .unwrap_or(AppAction::None)
            }
            CommandAction::BackendCommand { command } => {
                if command == "stop" && self.is_chat_active() {
                    self.session_state = SessionState::Idle;
                    self.clear_approvals();
                    AppAction::Abort
                } else {
                    self.composer.clear();
                    self.add_notice(format!("/{command} is not yet implemented in the TUI"));
                    AppAction::None
                }
            }
            CommandAction::OpenPicker { picker } => self.execute_picker_command(picker),
            CommandAction::LocalToggle { toggle } => self.execute_local_toggle(toggle),
            CommandAction::ShowInfo { topic } => {
                self.show_info_topic(topic);
                AppAction::None
            }
            CommandAction::Workflow { command } => self.execute_workflow_command(command),
        }
    }

    fn execute_workflow_command(&mut self, command: workflow::WorkflowCommand) -> AppAction {
        match command {
            workflow::WorkflowCommand::ShowPlan => self.show_current_plan(),
            workflow::WorkflowCommand::GoalPrompt => {
                self.submit_structured_prompt(workflow::goal_prompt())
            }
            workflow::WorkflowCommand::AgentMode => self.switch_to_agent_mode(),
            workflow::WorkflowCommand::GitDiff => {
                self.composer.clear();
                match self.current_project_root() {
                    Some(root) => AppAction::LoadDiff { root },
                    None => {
                        self.add_notice("No active project for /diff");
                        AppAction::None
                    }
                }
            }
            workflow::WorkflowCommand::ReviewPrompt => {
                self.submit_structured_prompt(workflow::review_prompt())
            }
            workflow::WorkflowCommand::CompactPrompt => {
                self.submit_structured_prompt(workflow::compact_prompt())
            }
        }
    }

    fn execute_picker_command(&mut self, picker: CommandPicker) -> AppAction {
        self.composer.clear();
        match picker {
            CommandPicker::Model => AppAction::LoadModels,
            CommandPicker::Mode => AppAction::LoadModes,
            CommandPicker::FileMention => self.start_file_mention_lookup(),
            CommandPicker::Permissions => {
                self.open_permissions_picker();
                AppAction::None
            }
        }
    }

    fn execute_local_toggle(&mut self, toggle: LocalToggle) -> AppAction {
        self.composer.clear();
        match toggle {
            LocalToggle::NewChat => {
                self.new_chat();
                AppAction::SubscribeCurrent
            }
            LocalToggle::ClearTranscript => {
                self.replace_with_notice("Transcript cleared".to_string());
                AppAction::None
            }
            LocalToggle::Quit => self.quit_action(),
        }
    }

    fn show_info_topic(&mut self, topic: InfoTopic) {
        self.composer.clear();
        match topic {
            InfoTopic::Help => self.help_open = true,
            InfoTopic::Status => {
                let project = self
                    .current_project()
                    .map(|project| project.slug.as_str())
                    .unwrap_or("-");
                let model = self.model().unwrap_or("default");
                let mode = self.mode().unwrap_or("agent");
                self.add_notice(format!(
                    "Status: project {project}, model {model}, mode {mode}, state {}",
                    self.session_state().as_str()
                ));
            }
        }
    }

    fn set_project(&mut self, project: OpenProjectResponse) {
        self.history_path = Some(history_path_for_root(&project.root));
        let history_entries = self
            .history_path
            .as_deref()
            .map(load_history)
            .unwrap_or_default();
        self.composer = ComposerState::new(history_entries);
        self.current_project = Some(project.clone());
        self.chat_id = uuid::Uuid::new_v4().to_string();
        self.session_state = SessionState::Idle;
        self.replace_with_notice(format!(
            "Switched to project {} at {}",
            project.slug,
            project.root.display()
        ));
        self.stream_controller.clear();
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.composer_mode = ComposerMode::Chat;
        self.modal_picker = None;
        self.clear_approvals();
        self.selected_tool_index = None;
        self.usage = None;
        self.model_context_windows.clear();
        self.default_context_window_tokens = None;
        self.retry_hint = None;
    }

    fn new_chat(&mut self) {
        self.chat_id = uuid::Uuid::new_v4().to_string();
        self.replace_with_session("New chat started".to_string(), None);
        self.session_state = SessionState::Idle;
        self.stream_controller.clear();
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.clear_approvals();
        self.selected_tool_index = None;
        self.usage = None;
        self.retry_hint = None;
    }

    fn submit_composer(&mut self) -> Option<(String, Value)> {
        if self.current_project.is_none() {
            return None;
        }
        let prompt = self.composer.submit_text()?;
        self.persist_history();
        self.transcript_state.push_user_message(prompt.clone());
        self.transcript_state.start_assistant(None);
        self.rebuild_render_transcript_from_state();
        self.session_state = SessionState::Generating;
        self.stream_controller.clear();
        self.usage = None;
        self.retry_hint = None;
        let params = self.take_pending_params();
        Some((prompt, params))
    }

    fn persist_history(&self) {
        if let Some(path) = &self.history_path {
            let _ = save_history(path, self.composer.history_entries());
        }
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
        self.stream_controller.push_delta(text);
        self.sync_assistant_stream_item();
    }

    fn sync_assistant_stream_item(&mut self) {
        let visible = self.stream_controller.visible();
        if visible.is_empty() {
            return;
        }
        match self.transcript.last_mut() {
            Some(TranscriptItem::Assistant(value)) => *value = visible,
            _ => self.transcript.push(TranscriptItem::Assistant(visible)),
        }
    }

    fn push_state_history_item(&mut self, key: String, item: TranscriptItem) {
        if self
            .rendered_state_keys
            .get(self.rendered_state_cursor)
            .is_some_and(|existing| existing == &key)
        {
            self.rendered_state_cursor += 1;
            return;
        }
        self.rendered_state_keys
            .truncate(self.rendered_state_cursor);
        self.rendered_state_keys.push(key);
        self.rendered_state_cursor += 1;
        self.push_history_item(item);
    }

    fn push_history_item(&mut self, item: TranscriptItem) {
        if self.native_scrollback && !item.keeps_live() {
            self.history.enqueue(item);
        } else {
            self.transcript.push(item);
        }
    }

    fn flush_live_assistant_to_history(&mut self) {
        let Some(last) = self.transcript.last() else {
            return;
        };
        if matches!(last, TranscriptItem::Assistant(_)) {
            let item = self.transcript.pop().expect("last checked");
            self.history.enqueue(item);
        }
    }

    fn run_stream_commit_tick(&mut self) {
        if run_commit_tick(&mut self.stream_controller).is_some() {
            self.sync_assistant_stream_item();
        }
    }

    fn finalize_assistant_stream(&mut self) {
        let final_content = self.stream_controller.finalize();
        if !final_content.is_empty() {
            match self.transcript.last_mut() {
                Some(TranscriptItem::Assistant(value)) => *value = final_content,
                _ => self
                    .transcript
                    .push(TranscriptItem::Assistant(final_content)),
            }
        }
        if self.native_scrollback {
            self.flush_live_assistant_to_history();
        }
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
        let text = text.into();
        self.transcript_state.push_notice(text.clone());
        self.push_history_item(TranscriptItem::Notice(text));
    }

    fn show_diff_result(&mut self, diff: String) {
        if diff.trim().is_empty() {
            self.add_notice("No git diff for the current project");
        } else {
            self.push_history_item(TranscriptItem::Diff(diff));
        }
    }

    fn replace_with_notice(&mut self, text: String) {
        self.transcript_state.reset();
        self.transcript_state.push_notice(text.clone());
        self.transcript.clear();
        self.history.clear_pending();
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.push_history_item(TranscriptItem::Notice(text));
    }

    fn replace_with_session(&mut self, title: String, subtitle: Option<String>) {
        self.transcript_state.reset();
        self.transcript_state.push_notice(title.clone());
        self.transcript.clear();
        self.history.clear_pending();
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.push_history_item(TranscriptItem::Session { title, subtitle });
    }

    fn rebuild_render_transcript_from_state(&mut self) {
        self.transcript.clear();
        self.history.clear_pending();
        self.selected_tool_index = None;
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        let messages = self.transcript_state.messages().to_vec();
        for message in &messages {
            self.append_render_message(message);
        }
    }

    fn append_render_message(&mut self, message: &TranscriptMessage) {
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
                if !message.reasoning.is_empty() {
                    self.push_state_history_item(
                        render_message_key(message, "reasoning", part),
                        TranscriptItem::Reasoning(message.reasoning.clone(), true),
                    );
                    part += 1;
                }
                if !message.content.is_empty() || message.tool_calls.is_empty() {
                    self.push_state_history_item(
                        render_message_key(message, "assistant", part),
                        TranscriptItem::Assistant(message.content.clone()),
                    );
                    part += 1;
                }
                for citation in &message.citations {
                    self.push_state_history_item(
                        render_message_key(message, "citation", part),
                        TranscriptItem::Citation(value_to_compact_string(citation)),
                    );
                    part += 1;
                }
                for block in &message.server_content_blocks {
                    self.push_state_history_item(
                        render_message_key(message, "server", part),
                        TranscriptItem::ServerContentBlock(value_to_compact_string(block)),
                    );
                    part += 1;
                }
                for tool in &message.tool_calls {
                    self.push_tool_call(tool);
                }
            }
            TranscriptRole::Tool => self.complete_tool(
                message.tool_call_id.as_deref().unwrap_or_default(),
                message.content.clone(),
                if message.tool_failed {
                    ToolStatus::Error
                } else {
                    ToolStatus::Success
                },
                now_ms(),
            ),
            TranscriptRole::Notice => {
                self.push_state_history_item(
                    render_message_key(message, "notice", 0),
                    TranscriptItem::Notice(message.content.clone()),
                );
            }
            TranscriptRole::Other(_) => {}
        }
    }

    fn approval_scope(&self, raw: &Value, event_seq: Option<u64>) -> String {
        let pause_id = raw
            .get("pause_id")
            .or_else(|| raw.get("id"))
            .or_else(|| raw.get("message_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| event_seq.map(|seq| seq.to_string()));
        match pause_id {
            Some(pause_id) if !pause_id.is_empty() => format!("{}:{pause_id}", self.chat_id),
            _ => self.chat_id.clone(),
        }
    }

    fn clear_approvals(&mut self) {
        self.approval_queue.clear();
        self.pending_approval_clears = 0;
    }

    fn enqueue_approval(&mut self, modal: ApprovalModalState) {
        self.approval_queue.push(modal);
    }

    fn pop_current_approval(&mut self) -> Option<ApprovalModalState> {
        let modal = self.approval_queue.pop_front();
        if modal.is_some() {
            self.pending_approval_clears = self.pending_approval_clears.saturating_add(1);
        }
        modal
    }

    fn handle_pause_cleared(&mut self) {
        if self.pending_approval_clears > 0 {
            self.pending_approval_clears -= 1;
        } else {
            self.approval_queue.pop_front();
        }
    }

    pub fn apply_chat_event(&mut self, event: ChatEvent) {
        self.handle_chat_event(event);
    }

    pub fn apply_stream_commit_tick(&mut self) {
        self.run_stream_commit_tick();
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
        self.subscription_status = SubscriptionStatus::Online;
        self.retry_hint = None;
        let protocol_event = event.protocol_event();
        let raw = event.raw;
        match protocol_event {
            SseEvent::Snapshot { .. } => self.handle_snapshot(&raw),
            SseEvent::StreamStarted { message_id } => {
                self.session_state = SessionState::Generating;
                self.stream_controller.clear();
                self.transcript_state.start_assistant(message_id.as_deref());
                self.rebuild_render_transcript_from_state();
            }
            SseEvent::StreamDelta { message_id, ops } => {
                self.handle_stream_delta(message_id.as_deref(), &ops)
            }
            SseEvent::StreamFinished {
                message_id, usage, ..
            } => {
                self.finalize_assistant_stream();
                self.transcript_state
                    .finish_assistant(message_id.as_deref(), usage.clone());
                if let Some(usage) = usage {
                    self.update_usage_value(&usage);
                } else {
                    self.update_usage(&raw);
                }
                if self.session_state != SessionState::Paused {
                    self.session_state = SessionState::Idle;
                }
            }
            SseEvent::RuntimeUpdated => self.handle_runtime_updated(&raw),
            SseEvent::PauseRequired => self.handle_pause_required(&raw, event.seq),
            SseEvent::PauseCleared => self.handle_pause_cleared(),
            SseEvent::MessageAdded { message } => {
                self.handle_message_added_payload(message.as_ref())
            }
            SseEvent::Unknown { .. } => {}
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
            self.transcript_state.reset_from_messages(messages);
            if self.native_scrollback {
                self.rendered_state_cursor = 0;
                let rendered_before = self.rendered_state_keys.len();
                let messages = self.transcript_state.messages().to_vec();
                for message in &messages {
                    self.append_render_message(message);
                }
                self.rendered_state_keys
                    .truncate(self.rendered_state_cursor);
                if self.rendered_state_cursor < rendered_before {
                    self.rebuild_render_transcript_from_state();
                }
            } else {
                self.rebuild_render_transcript_from_state();
            }
        }
        if let Some(runtime) = raw.get("runtime") {
            self.apply_runtime_state(runtime);
            self.update_usage(runtime);
            self.clear_approvals();
            if let Some(modal) =
                ApprovalModalState::from_event_in_scope(self.approval_scope(runtime, None), runtime)
            {
                self.enqueue_approval(modal);
            }
        }
    }

    fn handle_runtime_updated(&mut self, raw: &Value) {
        self.apply_runtime_state(raw);
    }

    fn update_usage(&mut self, raw: &Value) {
        if let Some(usage) = raw
            .get("usage")
            .or_else(|| raw.get("last_usage"))
            .or_else(|| raw.get("token_usage"))
        {
            self.update_usage_value(usage);
        }
    }

    fn update_usage_value(&mut self, usage: &Value) {
        self.transcript_state.set_usage(usage.clone());
        if let Some(summary) = UsageSummary::from_value(usage) {
            self.usage = Some(summary);
        }
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

    fn handle_pause_required(&mut self, raw: &Value, event_seq: Option<u64>) {
        self.session_state = SessionState::Paused;
        match ApprovalModalState::from_event_in_scope(self.approval_scope(raw, event_seq), raw) {
            Some(modal) => self.enqueue_approval(modal),
            None => self.add_notice("Approval required but no tool metadata was provided"),
        }
    }

    fn handle_stream_delta(&mut self, message_id: Option<&str>, ops: &[DeltaOp]) {
        self.transcript_state.apply_delta_ops(message_id, ops);
        for op in ops {
            match op {
                DeltaOp::AppendContent { text } => self.append_assistant(text),
                DeltaOp::AppendReasoning { text } => self.append_reasoning(text),
                DeltaOp::SetUsage { usage } => self.update_usage_value(usage),
                DeltaOp::AddCitation { citation } => {
                    self.push_history_item(TranscriptItem::Citation(value_to_compact_string(
                        citation,
                    )));
                }
                DeltaOp::AddServerContentBlock { block } => {
                    self.push_history_item(TranscriptItem::ServerContentBlock(
                        value_to_compact_string(block),
                    ));
                }
                DeltaOp::SetToolCalls { tool_calls } => {
                    for tool in tool_calls {
                        self.push_tool_call(tool);
                    }
                }
                DeltaOp::SetThinkingBlocks { .. }
                | DeltaOp::MergeExtra { .. }
                | DeltaOp::Unknown(_) => {}
            }
        }
    }

    fn handle_message_added_payload(&mut self, message: Option<&Value>) {
        let Some(message) = message else {
            return;
        };
        self.transcript_state.add_message(message);
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "tool" => self.handle_tool_message(message),
            "assistant" | "user" => {
                let message = TranscriptMessage::from_wire(message);
                self.append_render_message(&message);
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
        if !card.id.is_empty() {
            if let Some((idx, existing)) =
                self.transcript
                    .iter_mut()
                    .enumerate()
                    .find_map(|(idx, item)| match item {
                        TranscriptItem::Tool(existing) if existing.id == card.id => {
                            Some((idx, existing))
                        }
                        _ => None,
                    })
            {
                existing.update_from_tool_call(card);
                self.selected_tool_index = Some(idx);
                return;
            }
        }
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
                    card.result = result.clone();
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
        self.update_current_worker_from_list(&workers);
        self.events_pane.set_workers(workers);
    }

    fn push_daemon_event(&mut self, event: DaemonEventRecord) {
        self.update_current_worker_from_event(&event);
        self.events_pane.push_event(event);
    }

    fn begin_subscription_attempt(&mut self) {
        self.subscription_status = SubscriptionStatus::Waking;
        self.daemon_online = true;
    }

    fn record_chat_resubscribe(&mut self, message: &str) {
        self.subscription_status = SubscriptionStatus::Waking;
        self.daemon_online = true;
        self.retry_hint = retry_hint_from_message(message);
    }

    fn record_chat_disconnected(&mut self, message: &str) {
        if worker_waking_message(message) {
            self.subscription_status = SubscriptionStatus::Waking;
            self.daemon_online = true;
        } else {
            self.subscription_status = SubscriptionStatus::Offline;
            self.daemon_online = false;
        }
        self.retry_hint = retry_hint_from_message(message);
    }

    fn update_current_worker_from_list(&mut self, workers: &[WorkerInfo]) {
        let Some(project) = self.current_project.as_mut() else {
            return;
        };
        if let Some(worker) = workers
            .iter()
            .find(|worker| worker.project_id == project.project_id)
            .cloned()
        {
            project.worker = Some(worker);
        }
    }

    fn update_current_worker_from_event(&mut self, event: &DaemonEventRecord) {
        let Some(project_id) = event.project_id.as_deref() else {
            return;
        };
        let Some(project) = self.current_project.as_mut() else {
            return;
        };
        if project.project_id != project_id {
            return;
        }
        let state = match event.kind.as_str() {
            "worker_starting" => Some("starting"),
            "worker_ready" => Some("ready"),
            "worker_stopped" => Some("stopped"),
            "worker_crashed" => Some("crashed"),
            _ => None,
        };
        let Some(state) = state else {
            return;
        };
        match state {
            "starting" => self.subscription_status = SubscriptionStatus::Waking,
            "ready" if self.subscription_status == SubscriptionStatus::Waking => {
                self.subscription_status = SubscriptionStatus::Online;
            }
            _ => {}
        }
        let previous = project.worker.clone();
        project.worker = Some(WorkerInfo {
            project_id: project.project_id.clone(),
            pid: event
                .payload
                .get("pid")
                .and_then(Value::as_u64)
                .map(|pid| pid as u32)
                .or_else(|| previous.as_ref().and_then(|worker| worker.pid)),
            http_port: event
                .payload
                .get("http_port")
                .and_then(Value::as_u64)
                .map(|port| port as u16)
                .or_else(|| previous.as_ref().map(|worker| worker.http_port))
                .unwrap_or_default(),
            lsp_port: event
                .payload
                .get("lsp_port")
                .and_then(Value::as_u64)
                .map(|port| port as u16)
                .or_else(|| previous.as_ref().map(|worker| worker.lsp_port))
                .unwrap_or_default(),
            state: Value::String(state.to_string()),
            last_error: event
                .payload
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| previous.and_then(|worker| worker.last_error)),
        });
    }

    fn is_chat_active(&self) -> bool {
        matches!(
            self.session_state,
            SessionState::Generating | SessionState::ExecutingTools | SessionState::Paused
        )
    }

    fn quit_action(&mut self) -> AppAction {
        let abort_active = self.is_chat_active();
        if abort_active {
            self.session_state = SessionState::Idle;
            self.clear_approvals();
        }
        self.should_quit = true;
        AppAction::Quit { abort_active }
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
            } => self.quit_action(),
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
                code: KeyCode::Char('/'),
                modifiers,
                ..
            } if self.composer.is_empty()
                && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
            {
                self.composer.insert_text("/");
                self.open_slash_command_picker();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char('@'),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                self.composer.insert_text("@");
                self.start_file_mention_lookup()
            }
            KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::SHIFT,
                ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::ALT,
                ..
            } => {
                self.composer.insert_explicit_newline(Instant::now());
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } if self.composer.is_empty() && self.toggle_selected_tool() => AppAction::None,
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => match self.composer.enter(Instant::now()) {
                EnterDecision::InsertedNewline => AppAction::None,
                EnterDecision::Submit => self
                    .submit_composer()
                    .map(|(prompt, params)| AppAction::SendMessage { prompt, params })
                    .unwrap_or(AppAction::None),
            },
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.composer.backspace();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Delete,
                ..
            } => {
                self.composer.delete();
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Left,
                modifiers,
                ..
            } => {
                self.composer
                    .move_left(modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers,
                ..
            } => {
                self.composer
                    .move_right(modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Home,
                modifiers,
                ..
            } => {
                self.composer
                    .move_home(modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::End,
                modifiers,
                ..
            } => {
                self.composer
                    .move_end(modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Up,
                modifiers,
                ..
            } => {
                self.composer
                    .move_up_or_history(modifiers.contains(KeyModifiers::SHIFT));
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers,
                ..
            } => {
                self.composer
                    .move_down_or_history(modifiers.contains(KeyModifiers::SHIFT));
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
                self.composer.insert_char(ch, Instant::now());
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_approval_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        let action = {
            let modal = self.approval_queue.front_mut()?;
            modal.handle_key(key)
        };
        match action {
            ApprovalKeyAction::None | ApprovalKeyAction::ToggleFullArgs => Some(AppAction::None),
            ApprovalKeyAction::ApproveOnce => self.pop_current_approval().map(|modal| {
                self.push_history_item(TranscriptItem::Approval(
                    modal.clone(),
                    Some(ApprovalOutcome::ApprovedOnce),
                ));
                AppAction::SendToolDecisions {
                    decisions: modal.decisions(true),
                    patch: None,
                }
            }),
            ApprovalKeyAction::ApproveForChat => self.pop_current_approval().map(|modal| {
                let patch = approval_patch(&modal);
                self.push_history_item(TranscriptItem::Approval(
                    modal.clone(),
                    Some(ApprovalOutcome::ApprovedForChat),
                ));
                AppAction::SendToolDecisions {
                    patch: Some(patch),
                    decisions: modal.decisions(true),
                }
            }),
            ApprovalKeyAction::Deny => self.pop_current_approval().map(|modal| {
                self.push_history_item(TranscriptItem::Approval(
                    modal.clone(),
                    Some(ApprovalOutcome::Denied),
                ));
                AppAction::SendToolDecisions {
                    decisions: modal.decisions(false),
                    patch: None,
                }
            }),
        }
    }

    fn ctrl_c_action(&mut self) -> AppAction {
        if matches!(
            self.session_state,
            SessionState::Generating | SessionState::ExecutingTools | SessionState::Paused
        ) {
            self.session_state = SessionState::Idle;
            self.clear_approvals();
            self.add_notice("Cancel requested");
            self.last_ctrl_c = None;
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
            }
            | KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                let Some(picker) = self.modal_picker.take() else {
                    return AppAction::None;
                };
                self.accept_modal_picker(picker.kind, picker.accept())
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_prev();
                }
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    picker.select_next();
                }
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                if let Some(picker) = self.modal_picker.as_mut() {
                    if picker.is_multi() {
                        picker.toggle_selected();
                    } else {
                        self.composer.insert_char(' ', Instant::now());
                        self.update_slash_picker_filter();
                    }
                }
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                if self
                    .modal_picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == PickerKind::SlashCommand)
                {
                    self.composer.backspace();
                    self.update_slash_picker_filter();
                } else if let Some(picker) = self.modal_picker.as_mut() {
                    picker.pop_filter();
                }
                AppAction::None
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                if self
                    .modal_picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == PickerKind::SlashCommand)
                {
                    self.composer.insert_char(ch, Instant::now());
                    self.update_slash_picker_filter();
                } else if let Some(picker) = self.modal_picker.as_mut() {
                    picker.push_filter(ch);
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    #[cfg(test)]
    pub fn test_set_approval(&mut self, modal: ApprovalModalState) {
        self.clear_approvals();
        self.enqueue_approval(modal);
        self.session_state = SessionState::Paused;
    }

    #[cfg(test)]
    pub fn test_push_tool(&mut self, card: ToolCard) {
        self.transcript.push(TranscriptItem::Tool(card));
        self.selected_tool_index = Some(self.transcript.len() - 1);
    }

    #[cfg(test)]
    pub fn test_push_history_item(&mut self, item: TranscriptItem) {
        self.push_history_item(item);
    }

    #[cfg(test)]
    pub fn test_history_render_count(&self) -> usize {
        self.history.render_count()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    None,
    LoadProjects,
    LoadModels,
    LoadModes,
    LoadFileMentions {
        query: String,
        cursor: i64,
    },
    RefreshWorkers,
    OpenProject(PathBuf),
    SubscribeCurrent,
    SendMessage {
        prompt: String,
        params: Value,
    },
    SetParams {
        patch: Value,
    },
    LoadDiff {
        root: PathBuf,
    },
    SendToolDecisions {
        decisions: Vec<ToolDecision>,
        patch: Option<Value>,
    },
    Abort,
    Quit {
        abort_active: bool,
    },
}

async fn show_startup_notice(message: String) -> Result<(), TuiError> {
    let mut app = App::notice_only(message);
    let mut terminal = TerminalSession::start()?;
    apply_terminal_mode(&mut app, &terminal);
    render_frame(terminal.terminal_mut(), &mut app)?;
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
                render_frame(terminal.terminal_mut(), &mut app)?;
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
    CapsLoaded(Result<Value, String>),
    ModelsLoaded(Result<Value, String>),
    ModesLoaded(Result<Value, String>),
    FileMentionsLoaded(Result<Vec<String>, String>),
    WorkersLoaded(Result<Vec<WorkerInfo>, String>),
    CommandFinished(Result<(), String>),
    DiffLoaded(Result<String, String>),
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
    let history_path = history_path_for_root(&project.root);
    let mut app = App::with_history_path(project, Some(history_path));
    let mut terminal = TerminalSession::start()?;
    apply_terminal_mode(&mut app, &terminal);
    let (tx, mut rx) = mpsc::channel::<RuntimeEvent>(256);
    let mut subscriptions = SubscriptionManager::new();
    spawn_input_task(tx.clone());
    spawn_tick_task(tx.clone());
    spawn_daemon_events_task(client.clone(), tx.clone());
    spawn_worker_refresh_task(client.clone(), tx.clone());
    if let Some(project_id) = app.current_project_id().map(str::to_string) {
        app.begin_subscription_attempt();
        subscriptions.subscribe(
            client.clone(),
            project_id.clone(),
            app.chat_id().to_string(),
            tx.clone(),
        );
        load_caps(client.clone(), tx.clone(), project_id);
    }

    loop {
        render_frame(terminal.terminal_mut(), &mut app)?;
        if app.should_quit() {
            break;
        }
        let Some(event) = rx.recv().await else {
            break;
        };
        match event {
            RuntimeEvent::Input(Event::Key(key)) => {
                let action = app.handle_key(key);
                run_action(&mut app, action, &client, &tx, &mut subscriptions).await;
            }
            RuntimeEvent::Input(Event::Paste(text)) => app.composer.insert_paste(&text),
            RuntimeEvent::Input(Event::FocusGained | Event::FocusLost) => {}
            RuntimeEvent::Input(Event::Resize(_, _)) => {}
            RuntimeEvent::Tick => {
                app.run_stream_commit_tick();
                app.flush_pending_paste();
            }
            RuntimeEvent::Input(_) => {}
            RuntimeEvent::Chat { generation, event } => {
                subscriptions.apply_chat_event(&mut app, generation, event);
            }
            RuntimeEvent::ChatResubscribe {
                generation,
                message,
            } => {
                if subscriptions.is_current(generation) {
                    app.record_chat_resubscribe(&message);
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
                app.record_chat_disconnected(&message);
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
                if let Some(project_id) = app.current_project_id().map(str::to_string) {
                    app.begin_subscription_attempt();
                    subscriptions.subscribe(
                        client.clone(),
                        project_id.clone(),
                        app.chat_id().to_string(),
                        tx.clone(),
                    );
                    load_caps(client.clone(), tx.clone(), project_id);
                }
            }
            RuntimeEvent::ProjectOpened(Err(error)) => {
                app.add_notice(format!("Failed to open project: {error}"))
            }
            RuntimeEvent::CapsLoaded(Ok(caps)) => app.apply_caps(&caps),
            RuntimeEvent::CapsLoaded(Err(error)) => {
                if worker_waking_message(&error) {
                    app.subscription_status = SubscriptionStatus::Waking;
                }
                app.retry_hint = retry_hint_from_message(&error);
            }
            RuntimeEvent::ModelsLoaded(Ok(caps)) => app.open_model_picker(caps),
            RuntimeEvent::ModelsLoaded(Err(error)) => {
                app.retry_hint = retry_hint_from_message(&error);
                app.add_notice(format!("Failed to load models: {error}"))
            }
            RuntimeEvent::ModesLoaded(Ok(modes)) => app.open_mode_picker(modes),
            RuntimeEvent::ModesLoaded(Err(error)) => {
                app.retry_hint = retry_hint_from_message(&error);
                app.add_notice(format!("Failed to load modes: {error}"))
            }
            RuntimeEvent::FileMentionsLoaded(Ok(completions)) => {
                app.open_file_mention_picker(file_mention_items_from_completions(completions))
            }
            RuntimeEvent::FileMentionsLoaded(Err(error)) => {
                app.add_notice(format!("Failed to load file mentions: {error}"));
                app.open_file_mention_picker(Vec::new());
            }
            RuntimeEvent::WorkersLoaded(Ok(workers)) => app.set_workers(workers),
            RuntimeEvent::WorkersLoaded(Err(error)) => {
                if app.events_pane.open {
                    app.add_notice(format!("Failed to refresh workers: {error}"));
                }
            }
            RuntimeEvent::CommandFinished(Ok(())) => {}
            RuntimeEvent::CommandFinished(Err(error)) => {
                app.retry_hint = retry_hint_from_message(&error);
                app.add_notice(format!("Command failed: {error}"))
            }
            RuntimeEvent::DiffLoaded(Ok(diff)) => app.show_diff_result(diff),
            RuntimeEvent::DiffLoaded(Err(error)) => {
                app.add_notice(format!("Failed to load git diff: {error}"))
            }
        }
    }
    Ok(())
}

fn apply_terminal_mode(app: &mut App, terminal: &TerminalSession) {
    app.set_native_scrollback(terminal.mode() == crate::terminal::TerminalMode::Inline);
}

fn history_path_for_root(root: &std::path::Path) -> PathBuf {
    let hash = stable_path_hash(root);
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("refact")
        .join("tui_history")
        .join(format!("{hash}.json"))
}

fn stable_path_hash(path: &std::path::Path) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

async fn run_action(
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
        AppAction::LoadFileMentions { query, cursor } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .at_command_completion(&project_id, &query, cursor, 20)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::FileMentionsLoaded(result)).await;
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
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                app.begin_subscription_attempt();
                subscriptions.subscribe(
                    client.clone(),
                    project_id,
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
        AppAction::SetParams { patch } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .send_set_params(&project_id, &chat_id, patch)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx.send(RuntimeEvent::CommandFinished(result)).await;
                });
            }
        }
        AppAction::LoadDiff { root } => {
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = load_git_diff(root).await;
                let _ = tx.send(RuntimeEvent::DiffLoaded(result)).await;
            });
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
        AppAction::Quit { abort_active } => {
            if abort_active {
                if let Err(error) = send_abort_for_current_chat(app, client).await {
                    app.add_notice(format!("Abort before quit failed: {error}"));
                }
            }
        }
    }
}

async fn load_git_diff(root: PathBuf) -> Result<String, String> {
    tokio::task::spawn_blocking(move || git_diff(&root))
        .await
        .map_err(|error| error.to_string())?
}

fn git_diff(root: &std::path::Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("diff")
        .arg("--no-ext-diff")
        .arg("--")
        .current_dir(root)
        .output()
        .map_err(|error| format!("failed to run git diff: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("git diff exited with {}", output.status)
        } else {
            stderr
        })
    }
}

async fn send_abort_for_current_chat(app: &App, client: &DaemonClient) -> Result<(), String> {
    let Some(project_id) = app.current_project_id().map(str::to_string) else {
        return Ok(());
    };
    let chat_id = app.chat_id().to_string();
    match tokio::time::timeout(
        ABORT_BEFORE_QUIT_TIMEOUT,
        client.send_abort(&project_id, &chat_id),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("abort request timed out".to_string()),
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

fn load_caps(client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>, project_id: String) {
    tokio::spawn(async move {
        let result = client
            .get_caps(&project_id)
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(RuntimeEvent::CapsLoaded(result)).await;
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

fn notice_transcript_state(text: String) -> TranscriptState {
    let mut state = TranscriptState::new();
    state.push_notice(text);
    state
}

fn token_count(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value.get(*key)?.as_u64())
}

fn model_context_windows(caps: &Value) -> HashMap<String, u64> {
    let mut windows = HashMap::new();
    if let Some(models) = caps.get("chat_models") {
        collect_model_context_windows(models, &mut windows);
    }
    if let Some(models) = caps.get("models").and_then(|models| models.get("chat")) {
        collect_model_context_windows(models, &mut windows);
    }
    if let Some(models) = caps.get("available_models") {
        collect_model_context_windows(models, &mut windows);
    }
    windows
}

fn collect_model_context_windows(models: &Value, windows: &mut HashMap<String, u64>) {
    match models {
        Value::Object(map) => {
            for (id, model) in map {
                insert_model_context_window(id, model, windows);
            }
        }
        Value::Array(items) => {
            for model in items {
                if let Some(id) = model.get("id").and_then(Value::as_str) {
                    insert_model_context_window(id, model, windows);
                }
            }
        }
        _ => {}
    }
}

fn insert_model_context_window(id: &str, model: &Value, windows: &mut HashMap<String, u64>) {
    let Some(window) = context_window_from_model(model) else {
        return;
    };
    if !id.is_empty() {
        windows.insert(id.to_string(), window);
    }
    if let Some(model_id) = model
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        windows.insert(model_id.to_string(), window);
    }
}

fn context_window_from_model(model: &Value) -> Option<u64> {
    token_count(
        model,
        &[
            "n_ctx",
            "context_window",
            "context_window_tokens",
            "context_length",
            "max_context_window_tokens",
            "max_prompt_tokens",
            "max_model_len",
        ],
    )
    .or_else(|| model.get("limits").and_then(context_window_from_model))
    .or_else(|| model.get("base").and_then(context_window_from_model))
}

fn default_context_window(caps: &Value, windows: &HashMap<String, u64>) -> Option<u64> {
    default_chat_model(caps)
        .and_then(|model| context_window_for_model(windows, model))
        .or_else(|| {
            (windows.len() == 1)
                .then(|| windows.values().next().copied())
                .flatten()
        })
}

fn default_chat_model(caps: &Value) -> Option<&str> {
    caps.get("defaults")
        .and_then(|defaults| {
            string_field(
                defaults,
                &[
                    "chat_default_model",
                    "default_chat_model",
                    "chat_model",
                    "model",
                ],
            )
        })
        .or_else(|| {
            string_field(
                caps,
                &[
                    "chat_default_model",
                    "default_chat_model",
                    "chat_model",
                    "model",
                ],
            )
        })
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_str())
        .filter(|value| !value.is_empty())
}

fn context_window_for_model(windows: &HashMap<String, u64>, model: &str) -> Option<u64> {
    windows.get(model).copied().or_else(|| {
        windows.iter().find_map(|(id, window)| {
            id.rsplit('/').next().filter(|suffix| *suffix == model)?;
            Some(*window)
        })
    })
}

fn worker_waking_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("status 502")
        || message.contains("status 503")
        || message.contains("status 504")
        || message.contains("bad gateway")
        || message.contains("service unavailable")
        || (message.contains("worker")
            && (message.contains("starting")
                || message.contains("waking")
                || message.contains("spawn")
                || message.contains("not ready")
                || message.contains("unavailable")))
}

fn retry_hint_from_message(message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("retry-after")
        || lower.contains("retry_after")
    {
        return Some(
            retry_after_hint(message)
                .map(|retry_after| format!("rate limited; retry after {retry_after}"))
                .unwrap_or_else(|| "rate limited; retry later".to_string()),
        );
    }
    if worker_waking_message(message) {
        return Some("worker waking; retrying".to_string());
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return Some("request timed out; retrying".to_string());
    }
    None
}

fn retry_after_hint(message: &str) -> Option<String> {
    [
        "retry_after_ms",
        "retry-after",
        "retry_after",
        "retry after",
    ]
    .into_iter()
    .find_map(|needle| retry_value_after(message, needle))
}

fn retry_value_after(message: &str, needle: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    let start = lower.find(needle)? + needle.len();
    let rest = message
        .get(start..)?
        .trim_start_matches(|ch: char| ch == ':' || ch == '=' || ch == '"' || ch.is_whitespace());
    let value = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '-')
        .collect::<String>();
    (!value.is_empty()).then_some(value)
}

fn value_to_compact_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn render_message_key(message: &TranscriptMessage, part: &str, index: usize) -> String {
    let id = message
        .message_id
        .as_deref()
        .or(message.tool_call_id.as_deref())
        .unwrap_or_default();
    if id.is_empty() {
        format!(
            "{}:{}:{}:{}",
            message.role.as_str(),
            part,
            index,
            message.content.len()
        )
    } else {
        format!("{}:{}:{}", id, part, index)
    }
}

fn render_frame(
    terminal: &mut crate::terminal::RefactTerminal,
    app: &mut App,
) -> Result<(), TuiError> {
    if app.native_scrollback() {
        let width = terminal.size()?.width;
        for insertion in app.pending_history_insertions(width) {
            insert_history(terminal, insertion)?;
        }
    }
    terminal.draw(|frame| crate::ui::render(frame, app))?;
    Ok(())
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;

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

    fn pause_event(app: &App, tool_call_id: &str, tool_name: &str) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "pause_required".to_string(),
            raw: json!({"reasons": [{"type": "confirmation", "tool_name": tool_name, "command": format!("{tool_name}({tool_call_id})"), "rule": "*", "tool_call_id": tool_call_id}]}),
        }
    }

    fn assistant_text(app: &App) -> String {
        app.visible_transcript()
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Assistant(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>()
    }

    fn tool_cards(app: &App) -> Vec<&ToolCard> {
        app.visible_transcript()
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Tool(card) => Some(card),
                _ => None,
            })
            .collect()
    }

    #[derive(Clone, Default)]
    struct CommandState(Arc<(Mutex<Vec<Value>>, Condvar)>);

    impl CommandState {
        fn push(&self, command: Value) {
            let (lock, cond) = &*self.0;
            lock.lock().unwrap().push(command);
            cond.notify_all();
        }

        fn wait_for(&self, command_type: &str) -> bool {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (lock, cond) = &*self.0;
            let mut commands = lock.lock().unwrap();
            loop {
                if commands.iter().any(|command| {
                    command.get("type").and_then(Value::as_str) == Some(command_type)
                }) {
                    return true;
                }
                let now = Instant::now();
                if now >= deadline {
                    return false;
                }
                let wait = deadline.saturating_duration_since(now);
                let (next_commands, timeout) = cond.wait_timeout(commands, wait).unwrap();
                commands = next_commands;
                if timeout.timed_out() {
                    return false;
                }
            }
        }

        fn find_command(&self, command_type: &str) -> Option<Value> {
            let (lock, _) = &*self.0;
            lock.lock()
                .unwrap()
                .iter()
                .find(|command| command.get("type").and_then(Value::as_str) == Some(command_type))
                .cloned()
        }
    }

    fn spawn_command_server(state: CommandState) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                handle_command_connection(stream, state.clone());
            }
        });
        format!("http://{addr}")
    }

    fn handle_command_connection(mut stream: TcpStream, state: CommandState) {
        let mut data = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let Ok(n) = stream.read(&mut buf) else {
                return;
            };
            if n == 0 {
                return;
            }
            data.extend_from_slice(&buf[..n]);
            if data.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let Some(header_end) = data
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|idx| idx + 4)
        else {
            return;
        };
        let headers = String::from_utf8_lossy(&data[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then_some(value)
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while data.len() < header_end + content_length {
            let Ok(n) = stream.read(&mut buf) else {
                return;
            };
            if n == 0 {
                return;
            }
            data.extend_from_slice(&buf[..n]);
        }
        if content_length > 0 {
            let body = &data[header_end..header_end + content_length];
            state.push(serde_json::from_slice(body).unwrap_or(Value::Null));
        }
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"status\":\"accepted\"}";
        let _ = stream.write_all(response);
    }

    #[test]
    fn alternate_screen_mode_keeps_legacy_transcript_live() {
        let mut app = App::new(project());
        assert!(!app.native_scrollback());
        app.test_push_history_item(TranscriptItem::Notice("legacy".to_string()));
        assert_eq!(app.visible_transcript().len(), 2);
        assert_eq!(app.history_pending_count(), 0);
    }

    #[test]
    fn native_scrollback_queues_finalized_history() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.test_push_history_item(TranscriptItem::Notice("native".to_string()));
        assert_eq!(app.visible_transcript().len(), 0);
        assert_eq!(app.history_pending_count(), 2);
    }

    #[test]
    fn app_submits_message_and_streams_answer() {
        let mut app = App::new(project());
        app.composer.set_text("hello");
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
        assert_eq!(assistant_text(&app), "hi");
        assert!(!app.stream_has_committable_lines());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        assert_eq!(assistant_text(&app), "hi\n");
        assert_eq!(app.session_state(), SessionState::Idle);
    }

    #[test]
    fn app_newline_keys_insert_and_enter_submits() {
        let mut app = App::new(project());
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));
        assert_eq!(app.composer(), "a\nb");
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "a\nb"
        ));
    }

    #[test]
    fn slash_popup_filters_and_executes_local_command() {
        let mut app = App::new(project());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty())),
            AppAction::None
        );
        assert_eq!(app.composer(), "/");
        assert!(app.modal_picker().is_some());
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty()));
        let picker = app.modal_picker().unwrap();
        assert_eq!(picker.filtered_items()[0].id, "status");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| matches!(item, TranscriptItem::Notice(text) if text.contains("Status:"))));
    }

    #[test]
    fn at_opens_file_popup_and_accept_inserts_mention() {
        let mut app = App::new(project());
        app.composer.set_text("read ");
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::empty())),
            AppAction::LoadFileMentions { query, .. } if query == "read @"
        ));
        app.open_file_mention_picker(vec![PickerItem {
            id: "src/lib.rs".to_string(),
            title: "src/lib.rs".to_string(),
            description: "file mention".to_string(),
        }]);
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.composer(), "read @src/lib.rs ");
    }

    #[test]
    fn permissions_command_uses_multi_select_picker() {
        let mut app = App::new(project());
        app.execute_command_name("permissions");
        assert!(app.modal_picker().unwrap().is_multi());
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| matches!(item, TranscriptItem::Notice(text) if text.contains("Dangerous commands") && text.contains("Network tools"))));
    }

    #[test]
    fn plan_command_shows_current_plan_cell() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {"role": "plan", "content": "base plan", "extra": {"plan": {"mode": "agent", "version": 1}}}}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {"role": "event", "content": "delta one", "extra": {"event": {"subkind": "plan_delta", "payload": {"seq": 1}}}}}),
        });

        assert_eq!(app.execute_command_name("plan"), AppAction::None);
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Plan(text) if text.contains("base plan") && text.contains("delta one"))
        }));
    }

    #[test]
    fn goal_review_and_compact_insert_structured_prompts() {
        let mut app = App::new(project());
        assert!(matches!(
            app.execute_command_name("goal"),
            AppAction::SendMessage { prompt, .. } if prompt.contains("clarify the current goal")
        ));

        let mut app = App::new(project());
        assert!(matches!(
            app.execute_command_name("review"),
            AppAction::SendMessage { prompt, .. } if prompt.contains("Review the current project changes")
        ));

        let mut app = App::new(project());
        assert!(matches!(
            app.execute_command_name("compact"),
            AppAction::SendMessage { prompt, .. } if prompt.contains("ctx_probe")
        ));
    }

    #[test]
    fn agent_command_emits_mode_switch_patch() {
        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("agent"),
            AppAction::SetParams {
                patch: json!({"mode": "agent", "tool_use": "agent"})
            }
        );
        assert_eq!(app.mode(), Some("agent"));
    }

    #[test]
    fn diff_command_loads_local_project_diff() {
        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("diff"),
            AppAction::LoadDiff {
                root: PathBuf::from("/tmp/demo")
            }
        );
    }

    #[test]
    fn show_diff_result_renders_diff_cell() {
        let mut app = App::new(project());
        app.show_diff_result("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new".to_string());
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| { matches!(item, TranscriptItem::Diff(text) if text.contains("+new")) }));
    }

    #[tokio::test]
    async fn mode_switch_command_posts_set_params() {
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        let action = app.execute_command_name("agent");
        let (tx, mut rx) = mpsc::channel(1);
        let mut subscriptions = SubscriptionManager::new();
        run_action(&mut app, action, &client, &tx, &mut subscriptions).await;
        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::CommandFinished(Ok(())))
        ));

        let command = state.find_command("set_params").unwrap();
        assert_eq!(command["patch"]["mode"], "agent");
        assert_eq!(command["patch"]["tool_use"], "agent");
    }

    #[tokio::test]
    async fn load_git_diff_reads_project_root() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .arg("init")
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success());
        std::fs::write(dir.path().join("file.txt"), "old\n").unwrap();
        assert!(Command::new("git")
            .args(["add", "file.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("git")
            .args([
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=T",
                "commit",
                "-m",
                "init"
            ])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success());
        std::fs::write(dir.path().join("file.txt"), "new\n").unwrap();

        let diff = load_git_diff(dir.path().to_path_buf()).await.unwrap();
        assert!(diff.contains("--- a/file.txt"));
        assert!(diff.contains("+++ b/file.txt"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }

    #[test]
    fn app_paste_burst_does_not_submit_on_embedded_enters() {
        let mut app = App::new(project());
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));
        let action = app.handle_key(key(KeyCode::Enter));
        assert_eq!(action, AppAction::None);
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty()));
        let action = app.handle_key(key(KeyCode::Enter));
        assert_eq!(action, AppAction::None);
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()));
        app.composer
            .flush_pending_paste(Instant::now() + Duration::from_millis(100));
        assert_eq!(app.composer(), "ab\nc\nd");
    }

    #[test]
    fn app_history_persists_and_restores_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut app = App::with_history_path(project(), Some(path.clone()));
        app.composer.set_text("first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { prompt, .. } if prompt == "first"
        ));
        let mut app = App::with_history_path(project(), Some(path));
        app.composer.set_text("draft");
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.composer(), "first");
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.composer(), "draft");
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
    fn ctrl_c_during_generation_aborts_without_arming_quit_countdown() {
        let mut app = App::new(project());
        app.session_state = SessionState::Generating;

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.should_quit());
        assert!(app.last_ctrl_c.is_none());

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(!app.should_quit());
    }

    #[tokio::test]
    async fn active_generation_quit_sends_abort_command() {
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        app.session_state = SessionState::Generating;
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert_eq!(action, AppAction::Quit { abort_active: true });
        assert!(app.should_quit());

        let (tx, _rx) = mpsc::channel(1);
        let mut subscriptions = SubscriptionManager::new();
        run_action(&mut app, action, &client, &tx, &mut subscriptions).await;

        assert!(state.wait_for("abort"));
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
    fn two_approvals_before_first_resolved_preserve_fifo() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));
        app.handle_chat_event(pause_event(&app, "call-2", "cat"));

        let first = app.approval_modal().unwrap();
        assert_eq!(first.reasons()[0].tool_call_id, "call-1");
        assert_eq!(first.pending_after(), 1);

        let first_action = app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(
            first_action,
            AppAction::SendToolDecisions {
                decisions: vec![ToolDecision {
                    tool_call_id: "call-1".to_string(),
                    accepted: true,
                }],
                patch: None,
            }
        );
        assert_eq!(app.approval_pending_clear_count(), 1);
        let second = app.approval_modal().unwrap();
        assert_eq!(second.reasons()[0].tool_call_id, "call-2");
        assert_eq!(second.pending_after(), 0);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "pause_cleared".to_string(),
            raw: json!({}),
        });
        assert!(app.approval_modal().is_some());
        assert_eq!(app.approval_pending_clear_count(), 0);

        let second_action = app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(
            second_action,
            AppAction::SendToolDecisions {
                decisions: vec![ToolDecision {
                    tool_call_id: "call-2".to_string(),
                    accepted: false,
                }],
                patch: None,
            }
        );
        assert!(app.approval_modal().is_none());
    }

    #[test]
    fn markdown_collector_commits_complete_lines_to_transcript() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_started".to_string(),
            raw: json!({}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "append_content", "text": "# Title\npartial"}]}),
        });
        assert_eq!(assistant_text(&app), "# Title\npartial");
        assert!(app.stream_has_committable_lines());

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "append_content", "text": " tail"}]}),
        });
        assert_eq!(assistant_text(&app), "# Title\npartial tail");
        assert!(app.stream_has_committable_lines());

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        assert_eq!(assistant_text(&app), "# Title\npartial tail\n");
    }

    #[test]
    fn set_tool_calls_updates_existing_card_by_id() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "shell", "arguments": "{\"cmd\":\"echo 1\"}"}}]}]}),
        });
        assert_eq!(tool_cards(&app).len(), 1);
        app.complete_tool("call-1", "done".to_string(), ToolStatus::Success, now_ms());
        if let [card] = tool_cards(&app).as_slice() {
            assert_eq!(card.status, ToolStatus::Success);
            assert_eq!(card.result, "done");
        } else {
            panic!("expected one card");
        }
        app.handle_key(key(KeyCode::Enter));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "shell", "arguments": "{\"cmd\":\"echo 2\"}"}}]}]}),
        });
        let cards = tool_cards(&app);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].status, ToolStatus::Success);
        assert_eq!(cards[0].result, "done");
        assert!(cards[0].expanded);
        assert!(cards[0].args_preview.contains("echo 2"));
    }

    #[test]
    fn reasoning_toggle_survives_append_updates() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "append_reasoning", "text": "first"}]}),
        });
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Reasoning(_, true))
        ));
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Reasoning(_, false))
        ));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "append_reasoning", "text": " second"}]}),
        });
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Reasoning(text, false)) if text == "first second"
        ));
    }

    #[test]
    fn tool_expand_survives_result_update() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "shell", "arguments": "{\"cmd\":\"echo hi\"}"}}]}]}),
        });
        assert!(app.toggle_selected_tool());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {"role": "tool", "tool_call_id": "call-1", "content": "done", "tool_failed": false}}),
        });
        let cards = tool_cards(&app);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].status, ToolStatus::Success);
        assert_eq!(cards[0].result, "done");
        assert!(cards[0].expanded);
    }

    #[test]
    fn approval_decision_appends_record_cell() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));
        let action = app.handle_key(key(KeyCode::Char('y')));
        assert!(matches!(action, AppAction::SendToolDecisions { .. }));
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Approval(
                _,
                Some(ApprovalOutcome::ApprovedOnce)
            ))
        ));
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
        app.composer.set_text("hello");
        let action = app.handle_key(key(KeyCode::Enter));
        match action {
            AppAction::SendMessage { params, .. } => {
                assert_eq!(params["model"], "m1");
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn caps_update_context_window_for_selected_and_default_models() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "defaults": {"chat_default_model": "openai/gpt-demo"},
            "chat_models": {
                "openai/gpt-demo": {"n_ctx": 128_000},
                "openai/gpt-small": {"n_ctx": 32_000}
            }
        }));

        assert_eq!(app.context_window_tokens(), Some(128_000));

        app.open_model_picker(
            json!({"chat_models": {"openai/gpt-small": {"name": "Small", "n_ctx": 32_000}}}),
        );
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);

        assert_eq!(app.context_window_tokens(), Some(32_000));
    }

    #[test]
    fn subscription_status_tracks_waking_offline_and_retry_hints() {
        let mut app = App::new(project());

        app.begin_subscription_attempt();
        assert_eq!(app.subscription_status(), SubscriptionStatus::Waking);
        assert!(app.daemon_online());

        app.record_chat_disconnected("request failed with status 503: worker starting");
        assert_eq!(app.subscription_status(), SubscriptionStatus::Waking);
        assert_eq!(app.retry_hint(), Some("worker waking; retrying"));
        assert!(app.daemon_online());

        app.record_chat_disconnected("connection refused");
        assert_eq!(app.subscription_status(), SubscriptionStatus::Offline);
        assert!(!app.daemon_online());

        app.record_chat_resubscribe("request failed with status 429: retry-after: 2s");
        assert_eq!(app.subscription_status(), SubscriptionStatus::Waking);
        assert_eq!(app.retry_hint(), Some("rate limited; retry after 2s"));
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
            json!({"ops": [{"op": "append_content", "text": "kept\n"}]}),
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
        assert_eq!(assistant_text(&app), "kept\n");
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
        let delta = json!({"ops": [{"op": "append_content", "text": "once\n"}]});
        let first = chat_event(&app, 2, "stream_delta", delta.clone());
        assert_eq!(
            apply_tracked(&mut app, &mut tracker, first),
            ChatSeqDecision::Apply
        );
        let duplicate = chat_event(&app, 2, "stream_delta", delta);
        let decision = apply_tracked(&mut app, &mut tracker, duplicate);
        assert!(matches!(decision, ChatSeqDecision::Resubscribe(_)));
        assert_eq!(assistant_text(&app), "once\n");
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
            json!({"ops": [{"op": "append_content", "text": "fresh\n"}]}),
        );
        assert!(subscriptions.apply_chat_event(&mut app, 2, fresh));
        assert_eq!(assistant_text(&app), "fresh\n");
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
