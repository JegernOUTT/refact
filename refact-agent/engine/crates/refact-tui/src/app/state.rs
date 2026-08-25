use super::session_lifecycle::context_window_for_model;
use super::*;
use crate::client::{request_id, BrowserContextDecision};
use crate::commands::session as command_session;
use crate::protocol::{
    BrowserClosedEvent, BrowserContextOversizeEvent, BrowserFrameEvent, BrowserSnapshot,
    RuntimeUpdatedEvent,
};
use super::surfaces::activity::ActivitySurfaceState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistorySaveRequest {
    pub(super) path: PathBuf,
    pub(super) entries: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    Chat,
    ProjectPicker,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserContextDecisionOptions {
    pub include_actions: bool,
    pub include_console: bool,
    pub include_network: bool,
    pub include_mutations: bool,
    pub include_screenshot: bool,
    pub last_n_actions: Option<usize>,
    pub last_n_console: Option<usize>,
    pub last_n_network: Option<usize>,
}

impl BrowserContextDecisionOptions {
    pub fn include_all() -> Self {
        Self {
            include_actions: true,
            include_console: true,
            include_network: true,
            include_mutations: true,
            include_screenshot: true,
            last_n_actions: None,
            last_n_console: None,
            last_n_network: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserContextPromptState {
    pub event: BrowserContextOversizeEvent,
}

impl BrowserContextPromptState {
    fn decision(&self, options: BrowserContextDecisionOptions) -> BrowserContextDecision {
        BrowserContextDecision {
            pending_message_id: self.event.pending_message_id.clone(),
            include_actions: options.include_actions,
            include_console: options.include_console,
            include_network: options.include_network,
            include_mutations: options.include_mutations,
            include_screenshot: options.include_screenshot,
            last_n_actions: options.last_n_actions,
            last_n_console: options.last_n_console,
            last_n_network: options.last_n_network,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserState {
    pub is_open: bool,
    pub connected: bool,
    pub runtime_id: Option<String>,
    pub active_tab: Option<String>,
    pub current_url: Option<String>,
    pub current_title: Option<String>,
    pub tabs: Vec<crate::protocol::BrowserTab>,
    pub latest_frame: Option<BrowserFrameEvent>,
    pub timeline: Vec<Value>,
    pub last_toolbar_action: Option<String>,
    pub last_closed: Option<BrowserClosedEvent>,
    pub context_prompt: Option<BrowserContextPromptState>,
}

impl BrowserState {
    pub(super) fn apply_snapshot(&mut self, snapshot: Option<BrowserSnapshot>) {
        *self = Self::default();
        if let Some(snapshot) = snapshot {
            self.apply_status(snapshot);
        }
    }

    pub(super) fn apply_status(&mut self, snapshot: BrowserSnapshot) {
        self.is_open = !snapshot.runtime_id.is_empty();
        self.connected = snapshot.connected;
        self.runtime_id = (!snapshot.runtime_id.is_empty()).then_some(snapshot.runtime_id);
        self.active_tab = snapshot.active_tab;
        self.current_url = snapshot.url;
        self.current_title = snapshot.title;
        self.tabs = snapshot.tabs;
        self.last_closed = None;
    }

    pub(super) fn apply_frame(&mut self, frame: BrowserFrameEvent) {
        self.latest_frame = Some(frame);
    }

    pub(super) fn apply_closed(&mut self, event: BrowserClosedEvent) {
        if self
            .runtime_id
            .as_deref()
            .is_none_or(|runtime_id| runtime_id == event.runtime_id)
        {
            self.is_open = false;
            self.connected = false;
            self.runtime_id = Some(event.runtime_id.clone());
            self.active_tab = None;
            self.current_url = None;
            self.current_title = None;
            self.tabs.clear();
            self.context_prompt = None;
        }
        self.last_closed = Some(event);
    }

    pub(super) fn apply_timeline(&mut self, events: Vec<Value>) {
        self.timeline.extend(events);
    }

    pub(super) fn apply_context_oversize(&mut self, event: BrowserContextOversizeEvent) {
        self.context_prompt = Some(BrowserContextPromptState { event });
    }

    pub(super) fn apply_toolbar_action(&mut self, action: String) {
        self.last_toolbar_action = Some(action);
    }

    pub(super) fn take_context_prompt(
        &mut self,
        options: BrowserContextDecisionOptions,
    ) -> Option<(BrowserContextDecision, BrowserContextPromptState)> {
        let prompt = self.context_prompt.take()?;
        let decision = prompt.decision(options);
        Some((decision, prompt))
    }

    pub(super) fn restore_context_prompt(&mut self, prompt: BrowserContextPromptState) {
        if self.context_prompt.is_none() {
            self.context_prompt = Some(prompt);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BacktrackTarget {
    pub(super) index: usize,
    pub(super) message_id: Option<String>,
    pub(super) content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct LocalInputOwner {
    pub(super) project_id: String,
    pub(super) chat_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct LocalInputHandoff {
    pub(super) composer: ComposerState,
    pub(super) input_queue: InputQueue,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingSendRetry {
    pub(super) prompt: String,
    pub(super) params: Value,
    pub(super) correlation: ClientMessageCorrelation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InFlightSend {
    pub(super) correlation: ClientMessageCorrelation,
    pub(super) accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientMessageCorrelation {
    pub client_request_id: String,
    pub client_message_id: String,
}

impl ClientMessageCorrelation {
    pub(crate) fn new() -> Self {
        Self {
            client_request_id: request_id("user-message"),
            client_message_id: request_id("client-message"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingApprovalClear {
    pub(super) scope: String,
    pub(super) tool_call_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReasoningStateSnapshot {
    pub(super) boost_reasoning: bool,
    pub(super) reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingReasoningRollback {
    pub(super) patch: Value,
    pub(super) previous: ReasoningStateSnapshot,
}

#[derive(Debug, Clone)]
pub(super) struct BacktrackRollback {
    pub(super) transcript_state: TranscriptState,
    pub(super) session_state: SessionState,
    pub(super) usage: Option<UsageSummary>,
    pub(super) selected_backtrack_index: Option<usize>,
    pub(super) backtrack_target: Option<BacktrackTarget>,
    pub(super) backtrack_pending: Option<BacktrackTarget>,
    pub(super) last_escape_at: Option<Instant>,
    pub(super) prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDecisionRollback {
    pub(super) approval: ApprovalModalState,
    pub(super) tool_statuses: Vec<(String, ToolStatus)>,
}

impl BacktrackTarget {
    pub(super) fn matches(&self, message: &TranscriptMessage) -> bool {
        if let Some(message_id) = self.message_id.as_deref() {
            message.message_id.as_deref() == Some(message_id)
        } else {
            message.role == TranscriptRole::User && message.content == self.content
        }
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub(super) transcript: Vec<TranscriptItem>,
    pub(super) transcript_state: TranscriptState,
    pub(super) inbound_event_state: InboundEventState,
    pub(super) browser_state: BrowserState,
    pub(super) runtime_snapshot: Option<RuntimeUpdatedEvent>,
    pub(super) composer: ComposerState,
    pub(super) keymap: KeymapRegistry,
    pub(super) vim: VimState,
    pub(super) theme: TuiTheme,
    pub(super) input_queue: InputQueue,
    pub(super) input_queue_owner: Option<LocalInputOwner>,
    pub(super) local_input_handoffs: HashMap<LocalInputOwner, LocalInputHandoff>,
    pub(super) last_chat_by_project: HashMap<String, String>,
    pub(super) server_queue_size: usize,
    pub(super) server_queue_previews: Vec<String>,
    pub(super) history_path: Option<PathBuf>,
    pub(super) pending_history_save: Option<HistorySaveRequest>,
    pub(super) history_save_in_flight: bool,
    pub(super) history_failure_notified: bool,
    pub(super) tui_config_path: Option<PathBuf>,
    pub(super) composer_mode: ComposerMode,
    pub(super) picker: surfaces::ProjectPickerState,
    pub(super) modal_picker: Option<PickerState>,
    pub(super) theme_picker_snapshot: Option<surfaces::ThemePickerSnapshot>,
    pub(super) approval_queue: ApprovalQueue,
    pub(super) ask_questions_form: Option<AskQuestionsForm>,
    pub(super) pending_manual_ask_questions: Option<AskQuestionsRequest>,
    pub(super) handled_ask_questions_tool_ids: HashSet<String>,
    pub(super) pending_approval_clears: VecDeque<PendingApprovalClear>,
    pub(super) events_pane: EventsPaneState,
    pub(super) current_project: Option<OpenProjectResponse>,
    pub(super) chat_id: String,
    pub(super) worktree_meta: Option<crate::sessions::WorktreeMeta>,
    pub(super) pending_worktree_merge: Option<surfaces::WorktreeMergeConfirmation>,
    pub(super) session_title: Option<String>,
    pub(super) recent_sessions: Vec<PickerItem>,
    pub(super) show_session_header: bool,
    pub(super) model: Option<String>,
    pub(super) mode: Option<String>,
    pub(super) boost_reasoning: bool,
    pub(super) reasoning_effort: Option<String>,
    pub(super) pending_reasoning_rollback: Option<PendingReasoningRollback>,
    pub(super) pending_backtrack_rollback: Option<BacktrackRollback>,
    pub(super) pending_model: Option<String>,
    pub(super) pending_mode: Option<String>,
    pub(super) in_flight_send: Option<InFlightSend>,
    pub(super) pending_send_retry: Option<PendingSendRetry>,
    pub(super) session_state: SessionState,
    pub(super) subscription_status: SubscriptionStatus,
    pub(super) daemon_online: bool,
    pub(super) daemon_status: Option<DaemonStatus>,
    pub(super) daemon_base_url: Option<String>,
    pub(super) permission_policy: session::PermissionPolicy,
    pub(super) retry_hint: Option<String>,
    pub(super) model_context_windows: HashMap<String, u64>,
    pub(super) model_reasoning_caps: HashMap<String, ReasoningModelCaps>,
    pub(super) model_settings_caps: HashMap<String, surfaces::ModelSettingsCapabilities>,
    pub(super) default_context_window_tokens: Option<u64>,
    pub(super) thread_params: Value,
    pub(super) settings_surface: Option<surfaces::SettingsState>,
    pub(super) scroll_offset: usize,
    pub(super) selected_tool_index: Option<usize>,
    pub(super) selected_backtrack_index: Option<usize>,
    pub(super) backtrack_target: Option<BacktrackTarget>,
    pub(super) backtrack_pending: Option<BacktrackTarget>,
    pub(super) last_escape_at: Option<Instant>,
    pub(super) transcript_overlay: Option<PagerOverlay>,
    pub(super) board_surface: Option<surfaces::board::BoardSurface>,
    pub(super) transcript_overlay_visible_height: Option<usize>,
    pub(super) activity_surface: Option<ActivitySurfaceState>,
    pub(super) task_id: Option<String>,
    pub(super) history_surface: Option<surfaces::HistorySurface>,
    pub(super) goal_overlay_open: bool,
    pub(super) help_open: bool,
    pub(super) usage: Option<UsageSummary>,
    pub(super) should_quit: bool,
    pub(super) abort_in_flight: bool,
    pub(super) last_ctrl_c: Option<Instant>,
    pub(super) working_started_at_ms: Option<u64>,
    pub(super) working_tick: u64,
    pub(super) working_last_tick_at_ms: Option<u64>,
    pub(super) working_detail: Option<String>,
    pub(super) stream_controller: StreamController,
    pub(super) reasoning_stream_active: bool,
    pub(super) plan_stream_controller: Option<PlanStreamController>,
    pub(super) stream_chunking_policy: AdaptiveChunkingPolicy,
    pub(super) history_render_mode: HistoryRenderMode,
    pub(super) notifications: NotificationManager,
    pub(super) history: HistoryBuffer,
    pub(super) resize_reflow: ResizeReflowState,
    pub(super) resize_reflow_row_cap: usize,
    pub(super) native_scrollback: bool,
    pub(super) rendered_message_count: usize,
    pub(super) rendered_state_cursor: usize,
    pub(super) rendered_state_keys: Vec<String>,
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
        let chat_id = uuid::Uuid::new_v4().to_string();
        let input_queue_owner = Some(LocalInputOwner {
            project_id: project.project_id.clone(),
            chat_id: chat_id.clone(),
        });
        let keymap = KeymapRegistry::default();
        let vim = VimState::new(keymap.vim_mode_enabled());
        let tui_config_path = default_tui_config_path();
        let theme = TuiTheme::default();
        initialize_syntax_theme(&theme, tui_config_path.as_deref());
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
            inbound_event_state: InboundEventState::default(),
            browser_state: BrowserState::default(),
            runtime_snapshot: None,
            composer: ComposerState::new(history_entries),
            keymap,
            vim,
            theme,
            input_queue: InputQueue::new(),
            input_queue_owner,
            local_input_handoffs: HashMap::new(),
            last_chat_by_project: HashMap::new(),
            server_queue_size: 0,
            server_queue_previews: Vec::new(),
            history_path,
            pending_history_save: None,
            history_save_in_flight: false,
            history_failure_notified: false,
            tui_config_path,
            composer_mode: ComposerMode::Chat,
            picker: surfaces::ProjectPickerState::new(Vec::new()),
            modal_picker: None,
            theme_picker_snapshot: None,
            approval_queue: ApprovalQueue::new(),
            ask_questions_form: None,
            pending_manual_ask_questions: None,
            handled_ask_questions_tool_ids: HashSet::new(),
            pending_approval_clears: VecDeque::new(),
            events_pane: EventsPaneState::new(),
            current_project: Some(project),
            chat_id,
            worktree_meta: None,
            pending_worktree_merge: None,
            session_title: None,
            recent_sessions: Vec::new(),
            show_session_header: false,
            model: None,
            mode: None,
            boost_reasoning: false,
            reasoning_effort: None,
            pending_reasoning_rollback: None,
            pending_backtrack_rollback: None,
            pending_model: None,
            pending_mode: None,
            in_flight_send: None,
            pending_send_retry: None,
            session_state: SessionState::Idle,
            subscription_status: SubscriptionStatus::Online,
            daemon_online: true,
            daemon_status: None,
            daemon_base_url: None,
            permission_policy: command_session::PermissionPolicy::default(),
            retry_hint: None,
            model_context_windows: HashMap::new(),
            model_reasoning_caps: HashMap::new(),
            model_settings_caps: HashMap::new(),
            default_context_window_tokens: None,
            thread_params: Value::Object(Map::new()),
            settings_surface: None,
            scroll_offset: 0,
            selected_tool_index: None,
            selected_backtrack_index: None,
            backtrack_target: None,
            backtrack_pending: None,
            last_escape_at: None,
            transcript_overlay: None,
            board_surface: None,
            transcript_overlay_visible_height: None,
            activity_surface: None,
            task_id: None,
            history_surface: None,
            goal_overlay_open: false,
            help_open: false,
            usage: None,
            should_quit: false,
            abort_in_flight: false,
            last_ctrl_c: None,
            working_started_at_ms: None,
            working_tick: 0,
            working_last_tick_at_ms: None,
            working_detail: None,
            stream_controller: StreamController::new(None, std::path::Path::new(".")),
            reasoning_stream_active: false,
            plan_stream_controller: None,
            stream_chunking_policy: AdaptiveChunkingPolicy::default(),
            history_render_mode: HistoryRenderMode::Rich,
            notifications: NotificationManager::default(),
            history: HistoryBuffer::new(),
            resize_reflow: ResizeReflowState::default(),
            resize_reflow_row_cap: resize_reflow_row_cap_from_env(),
            native_scrollback: false,
            rendered_message_count: 0,
            rendered_state_cursor: 0,
            rendered_state_keys: Vec::new(),
        }
    }

    pub(super) fn notice_only(notice: impl Into<String>) -> Self {
        let notice = notice.into();
        let tui_config_path = default_tui_config_path();
        let theme = TuiTheme::default();
        initialize_syntax_theme(&theme, tui_config_path.as_deref());
        Self {
            transcript: vec![TranscriptItem::Notice(notice.clone())],
            transcript_state: notice_transcript_state(notice),
            inbound_event_state: InboundEventState::default(),
            browser_state: BrowserState::default(),
            runtime_snapshot: None,
            composer: ComposerState::new(Vec::new()),
            keymap: KeymapRegistry::default(),
            vim: VimState::new(false),
            theme,
            input_queue: InputQueue::new(),
            input_queue_owner: None,
            local_input_handoffs: HashMap::new(),
            last_chat_by_project: HashMap::new(),
            server_queue_size: 0,
            server_queue_previews: Vec::new(),
            history_path: None,
            pending_history_save: None,
            history_save_in_flight: false,
            history_failure_notified: false,
            tui_config_path,
            composer_mode: ComposerMode::Chat,
            picker: surfaces::ProjectPickerState::new(Vec::new()),
            modal_picker: None,
            theme_picker_snapshot: None,
            approval_queue: ApprovalQueue::new(),
            ask_questions_form: None,
            pending_manual_ask_questions: None,
            handled_ask_questions_tool_ids: HashSet::new(),
            pending_approval_clears: VecDeque::new(),
            events_pane: EventsPaneState::new(),
            current_project: None,
            chat_id: uuid::Uuid::new_v4().to_string(),
            worktree_meta: None,
            pending_worktree_merge: None,
            session_title: None,
            recent_sessions: Vec::new(),
            show_session_header: false,
            model: None,
            mode: None,
            boost_reasoning: false,
            reasoning_effort: None,
            pending_reasoning_rollback: None,
            pending_backtrack_rollback: None,
            pending_model: None,
            pending_mode: None,
            in_flight_send: None,
            pending_send_retry: None,
            session_state: SessionState::Error,
            subscription_status: SubscriptionStatus::Offline,
            daemon_online: false,
            daemon_status: None,
            daemon_base_url: None,
            permission_policy: command_session::PermissionPolicy::default(),
            retry_hint: None,
            model_context_windows: HashMap::new(),
            model_reasoning_caps: HashMap::new(),
            model_settings_caps: HashMap::new(),
            default_context_window_tokens: None,
            thread_params: Value::Object(Map::new()),
            settings_surface: None,
            scroll_offset: 0,
            selected_tool_index: None,
            selected_backtrack_index: None,
            backtrack_target: None,
            backtrack_pending: None,
            last_escape_at: None,
            transcript_overlay: None,
            board_surface: None,
            transcript_overlay_visible_height: None,
            activity_surface: None,
            task_id: None,
            history_surface: None,
            goal_overlay_open: false,
            help_open: false,
            usage: None,
            should_quit: false,
            abort_in_flight: false,
            last_ctrl_c: None,
            working_started_at_ms: None,
            working_tick: 0,
            working_last_tick_at_ms: None,
            working_detail: None,
            stream_controller: StreamController::new(None, std::path::Path::new(".")),
            reasoning_stream_active: false,
            plan_stream_controller: None,
            stream_chunking_policy: AdaptiveChunkingPolicy::default(),
            history_render_mode: HistoryRenderMode::Rich,
            notifications: NotificationManager::default(),
            history: HistoryBuffer::new(),
            resize_reflow: ResizeReflowState::default(),
            resize_reflow_row_cap: resize_reflow_row_cap_from_env(),
            native_scrollback: false,
            rendered_message_count: 0,
            rendered_state_cursor: 0,
            rendered_state_keys: Vec::new(),
        }
    }

    pub fn composer(&self) -> &str {
        self.composer.text()
    }

    pub(crate) fn worktree_meta(&self) -> Option<&crate::sessions::WorktreeMeta> {
        self.worktree_meta.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn pending_worktree_merge(&self) -> Option<&surfaces::WorktreeMergeConfirmation> {
        self.pending_worktree_merge.as_ref()
    }

    pub fn composer_state(&self) -> &ComposerState {
        &self.composer
    }

    pub fn composer_history_search(&self) -> Option<HistorySearchView> {
        self.composer.history_search_view()
    }

    pub fn keymap(&self) -> &KeymapRegistry {
        &self.keymap
    }

    pub fn theme(&self) -> &TuiTheme {
        &self.theme
    }

    pub fn vim_enabled(&self) -> bool {
        self.vim.enabled()
    }

    pub fn vim_mode(&self) -> VimMode {
        self.vim.mode()
    }

    pub fn keymap_help_rows(&self) -> Vec<HelpRow> {
        self.keymap.help_rows()
    }

    pub fn input_queue(&self) -> &InputQueue {
        &self.input_queue
    }

    pub fn server_queue_size(&self) -> usize {
        self.server_queue_size
    }

    pub fn server_queue_previews(&self) -> &[String] {
        &self.server_queue_previews
    }

    pub fn composer_mode(&self) -> ComposerMode {
        self.composer_mode
    }

    pub fn daemon_online(&self) -> bool {
        self.daemon_online
    }

    pub fn daemon_status(&self) -> Option<&DaemonStatus> {
        self.daemon_status.as_ref()
    }

    pub fn daemon_base_url(&self) -> Option<&str> {
        self.daemon_base_url.as_deref()
    }

    pub fn permission_policy(&self) -> session::PermissionPolicy {
        self.permission_policy
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn visible_transcript(&self) -> &[TranscriptItem] {
        &self.transcript
    }

    pub fn history_pending_count(&self) -> usize {
        self.history.pending_cell_count()
    }

    pub fn history_inserted_cell_count(&self) -> usize {
        self.history.inserted_cell_count()
    }

    pub fn history_render_count(&self) -> usize {
        self.history.render_count()
    }

    pub(crate) fn render_transcript_item(
        &mut self,
        item: &TranscriptItem,
        selected: bool,
        content_width: usize,
    ) -> Vec<crate::vendored::terminal_hyperlinks::HyperlinkLine> {
        self.history
            .render_transcript_item(item, selected, content_width)
    }

    pub fn native_scrollback(&self) -> bool {
        self.native_scrollback
    }

    pub fn rendered_message_count(&self) -> usize {
        self.rendered_message_count
    }

    pub fn transcript_state(&self) -> &TranscriptState {
        &self.transcript_state
    }

    pub fn inbound_event_state(&self) -> &InboundEventState {
        &self.inbound_event_state
    }

    pub fn browser_state(&self) -> &BrowserState {
        &self.browser_state
    }

    pub fn runtime_snapshot(&self) -> Option<&RuntimeUpdatedEvent> {
        self.runtime_snapshot.as_ref()
    }

    pub fn stream_has_committable_lines(&self) -> bool {
        self.stream_controller.stable_lines_ready()
            || self
                .plan_stream_controller
                .as_ref()
                .is_some_and(PlanStreamController::stable_lines_ready)
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

    pub fn approval_modal(&self) -> Option<&ApprovalModalState> {
        self.approval_queue.front()
    }

    pub fn ask_questions_form(&self) -> Option<&AskQuestionsForm> {
        self.ask_questions_form.as_ref()
    }

    #[cfg(test)]
    pub fn test_set_ask_questions_form(&mut self, form: AskQuestionsForm) {
        self.ask_questions_form = Some(form);
        self.set_session_state(SessionState::WaitingUserInput);
    }

    #[cfg(test)]
    pub fn test_open_history_surface(&mut self, trajectories: Vec<TrajectoryMeta>) {
        self.open_history_surface(trajectories);
    }

    #[cfg(test)]
    pub(super) fn approval_pending_clear_count(&self) -> usize {
        self.pending_approval_clears.len()
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

    pub fn selected_backtrack_index(&self) -> Option<usize> {
        self.selected_backtrack_index
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn composer_height(&self, width: u16) -> u16 {
        let text_width = width
            .saturating_sub(crate::ui_consts::LIVE_PREFIX_COLS + 2)
            .max(1);
        self.composer.height(text_width, 8) + 1 + self.queue_preview_height()
    }

    pub fn queue_preview_height(&self) -> u16 {
        if self.input_queue.is_empty() && self.server_queue_size == 0 {
            0
        } else {
            1
        }
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

    pub fn session_title(&self) -> Option<&str> {
        self.session_title.as_deref()
    }

    pub fn session_tabs(&self) -> Vec<SessionTab> {
        self.recent_sessions
            .iter()
            .cloned()
            .map(|item| session_tab_from_picker_item(item, &self.chat_id))
            .collect()
    }

    pub fn session_header_subtitle(&self) -> String {
        session_header_subtitle(self.model(), self.current_project_root().as_deref())
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    pub fn reasoning_effort_label(&self) -> &str {
        self.reasoning_effort
            .as_deref()
            .unwrap_or(if self.boost_reasoning { "on" } else { "off" })
    }

    pub fn session_state(&self) -> SessionState {
        self.session_state
    }

    pub fn terminal_title(&self) -> String {
        terminal_title(
            self.current_project().map(|project| project.slug.as_str()),
            self.session_state.as_str(),
        )
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

    pub fn working_elapsed_ms(&self) -> u64 {
        self.working_started_at_ms
            .filter(|_| self.session_state.shows_working_indicator())
            .map(|started| now_ms().saturating_sub(started))
            .unwrap_or_default()
    }

    pub fn working_tick(&self) -> u64 {
        self.working_tick
    }

    pub fn working_detail(&self) -> Option<&str> {
        self.working_detail.as_deref()
    }

    pub(super) fn current_project_root(&self) -> Option<PathBuf> {
        self.current_project
            .as_ref()
            .map(|project| project.root.clone())
    }

    pub(super) fn is_chat_active(&self) -> bool {
        matches!(
            self.session_state,
            SessionState::Generating
                | SessionState::ExecutingTools
                | SessionState::Paused
                | SessionState::WaitingIde
                | SessionState::WaitingUserInput
        )
    }
    pub fn modal_picker(&self) -> Option<&PickerState> {
        self.modal_picker.as_ref()
    }

    pub fn transcript_overlay(&self) -> Option<&PagerOverlay> {
        self.transcript_overlay.as_ref()
    }

    pub(crate) fn task_board_surface(&self) -> Option<&surfaces::board::BoardSurface> {
        self.board_surface.as_ref()
    }

    pub(crate) fn task_board_discoverable(&self) -> bool {
        surfaces::board::task_board_enabled()
            && matches!(self.mode.as_deref(), Some("task_planner" | "task_agent"))
    }

    pub fn history_surface(&self) -> Option<&surfaces::HistorySurface> {
        self.history_surface.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_only_initializes_offline_error_state() {
        let app = App::notice_only("startup failed");

        assert_eq!(app.session_state(), SessionState::Error);
        assert_eq!(app.subscription_status(), SubscriptionStatus::Offline);
        assert!(!app.daemon_online());
        assert!(matches!(
            app.visible_transcript(),
            [TranscriptItem::Notice(text)] if text == "startup failed"
        ));
    }

    #[test]
    fn task_modes_are_board_discoverable_when_surfaces_are_enabled() {
        let previous = std::env::var_os("REFACT_TUI_SURFACES");
        std::env::set_var("REFACT_TUI_SURFACES", "1");
        let mut app = App::notice_only("test");
        app.mode = Some("task_planner".to_string());
        assert!(app.task_board_discoverable());
        app.mode = Some("task_agent".to_string());
        assert!(app.task_board_discoverable());
        app.mode = Some("agent".to_string());
        assert!(!app.task_board_discoverable());
        match previous {
            Some(value) => std::env::set_var("REFACT_TUI_SURFACES", value),
            None => std::env::remove_var("REFACT_TUI_SURFACES"),
        }
    }
}
