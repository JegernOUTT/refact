use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::{json, Map, Value};

use crate::approvals::{ApprovalModalState, ApprovalQueue};
use crate::ask_questions::{
    AskQuestionType, AskQuestionsForm, AskQuestionsOutcome, AskQuestionsRequest,
};
use crate::client::{
    BrowserContextDecision, ChatEvent, DaemonStatus, CompetitorImportInfoResponse,
    CompetitorImportRunResponse, GoalBudget, GoalControlAction, HooksResponse,
    KnowledgeGraphResponse, McpViewData, OpenProjectResponse, ProjectEntry, ProviderListResponse,
    ProviderOAuthLogoutResponse, SlashCommandsListResponse, TaskBoardViewData, ToolDecision,
    WorkerInfo,
};
use crate::commands::{command_by_name, session, workflow, CommandAction, InfoTopic, LocalToggle};
use crate::composer::queue::{InputQueue, QueuedInput, INPUT_QUEUE_CAPACITY};
use crate::composer::{load_history, save_history, ComposerState, EnterDecision, HistorySearchView};
use crate::events_pane::{DaemonEventRecord, EventsPaneState};
use crate::history::cells::HistoryRenderMode;
use crate::history::{
    insert_history, resize_reflow_row_cap_from_env, HistoryBuffer, HistoryInsertion,
    ResizeReflowState, RESIZE_REFLOW_PENDING_CELL_CAP,
};
use crate::keymap::{
    HelpRow, KeyAction, KeyContext, KeyDispatch, KeymapRegistry, VimEffect, VimMode, VimState,
};
use crate::notifications::{NotificationConfig, NotificationKind, NotificationManager};
use crate::overlay::{PagerAction, PagerOverlay};
use crate::pickers::{
    file_mention_items_from_completions, ModePickerItem, PickerItem, PickerKind, PickerState,
};
use crate::protocol::{
    DeltaOp, InboundEventState, SseEvent, TranscriptMessage, TranscriptRole, TranscriptState,
};
use crate::read_only_views::ReadOnlyView;
use crate::render::highlight;
use crate::sessions::{
    last_branch_message_id, session_tab_from_picker_item, SessionTab, TrajectoryMeta,
};
use crate::streaming::{
    run_commit_tick, AdaptiveChunkingPolicy, CommitTickScope, PlanStreamController,
    StreamController,
};
use crate::terminal::{terminal_title, TerminalSession};
use crate::text_safety::{sanitize_tool_inline, sanitize_tool_text, truncate_graphemes};
use crate::theme::TuiTheme;
use crate::tools::{
    now_ms, ToolCard, ToolStatus, MAX_SUBCHAT_ATTACHED_FILES, MAX_SUBCHAT_DEPTH,
    MAX_SUBCHAT_PROGRESS_CHARS,
};

mod browser;
mod chat_events;
mod command_results;
mod goal;
mod input;
mod runtime;
#[path = "app/session.rs"]
mod session_lifecycle;
mod state;
pub(crate) mod surfaces;
mod transcript;
mod workers;
use self::command_results::CommandOrigin;
pub use self::goal::GoalCommandKind;
pub use self::session_lifecycle::{ClipboardCopySource, SessionState, SubscriptionStatus, UsageSummary};
pub use self::state::{
    App, BrowserContextDecisionOptions, BrowserContextPromptState, BrowserState,
    ClientMessageCorrelation, ComposerMode,
};
pub use surfaces::ProjectPickerState;
pub use transcript::TranscriptItem;
use self::session_lifecycle::{resolve_chat_model_id, ReasoningModelCaps};
use self::state::{
    BacktrackRollback, BacktrackTarget, HistorySaveRequest, InFlightSend, LocalInputHandoff,
    LocalInputOwner, PendingApprovalClear, PendingReasoningRollback, PendingSendRetry,
    ReasoningStateSnapshot, ToolDecisionRollback,
};
use chat_events::SubagentSummary;
#[cfg(test)]
use transcript::line_to_plain_string;
use transcript::{current_goal_cell_data, current_plan_cell_data, session_header_subtitle};
pub use runtime::run;

#[cfg(test)]
use crate::client::{ChatSeqDecision, ChatSeqTracker, DaemonClient};
#[cfg(test)]
use std::process::Command;
#[cfg(test)]
use tokio::sync::mpsc;

#[cfg(test)]
use runtime::{
    edit_text_with_editor_command, execute_history_save_request,
    external_editor_command_from_values, load_git_diff, parse_editor_command, run_action,
    DaemonEventSubscription, RuntimeEvent, SubscriptionManager, SubscriptionTask,
};

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
const CHAT_RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const CHAT_RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(4);
const CHAT_MAX_RECONNECT_ATTEMPTS: u32 = 8;
const DAEMON_EVENTS_RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const DAEMON_EVENTS_RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(5);
const DAEMON_EVENTS_MAX_RECONNECT_ATTEMPTS: u32 = 8;
const ABORT_BEFORE_QUIT_TIMEOUT: Duration = Duration::from_secs(1);
const LIVE_TRANSCRIPT_ITEM_LIMIT: usize = 10_000;
const LIVE_TRANSCRIPT_RETENTION_NOTICE: &str =
    "Older live transcript items dropped after reaching 10000 live items";
const WORKING_ANIMATION_INTERVAL: Duration = Duration::from_millis(100);
const ASSISTANT_STREAM_RESERVED_COLS: u16 = 4;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorCommand {
    program: String,
    args: Vec<String>,
}

#[derive(Debug, Clone)]
enum CommandContextTag {
    SendMessage {
        prompt: String,
        params: Value,
        correlation: ClientMessageCorrelation,
    },
    Reasoning {
        previous: ReasoningStateSnapshot,
    },
    RetryFromIndex {
        rollback: Option<BacktrackRollback>,
    },
    ToolDecisions {
        client_request_id: String,
        rollback: ToolDecisionRollback,
    },
    Abort {
        origin: CommandOrigin,
    },
    BrowserContextDecision {
        origin: CommandOrigin,
        prompt: BrowserContextPromptState,
    },
    Rename {
        origin: CommandOrigin,
        title: String,
    },
    Fork {
        origin: CommandOrigin,
        target_chat_id: String,
        title: Option<String>,
    },
    Archive {
        origin: CommandOrigin,
        chat_id: String,
    },
    Other,
}

impl App {
    fn apply_tui_config_content(&mut self, content: &str) {
        match KeymapRegistry::from_config_file_content(Some(content)) {
            Ok(keymap) => {
                self.vim.set_enabled(keymap.vim_mode_enabled());
                for warning in keymap.warnings() {
                    self.add_notice(format!("TUI keymap warning: {warning}"));
                }
                self.keymap = keymap;
            }
            Err(error) => self.add_notice(format!("Failed to load TUI keymap config: {error}")),
        }
        let theme_home = self.tui_theme_home();
        match TuiTheme::from_config_file_content_with_custom_dir(
            Some(content),
            theme_home.as_deref(),
        ) {
            Ok(theme) => self.set_theme(theme),
            Err(error) => self.add_notice(format!("Failed to load TUI theme config: {error}")),
        }
        match NotificationConfig::from_config_file_content(Some(content)) {
            Ok(config) => self.notifications.set_config(config),
            Err(error) => {
                self.add_notice(format!("Failed to load TUI notification config: {error}"))
            }
        }
    }

    fn set_session_state(&mut self, state: SessionState) {
        let was_working = self.session_state.shows_working_indicator();
        let is_working = state.shows_working_indicator();
        self.session_state = state;
        match (was_working, is_working) {
            (false, true) => {
                self.working_started_at_ms = Some(now_ms());
                self.working_tick = 0;
                self.working_last_tick_at_ms = None;
                self.working_detail = self.latest_tool_detail();
            }
            (true, false) => self.clear_working_indicator(),
            _ => {}
        }
    }

    fn clear_working_indicator(&mut self) {
        self.working_started_at_ms = None;
        self.working_tick = 0;
        self.working_last_tick_at_ms = None;
        self.working_detail = None;
    }

    fn tick_working_indicator(&mut self) {
        if !self.session_state.shows_working_indicator() {
            return;
        }
        if self.working_started_at_ms.is_none() {
            self.working_started_at_ms = Some(now_ms());
        }
        let now = now_ms();
        if self.working_last_tick_at_ms.is_some_and(|last| {
            now.saturating_sub(last) < WORKING_ANIMATION_INTERVAL.as_millis() as u64
        }) {
            return;
        }
        self.working_last_tick_at_ms = Some(now);
        self.working_tick = self.working_tick.wrapping_add(1);
    }

    fn set_working_detail(&mut self, detail: String) {
        if self.session_state.shows_working_indicator() && !detail.is_empty() {
            self.working_detail = Some(detail);
        }
    }

    fn clear_stream_controllers(&mut self) {
        self.stream_controller.clear();
        self.reasoning_stream_active = false;
        self.plan_stream_controller = None;
        self.stream_chunking_policy.reset();
    }

    fn latest_tool_detail(&self) -> Option<String> {
        self.transcript.iter().rev().find_map(|item| match item {
            TranscriptItem::Tool(card) if card.status.is_active() => Some(card.summary()),
            _ => None,
        })
    }

    pub fn pending_history_insertions(&mut self, width: u16) -> Vec<HistoryInsertion> {
        self.history
            .drain_pending_capped(width, RESIZE_REFLOW_PENDING_CELL_CAP)
    }

    fn resize_reflow_insertions(&mut self, width: u16) -> Vec<HistoryInsertion> {
        self.history
            .reflow_insertions(width, self.resize_reflow_row_cap)
    }

    fn note_terminal_resize_width(&mut self, width: u16) -> bool {
        self.update_stream_width_for_terminal(width);
        if !self.native_scrollback {
            self.resize_reflow.clear();
            return false;
        }
        let width_change = self.resize_reflow.note_width(width);
        if !width_change.changed || self.history.source_cell_count() == 0 {
            return false;
        }
        if self.should_mark_resize_reflow_as_stream_time() {
            self.resize_reflow.mark_resize_requested_during_stream();
        }
        self.resize_reflow.schedule_debounced(Some(width));
        true
    }

    fn update_stream_width_for_terminal(&mut self, width: u16) {
        let should_sync_assistant =
            self.stream_controller.has_live_tail() || self.stream_controller.stable_lines_ready();
        let should_sync_plan = self
            .plan_stream_controller
            .as_ref()
            .is_some_and(|controller| {
                controller.has_live_tail() || controller.stable_lines_ready()
            });
        let should_sync = should_sync_assistant || should_sync_plan;
        if self.session_state != SessionState::Generating && !should_sync {
            return;
        }
        self.stream_controller
            .set_width(assistant_stream_width(width));
        if let Some(controller) = &mut self.plan_stream_controller {
            controller.set_width(assistant_stream_width(width));
        }
        if should_sync {
            if should_sync_assistant && self.native_scrollback {
                self.sync_assistant_stream_tail_item();
            } else if should_sync_assistant {
                self.sync_assistant_stream_item();
            }
            if should_sync_plan {
                self.sync_plan_stream_tail_item();
            }
        }
    }

    fn note_terminal_height_resize(&mut self) -> bool {
        if !self.native_scrollback || self.history.source_cell_count() == 0 {
            return false;
        }
        if self.should_mark_resize_reflow_as_stream_time() {
            self.resize_reflow.mark_resize_requested_during_stream();
        }
        self.resize_reflow.schedule_debounced(None);
        true
    }

    fn resize_reflow_is_due(&self) -> bool {
        self.resize_reflow.pending_is_due(Instant::now())
    }

    fn resize_reflow_delay(&self) -> Option<Duration> {
        self.resize_reflow
            .pending_until()
            .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
    }

    fn finish_resize_reflow(&mut self, width: u16, ran_during_stream: bool) {
        self.resize_reflow.clear_pending_reflow();
        self.resize_reflow.mark_reflowed_width(width);
        if ran_during_stream {
            self.resize_reflow.mark_ran_during_stream();
        }
    }

    fn schedule_final_stream_resize_reflow(&mut self) {
        if self.native_scrollback
            && self.history.source_cell_count() > 0
            && self.resize_reflow.take_stream_finish_reflow_needed()
        {
            self.resize_reflow.schedule_immediate();
        }
    }

    fn should_mark_resize_reflow_as_stream_time(&self) -> bool {
        self.session_state.shows_working_indicator()
            || self
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::Assistant(_)))
    }

    pub fn set_native_scrollback(&mut self, enabled: bool) {
        if enabled && !self.native_scrollback {
            let old_selected = self.selected_tool_index;
            let old_items = std::mem::take(&mut self.transcript);
            self.selected_tool_index = None;
            for (old_idx, item) in old_items.into_iter().enumerate() {
                if item.keeps_live() {
                    let new_idx = self.transcript.len();
                    self.push_live_item(item);
                    if old_selected == Some(old_idx) {
                        self.selected_tool_index = Some(new_idx);
                    }
                } else {
                    self.history.enqueue(item);
                }
            }
            self.mark_rendered_state_from_messages();
        }
        self.native_scrollback = enabled;
    }

    pub fn begin_frame_render(&mut self) {
        self.rendered_message_count = 0;
    }

    pub fn note_rendered_messages(&mut self, count: usize) {
        self.rendered_message_count = self.rendered_message_count.saturating_add(count);
    }

    fn submit_structured_prompt(&mut self, prompt: &str) -> AppAction {
        self.composer.set_text(prompt);
        self.submit_composer().unwrap_or(AppAction::None)
    }

    fn show_current_plan(&mut self) -> AppAction {
        self.composer.clear();
        match current_plan_cell_data(self.transcript_state.messages()) {
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

    pub fn execute_command_name(&mut self, name: &str) -> AppAction {
        let (name, args) = split_command_name_and_args(name);
        let Some(command) = command_by_name(name) else {
            self.add_notice(format!("/{name} is not registered"));
            return AppAction::None;
        };
        match command.action {
            CommandAction::BackendCommand { command } => {
                if command == "stop" && self.is_chat_active() {
                    self.cancel_queue_edit();
                    self.abort_in_flight = true;
                    self.add_notice("Cancel requested");
                    AppAction::Abort
                } else {
                    self.composer.clear();
                    self.add_notice(format!(
                        "/{command} is only available while a response is running"
                    ));
                    AppAction::None
                }
            }
            CommandAction::OpenPicker { picker } => self.execute_picker_command(picker),
            CommandAction::LocalToggle { toggle } => self.execute_local_toggle(toggle, args),
            CommandAction::ShowInfo { topic } => {
                self.show_info_topic(topic);
                AppAction::None
            }
            CommandAction::Session { command } => self.execute_session_command(command, args),
            CommandAction::Workflow { command } => {
                if command == workflow::WorkflowCommand::ShowGoal {
                    self.execute_goal_command(args, self.goal_presentation().is_none())
                } else {
                    self.execute_workflow_command(command)
                }
            }
            CommandAction::Misc { command } => self.execute_misc_command(command, args),
            CommandAction::Unavailable { reason } => {
                self.composer.clear();
                self.add_notice(format!("/{name} unavailable: {reason}"));
                AppAction::None
            }
        }
    }

    fn execute_workflow_command(&mut self, command: workflow::WorkflowCommand) -> AppAction {
        match command {
            workflow::WorkflowCommand::ShowPlan => self.show_current_plan(),
            workflow::WorkflowCommand::ShowGoal => {
                self.execute_goal_command("", self.goal_presentation().is_none())
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

    fn execute_local_toggle(&mut self, toggle: LocalToggle, _args: &str) -> AppAction {
        match toggle {
            LocalToggle::ClearTranscript => {
                self.composer.clear();
                self.replace_with_notice("Transcript cleared".to_string());
                AppAction::None
            }
            LocalToggle::Events => {
                self.composer.clear();
                self.events_pane.toggle();
                AppAction::RefreshWorkers
            }
            LocalToggle::Quit => self.quit_action(),
        }
    }

    fn show_info_topic(&mut self, topic: InfoTopic) {
        self.composer.clear();
        match topic {
            InfoTopic::Help => self.help_open = true,
            InfoTopic::Status => {
                self.show_status_card();
            }
        }
    }

    fn show_debug_config_card(&mut self) {
        let config_path = self
            .tui_config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        self.push_history_item(TranscriptItem::Info(vec![
            "TUI debug config".to_string(),
            format!("Config: {config_path}"),
            format!("Theme: {}", self.theme.name()),
            format!(
                "Vim: {}",
                if self.vim.enabled() {
                    self.vim.mode().label()
                } else {
                    "off"
                }
            ),
            format!("Notifications: {}", self.notification_status_label()),
            format!(
                "Commands: {} registered",
                crate::commands::command_registry().len()
            ),
        ]));
    }

    fn show_subagents_card(&mut self) {
        let summaries = self.subagent_summaries();
        if summaries.is_empty() {
            self.push_history_item(TranscriptItem::Info(vec![
                "Subagents".to_string(),
                "No active subagents".to_string(),
            ]));
            return;
        }
        let mut lines = vec!["Subagents".to_string()];
        for summary in summaries.iter().take(MAX_SUBCHAT_DEPTH) {
            lines.push(summary.detail());
        }
        if summaries.len() > MAX_SUBCHAT_DEPTH {
            lines.push(format!(
                "… {} more subagents",
                summaries.len() - MAX_SUBCHAT_DEPTH
            ));
        }
        self.push_history_item(TranscriptItem::Info(lines));
    }

    fn subagent_summaries(&self) -> Vec<SubagentSummary> {
        let finished_tool_ids = self
            .transcript_state
            .messages()
            .iter()
            .filter(|message| message.role.is_tool_result())
            .filter_map(|message| message.tool_call_id.clone())
            .collect::<Vec<_>>();
        let mut summaries = Vec::<SubagentSummary>::new();
        for message in self.transcript_state.messages() {
            if message.role != TranscriptRole::Assistant {
                continue;
            }
            for tool in &message.tool_calls {
                let card = ToolCard::from_tool_call(tool);
                if card.subchat_log.is_empty() && card.attached_files.is_empty() {
                    continue;
                }
                let active = card.subchat_active && !finished_tool_ids.contains(&card.id);
                summaries.push(SubagentSummary {
                    tool_call_id: card.id,
                    tool_name: card.name,
                    progress: card.subchat_log.last().cloned(),
                    attached_files: card.attached_files.len(),
                    depth: card.subchat_depth,
                    active,
                    truncated: card.subchat_truncated,
                });
            }
        }
        for item in &self.transcript {
            let TranscriptItem::Tool(card) = item else {
                continue;
            };
            if card.subchat_log.is_empty() && card.attached_files.is_empty() {
                continue;
            }
            if summaries
                .iter()
                .any(|summary| summary.tool_call_id == card.id)
            {
                continue;
            }
            summaries.push(SubagentSummary {
                tool_call_id: card.id.clone(),
                tool_name: card.name.clone(),
                progress: card.subchat_log.last().cloned(),
                attached_files: card.attached_files.len(),
                depth: card.subchat_depth,
                active: card.subchat_active && card.status.is_active(),
                truncated: card.subchat_truncated,
            });
        }
        summaries
    }

    fn apply_daemon_status(&mut self, status: DaemonStatus, base_url: String) {
        self.daemon_online = true;
        self.daemon_status = Some(status);
        self.daemon_base_url = Some(base_url);
        self.show_status_card();
    }

    fn record_daemon_status_error(&mut self, error: &str) {
        self.daemon_online = false;
        self.retry_hint = retry_hint_from_message(error);
        self.add_notice(format!("Failed to load daemon status: {error}"));
    }

    fn submit_composer(&mut self) -> Option<AppAction> {
        if self.current_project.is_none() {
            return None;
        }
        if self.input_queue.is_editing() {
            return self.finish_queue_edit();
        }
        let mut prompt = self.composer.submit_text()?;
        if let Some(request) = self.pending_manual_ask_questions.take() {
            if !prompt.starts_with(&format!("[QA:{}]", request.tool_call_id)) {
                prompt = request.format_manual_reply(&prompt);
            }
            self.handled_ask_questions_tool_ids
                .insert(request.tool_call_id.clone());
        }
        self.persist_history();
        if self.backtrack_pending.is_some() {
            return Some(self.start_backtrack_turn(prompt));
        }
        let (params, correlation) = self.take_submit_params(&prompt);
        if self.is_chat_active() && self.session_state != SessionState::WaitingUserInput {
            self.enqueue_input(prompt, params);
            Some(AppAction::None)
        } else {
            Some(self.start_prompt_turn(prompt, params, correlation))
        }
    }

    fn start_backtrack_turn(&mut self, prompt: String) -> AppAction {
        let Some(target) = self.backtrack_pending.take() else {
            return AppAction::None;
        };
        self.in_flight_send = None;
        self.pending_send_retry = None;
        if !self
            .transcript_state
            .messages()
            .get(target.index)
            .is_some_and(|message| target.matches(message))
        {
            self.add_notice("Backtrack target changed; open backtrack again");
            return AppAction::None;
        }
        self.pending_backtrack_rollback = Some(BacktrackRollback {
            transcript_state: self.transcript_state.clone(),
            session_state: self.session_state,
            usage: self.usage,
            selected_backtrack_index: self.selected_backtrack_index,
            backtrack_target: self.backtrack_target.clone(),
            backtrack_pending: Some(target.clone()),
            last_escape_at: self.last_escape_at,
            prompt: prompt.clone(),
        });
        self.transcript_state.truncate_messages(target.index);
        self.transcript_state.push_user_message(prompt.clone());
        self.transcript_state.start_assistant(None);
        self.rebuild_render_transcript_from_state();
        self.set_session_state(SessionState::Generating);
        self.clear_stream_controllers();
        self.usage = None;
        self.retry_hint = None;
        self.clear_backtrack_selection();
        AppAction::RetryFromIndex {
            index: target.index,
            content: Value::String(prompt),
        }
    }

    fn start_prompt_turn(
        &mut self,
        prompt: String,
        params: Value,
        correlation: Option<ClientMessageCorrelation>,
    ) -> AppAction {
        let correlation = correlation.unwrap_or_else(ClientMessageCorrelation::new);
        self.in_flight_send = Some(InFlightSend {
            correlation: correlation.clone(),
            accepted: false,
        });
        self.cancel_backtrack();
        self.clear_active_ask_questions();
        self.transcript_state
            .push_optimistic_user_message(prompt.clone(), correlation.client_message_id.clone());
        self.transcript_state.start_assistant(None);
        self.rebuild_render_transcript_from_state();
        self.set_session_state(SessionState::Generating);
        self.clear_stream_controllers();
        self.usage = None;
        self.retry_hint = None;
        AppAction::SendMessage {
            prompt,
            params,
            correlation,
        }
    }

    fn take_submit_params(&mut self, prompt: &str) -> (Value, Option<ClientMessageCorrelation>) {
        if self
            .pending_send_retry
            .as_ref()
            .is_some_and(|retry| retry.prompt == prompt)
        {
            return self
                .pending_send_retry
                .take()
                .map(|retry| (retry.params, Some(retry.correlation)))
                .unwrap_or_else(|| (Value::Object(Map::new()), None));
        }
        self.pending_send_retry = None;
        (self.take_pending_params(), None)
    }

    fn dispatch_next_queued_input(&mut self) -> AppAction {
        if !self.input_queue_matches_current_session() {
            return AppAction::None;
        }
        let Some(QueuedInput { text, params, .. }) = self.input_queue.pop_next_ready() else {
            return AppAction::None;
        };
        self.start_prompt_turn(text, params, None)
    }

    fn enqueue_input(&mut self, prompt: String, params: Value) {
        if self.input_queue.enqueue(prompt, params) {
            self.input_queue.clear_selection();
        } else {
            self.add_notice(format!(
                "Warning: input queue is full (maximum {INPUT_QUEUE_CAPACITY} prompts); prompt was not queued"
            ));
        }
    }

    fn submit_ask_questions_reply(&mut self, prompt: String) -> AppAction {
        let (params, correlation) = self.take_submit_params(&prompt);
        self.start_prompt_turn(prompt, params, correlation)
    }

    fn set_params_context(&mut self, patch: &Value) -> CommandContextTag {
        match self.pending_reasoning_rollback.take() {
            Some(pending) if pending.patch == *patch => CommandContextTag::Reasoning {
                previous: pending.previous,
            },
            Some(pending) => {
                self.pending_reasoning_rollback = Some(pending);
                CommandContextTag::Other
            }
            None => CommandContextTag::Other,
        }
    }

    fn begin_queue_edit(&mut self) -> AppAction {
        let draft = self.composer.text().to_string();
        match self.input_queue.begin_edit_selected(draft) {
            Some(text) => {
                self.composer.set_text(text);
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    fn finish_queue_edit(&mut self) -> Option<AppAction> {
        let text = self.composer.submit_text()?;
        let draft = self.input_queue.finish_edit(text).unwrap_or_default();
        self.composer.set_text(draft);
        if self.is_chat_active() {
            Some(AppAction::None)
        } else {
            Some(self.dispatch_next_queued_input())
        }
    }

    fn cancel_queue_edit(&mut self) -> bool {
        let Some(draft) = self.input_queue.cancel_edit() else {
            return false;
        };
        self.composer.set_text(draft);
        true
    }

    fn remove_selected_queue_item(&mut self) -> AppAction {
        self.input_queue.remove_selected();
        AppAction::None
    }

    fn handle_idle_escape(&mut self) -> AppAction {
        if !self.composer.is_empty() {
            return AppAction::None;
        }
        if self.selected_backtrack_index.is_some() {
            self.select_previous_user_message();
            return AppAction::None;
        }
        let now = Instant::now();
        if self
            .last_escape_at
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(900))
        {
            self.select_previous_user_message();
            self.last_escape_at = None;
        } else {
            self.last_escape_at = Some(now);
        }
        AppAction::None
    }

    fn select_previous_user_message(&mut self) {
        let indexes = self.user_message_indexes();
        if indexes.is_empty() {
            self.add_notice("No previous user message to backtrack");
            self.clear_backtrack_selection();
            return;
        }
        let next = match self.selected_backtrack_index {
            Some(current) => indexes
                .iter()
                .copied()
                .rev()
                .find(|idx| *idx < current)
                .unwrap_or(current),
            None => *indexes.last().expect("indexes checked"),
        };
        self.selected_backtrack_index = Some(next);
        self.backtrack_target = self.backtrack_target_at(next);
        self.input_queue.clear_selection();
        self.selected_tool_index = None;
    }

    fn accept_backtrack_selection(&mut self) -> AppAction {
        let Some(index) = self.selected_backtrack_index else {
            return AppAction::None;
        };
        let Some(target) = self.backtrack_target_at(index) else {
            self.clear_backtrack_selection();
            self.add_notice("Backtrack target is no longer available");
            return AppAction::None;
        };
        self.composer.set_text(target.content.clone());
        self.backtrack_target = Some(target.clone());
        self.backtrack_pending = Some(target);
        AppAction::None
    }

    fn user_message_indexes(&self) -> Vec<usize> {
        self.transcript_state
            .messages()
            .iter()
            .enumerate()
            .filter_map(|(idx, message)| {
                (message.role == TranscriptRole::User && !message.content.trim().is_empty())
                    .then_some(idx)
            })
            .collect()
    }

    fn backtrack_target_at(&self, index: usize) -> Option<BacktrackTarget> {
        let message = self.transcript_state.messages().get(index)?;
        if message.role != TranscriptRole::User || message.content.trim().is_empty() {
            return None;
        }
        Some(BacktrackTarget {
            index,
            message_id: message.message_id.clone(),
            content: message.content.clone(),
        })
    }

    fn clear_backtrack_selection(&mut self) {
        self.selected_backtrack_index = None;
        self.backtrack_target = None;
        self.last_escape_at = None;
    }

    fn cancel_backtrack(&mut self) {
        self.clear_backtrack_selection();
        self.backtrack_pending = None;
        self.pending_backtrack_rollback = None;
    }

    fn restore_backtrack_rollback(&mut self, rollback: BacktrackRollback) {
        self.transcript_state = rollback.transcript_state;
        self.set_session_state(rollback.session_state);
        self.usage = rollback.usage;
        self.selected_backtrack_index = rollback.selected_backtrack_index;
        self.backtrack_target = rollback.backtrack_target;
        self.backtrack_pending = rollback.backtrack_pending;
        self.last_escape_at = rollback.last_escape_at;
        self.composer.set_text(rollback.prompt);
        self.clear_stream_controllers();
        self.rebuild_render_transcript_from_state();
    }

    fn persist_history(&mut self) {
        if let Some(path) = &self.history_path {
            self.pending_history_save = Some(HistorySaveRequest {
                path: path.clone(),
                entries: self.composer.history_entries().to_vec(),
            });
        }
    }

    fn take_pending_history_save(&mut self) -> Option<HistorySaveRequest> {
        if self.history_save_in_flight {
            return None;
        }
        let request = self.pending_history_save.take()?;
        self.history_save_in_flight = true;
        Some(request)
    }

    fn handle_history_save_result(&mut self, result: Result<(), String>) {
        self.history_save_in_flight = false;
        match result {
            Ok(()) => self.history_failure_notified = false,
            Err(error) if !self.history_failure_notified => {
                self.history_failure_notified = true;
                self.add_notice(error);
            }
            Err(_) => {}
        }
    }

    fn set_workers(&mut self, workers: Vec<WorkerInfo>) {
        self.update_current_worker_from_list(&workers);
        self.events_pane.set_workers(workers);
        self.refresh_activity_surface();
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

    fn record_chat_disconnected(&mut self, message: &str, unreachable: bool, auth_stale: bool) {
        if auth_stale {
            self.record_subscription_offline(
                "SSE authentication is stale; refresh daemon credentials, then retry the subscription",
            );
        } else if unreachable || worker_waking_message(message) {
            self.subscription_status = SubscriptionStatus::Waking;
            self.daemon_online = true;
        } else {
            self.subscription_status = SubscriptionStatus::Offline;
            self.daemon_online = false;
        }
        if !auth_stale {
            self.retry_hint = retry_hint_from_message(message);
        }
    }

    fn record_subscription_exhausted(&mut self) {
        self.record_subscription_offline("SSE reconnect retries exhausted; retry the subscription");
    }

    fn record_subscription_offline(&mut self, reason: &str) {
        self.subscription_status = SubscriptionStatus::Offline;
        self.daemon_online = false;
        self.set_session_state(SessionState::Error);
        self.retry_hint = Some(reason.to_string());
    }

    fn handle_daemon_events_disconnected(&mut self, message: String, retrying: bool) {
        if retrying {
            self.add_notice(format!(
                "Daemon events disconnected: {message}; reconnecting with bounded backoff…"
            ));
        } else {
            self.add_notice(format!(
                "Daemon events disconnected: {message}; reconnect limit reached"
            ));
        }
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
        let Some(project) = self.current_project.as_mut() else {
            return;
        };
        let Some(state) = workers::update_current_worker_from_event(project, event) else {
            return;
        };
        match state {
            "starting" => self.subscription_status = SubscriptionStatus::Waking,
            "ready" if self.subscription_status == SubscriptionStatus::Waking => {
                self.subscription_status = SubscriptionStatus::Online;
            }
            _ => {}
        }
    }

    fn quit_action(&mut self) -> AppAction {
        let abort_active = self.is_chat_active();
        if abort_active {
            self.abort_in_flight = true;
        } else {
            self.clear_active_ask_questions();
            self.should_quit = true;
        }
        AppAction::Quit { abort_active }
    }

    #[cfg(test)]
    pub fn test_set_approval(&mut self, modal: ApprovalModalState) {
        self.clear_approvals();
        self.enqueue_approval(modal);
        self.set_session_state(SessionState::Paused);
    }

    #[cfg(test)]
    pub fn test_push_tool(&mut self, card: ToolCard) {
        self.push_live_item(TranscriptItem::Tool(card));
        self.selected_tool_index = Some(self.transcript.len() - 1);
    }

    #[cfg(test)]
    pub fn test_push_history_item(&mut self, item: TranscriptItem) {
        self.push_history_item(item);
    }

    #[cfg(test)]
    pub fn test_set_history_items(&mut self, items: Vec<TranscriptItem>) {
        self.transcript = items;
    }

    #[cfg(test)]
    pub fn test_set_daemon_status(&mut self, status: DaemonStatus, base_url: impl Into<String>) {
        self.daemon_online = true;
        self.daemon_status = Some(status);
        self.daemon_base_url = Some(base_url.into());
    }

    #[cfg(test)]
    pub fn test_apply_tui_config_content(&mut self, content: &str) {
        self.apply_tui_config_content(content);
    }

    #[cfg(test)]
    pub fn test_take_pending_notifications(&mut self) -> Vec<Vec<u8>> {
        self.take_pending_notifications()
    }

    #[cfg(test)]
    pub fn test_set_terminal_focus(&mut self, focused: bool) {
        self.set_terminal_focus(focused);
    }

    #[cfg(test)]
    pub fn test_set_notifications_config(&mut self, config: NotificationConfig) {
        self.notifications.set_config(config);
    }

    #[cfg(test)]
    pub fn test_set_keymap(&mut self, keymap: KeymapRegistry) {
        self.vim.set_enabled(keymap.vim_mode_enabled());
        self.keymap = keymap;
    }

    #[cfg(test)]
    pub fn test_set_tui_config_path(&mut self, path: PathBuf) {
        self.tui_config_path = Some(path);
    }

    #[cfg(test)]
    pub fn test_set_composer_text(&mut self, text: &str) {
        self.composer.set_text(text);
    }

    #[cfg(test)]
    pub fn test_set_recent_sessions(&mut self, items: Vec<PickerItem>) {
        self.set_recent_sessions(items);
    }

    #[cfg(test)]
    pub fn test_insert_paste(&mut self, text: &str) {
        self.composer.insert_paste(text);
    }

    #[cfg(test)]
    pub fn test_execute_command_name(&mut self, name: &str) -> AppAction {
        self.execute_command_name(name)
    }

    #[cfg(test)]
    pub fn test_set_thread_params(&mut self, params: Value) {
        self.thread_params = params;
    }

    pub fn test_show_task_board(&mut self, data: TaskBoardViewData) {
        self.show_task_board(data);
    }

    pub fn test_set_task_context(&mut self, task_id: Option<String>, mode: Option<String>) {
        self.task_id = task_id;
        self.mode = mode;
    }

    pub fn test_goal_overlay_open(&self) -> bool {
        self.goal_overlay_open
    }

    pub fn submit_browser_context_decision(
        &mut self,
        options: BrowserContextDecisionOptions,
    ) -> AppAction {
        let Some((decision, prompt)) = self.browser_state.take_context_prompt(options) else {
            return AppAction::None;
        };
        AppAction::SendBrowserContextDecision { decision, prompt }
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
    LoadReadOnlyView {
        view: ReadOnlyView,
    },
    LoadTaskBoard {
        task_id: Option<String>,
    },
    LoadProviderLogoutChoices,
    ProviderOAuthLogout {
        provider: String,
    },
    LoadCompetitorImportSources,
    RunCompetitorImport {
        source: Option<String>,
        scope: String,
    },
    LoadSessions,
    RefreshRecentSessions,
    RefreshWorkers,
    LoadDaemonStatus,
    OpenProject(PathBuf),
    SubscribeCurrent,
    SendMessage {
        prompt: String,
        params: Value,
        correlation: ClientMessageCorrelation,
    },
    RetryFromIndex {
        index: usize,
        content: Value,
    },
    SetParams {
        patch: Value,
    },
    GoalCommand {
        kind: GoalCommandKind,
        content: Option<String>,
        budget: Option<GoalBudget>,
    },
    GoalControl {
        action: GoalControlAction,
    },
    RenameChat {
        title: String,
    },
    ForkChat {
        target_chat_id: String,
        source_chat_id: String,
        up_to_message_id: String,
        title: Option<String>,
    },
    ArchiveChat {
        chat_id: String,
        new_chat_id: String,
    },
    LoadDiff {
        root: PathBuf,
    },
    Worktree {
        action: surfaces::WorktreeAction,
    },
    CopyToClipboard {
        text: String,
        source: ClipboardCopySource,
    },
    OpenExternalEditor {
        draft: String,
    },
    OpenImageExternally {
        image: crate::terminal_image::InlineImage,
    },
    SendToolDecisions {
        client_request_id: String,
        decisions: Vec<ToolDecision>,
        patch: Option<Value>,
        rollback: ToolDecisionRollback,
    },
    SendBrowserContextDecision {
        decision: BrowserContextDecision,
        prompt: BrowserContextPromptState,
    },
    Abort,
    Quit {
        abort_active: bool,
    },
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

fn load_tui_config_content() -> Option<String> {
    let path = default_tui_config_path()?;
    fs::read_to_string(path).ok()
}

fn default_tui_config_path() -> Option<PathBuf> {
    if cfg!(test) {
        None
    } else {
        KeymapRegistry::default_config_path()
    }
}

fn initialize_syntax_theme(theme: &TuiTheme, config_path: Option<&Path>) {
    let theme_home = config_path
        .and_then(|path| path.parent())
        .map(Path::to_path_buf);
    let _ = highlight::set_theme_override(Some(theme.syntax_theme_name().to_string()), theme_home);
}

fn stable_path_hash(path: &std::path::Path) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn is_empty_live_assistant(message: &TranscriptMessage) -> bool {
    message.role == TranscriptRole::Assistant
        && !message.stream_finished
        && message.content.is_empty()
        && message.images.is_empty()
        && message.reasoning.is_empty()
        && message.tool_calls.is_empty()
        && message.citations.is_empty()
        && message.thinking_blocks.is_empty()
        && message.server_content_blocks.is_empty()
        && message.extra.is_empty()
        && message.unknown_delta_ops.is_empty()
}

fn split_command_name_and_args(input: &str) -> (&str, &str) {
    let input = input.trim().trim_start_matches('/').trim_start();
    match input.find(char::is_whitespace) {
        Some(index) => {
            let (name, args) = input.split_at(index);
            (name, args.trim())
        }
        None => (input, ""),
    }
}

fn notice_transcript_state(text: String) -> TranscriptState {
    let mut state = TranscriptState::new();
    state.push_notice(text);
    state
}

fn thread_update_params(raw: &Value) -> &Value {
    raw.get("params")
        .filter(|value| value.is_object())
        .unwrap_or(raw)
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

fn assistant_stream_width(width: u16) -> Option<usize> {
    crate::render::width::usable_content_width_u16(width, ASSISTANT_STREAM_RESERVED_COLS)
        .or(Some(1))
}

fn render_frame(terminal: &mut TerminalSession, app: &mut App) -> Result<(), TuiError> {
    terminal.set_title(&app.terminal_title())?;
    if app.native_scrollback() {
        let width = terminal.terminal_mut().size()?.width;
        app.note_terminal_resize_width(width);
        if app.resize_reflow_is_due() {
            let ran_during_stream = app.should_mark_resize_reflow_as_stream_time();
            terminal.clear_for_resize_reflow()?;
            for insertion in app.resize_reflow_insertions(width) {
                insert_history(terminal.terminal_mut(), insertion)?;
            }
            app.finish_resize_reflow(width, ran_during_stream);
        } else {
            for insertion in app.pending_history_insertions(width) {
                insert_history(terminal.terminal_mut(), insertion)?;
            }
        }
    }
    let completed_frame = terminal
        .terminal_mut()
        .draw(|frame| crate::ui::render(frame, app))?;
    if !app.native_scrollback() {
        let positions = crate::terminal_image::image_positions(
            completed_frame.buffer,
            app.visible_transcript(),
        );
        let mut images = crate::app::transcript::inline_images_for_visible_transcript(
            app.visible_transcript(),
            crate::terminal_probe::image_protocol_from_env(),
            &positions,
        );
        browser::append_inline_image(app, completed_frame.buffer, &mut images);
        terminal.write_inline_images(&images)?;
    }
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

fn composer_search_text(key: KeyEvent) -> Option<char> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn is_ctrl_c_key(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('c' | 'C'))
}

fn is_plain_space_key(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(key.code, KeyCode::Char(' '))
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::PauseReason;
    use crate::ui::goal_dock;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    fn project_b() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p2".to_string(),
            slug: "other".to_string(),
            root: PathBuf::from("/tmp/other"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn assert_tool_decision_action(
        action: &AppAction,
        decisions: Vec<ToolDecision>,
        patch: Option<Value>,
    ) {
        match action {
            AppAction::SendToolDecisions {
                client_request_id,
                decisions: actual_decisions,
                patch: actual_patch,
                ..
            } => {
                assert!(!client_request_id.is_empty());
                assert_eq!(actual_decisions, &decisions);
                assert_eq!(actual_patch, &patch);
            }
            other => panic!("expected tool decision action, got {other:?}"),
        }
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
            raw: json!({"reasons": [pause_reason_value(tool_call_id, tool_name)]}),
        }
    }

    fn pause_reason_value(tool_call_id: &str, tool_name: &str) -> Value {
        json!({"type": "confirmation", "tool_name": tool_name, "command": format!("{tool_name}({tool_call_id})"), "rule": "*", "tool_call_id": tool_call_id})
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

    fn assistant_cell_count(app: &App) -> usize {
        app.visible_transcript()
            .iter()
            .filter(|item| matches!(item, TranscriptItem::Assistant(_)))
            .count()
    }

    fn reasoning_text(app: &App) -> String {
        app.visible_transcript()
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Reasoning(text, _) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>()
    }

    fn reasoning_cell_count(app: &App) -> usize {
        app.visible_transcript()
            .iter()
            .filter(|item| matches!(item, TranscriptItem::Reasoning(_, _)))
            .count()
    }

    fn injected_model_text() -> &'static str {
        "lead \u{1b}[31mred \u{1b}[2Jclear\u{7} bell \u{009b}31mcsi \u{1b}]8;;http://evil\u{7}TEXT\u{1b}]8;;\u{7} tail"
    }

    fn assert_escape_inert(text: &str) {
        assert!(!text.as_bytes().contains(&0x1b), "raw ESC in {text:?}");
        assert!(!text.as_bytes().contains(&0x07), "raw BEL in {text:?}");
        assert!(!text.as_bytes().contains(&0x9b), "raw CSI byte in {text:?}");
        assert!(!text.contains('\u{009b}'), "raw CSI char in {text:?}");
        assert!(!text.contains("http://evil"), "raw OSC8 URL in {text:?}");
    }

    fn assert_model_text_survives(text: &str) {
        for fragment in ["lead", "red", "clear", "bell", "csi", "TEXT", "tail"] {
            assert!(text.contains(fragment), "missing {fragment:?} in {text:?}");
        }
    }

    fn assert_rendered_item_escape_inert(item: &TranscriptItem) {
        let lines = crate::history::cells::cell_from_transcript_item(item, false).render(80);
        for line in lines {
            for span in line.spans {
                assert_escape_inert(span.content.as_ref());
            }
        }
    }

    fn stream_tail_plain_lines(app: &App) -> Vec<String> {
        let mut stream = app.stream_controller.clone();
        stream
            .current_tail_lines()
            .iter()
            .map(|line| line_to_plain_string(&line.line))
            .collect()
    }

    fn plan_stream_text(app: &App) -> String {
        app.visible_transcript()
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::PlanStream(lines) => Some(
                    lines
                        .iter()
                        .map(|line| line_to_plain_string(&line.line))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
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

    fn questions_cell_count(app: &App) -> usize {
        app.visible_transcript()
            .iter()
            .filter(|item| rendered_item_plain_text(item).contains("Questions"))
            .count()
    }

    fn session_cell_count(app: &App) -> usize {
        app.visible_transcript()
            .iter()
            .filter(|item| matches!(item, TranscriptItem::Session { .. }))
            .count()
    }

    fn session_text(app: &App) -> String {
        app.visible_transcript()
            .iter()
            .filter(|item| matches!(item, TranscriptItem::Session { .. }))
            .map(rendered_item_plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rendered_item_plain_text(item: &TranscriptItem) -> String {
        crate::history::cells::cell_from_transcript_item(item, false)
            .render(80)
            .iter()
            .map(line_to_plain_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assistant_added_event(app: &App, id: &str, content: &str) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": id,
                "role": "assistant",
                "content": content,
                "stream_finished": true,
            }}),
        }
    }

    fn snapshot_event(app: &App, messages: Vec<Value>) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": messages}),
        }
    }

    fn tool_call_delta_event(app: &App, tool_call_id: &str) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{
                "id": tool_call_id,
                "function": {"name": "shell", "arguments": "{\"cmd\":\"echo 1\"}"}
            }]}]}),
        }
    }

    fn tool_result_message(tool_call_id: &str, content: &str) -> Value {
        json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "tool_failed": false,
            "content": content,
            "stream_finished": true,
        })
    }

    fn tool_result_event(app: &App, tool_call_id: &str, content: &str) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": tool_result_message(tool_call_id, content)}),
        }
    }

    fn ask_questions_tool_event(app: &App, tool_call_id: &str, questions: Value) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({
                "message": {
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "tool_failed": false,
                    "content": json!({
                        "type": "ask_questions",
                        "tool_call_id": tool_call_id,
                        "questions": questions,
                    }).to_string()
                }
            }),
        }
    }

    fn waiting_user_input_event(app: &App) -> ChatEvent {
        runtime_updated_event(app, "waiting_user_input")
    }

    fn runtime_updated_event(app: &App, state: &str) -> ChatEvent {
        ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: json!({"state": state}),
        }
    }

    fn app_with_queued_input() -> App {
        let mut app = App::new(project());
        app.composer.set_text("first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { prompt, .. } if prompt == "first"
        ));
        app.composer.set_text("second");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.input_queue().len(), 1);
        app
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
    fn live_transcript_drops_oldest_items_after_limit() {
        let mut app = App::new(project());
        for idx in 0..10_005 {
            app.test_push_history_item(TranscriptItem::Notice(format!("notice {idx}")));
        }

        assert_eq!(app.visible_transcript().len(), 10_000);
        assert!(matches!(
            app.visible_transcript().first(),
            Some(TranscriptItem::Notice(text)) if text == LIVE_TRANSCRIPT_RETENTION_NOTICE
        ));
        assert!(!app
            .visible_transcript()
            .iter()
            .any(|item| { matches!(item, TranscriptItem::Notice(text) if text == "notice 0") }));
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text == "notice 10004")
        }));
    }

    #[test]
    fn live_transcript_retention_preserves_running_tool() {
        let mut app = App::new(project());
        app.test_push_tool(ToolCard::from_tool_call(
            &json!({"id": "call-keep", "function": {"name": "shell", "arguments": "{}"}}),
        ));
        for idx in 0..10_005 {
            app.test_push_history_item(TranscriptItem::Notice(format!("notice {idx}")));
        }

        assert_eq!(app.visible_transcript().len(), 10_000);
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| { matches!(item, TranscriptItem::Tool(card) if card.id == "call-keep") }));
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
    fn native_scrollback_prompt_start_does_not_enqueue_blank_assistant() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        app.composer.set_text("hello");

        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { .. }
        ));

        assert_eq!(app.history_pending_count(), 1);
        let insertions = app.pending_history_insertions(80);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(app.visible_transcript(), &[]);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        assert_eq!(app.history_pending_count(), 0);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_started".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"message_id": "a1", "ops": [{"op": "append_content", "text": "hi"}]}),
        });
        assert_eq!(app.history_pending_count(), 0);
        assert_eq!(assistant_text(&app), "hi");

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        assert_eq!(assistant_text(&app), "");
        assert_eq!(app.history_pending_count(), 1);
        let insertions = app.pending_history_insertions(80);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn stream_added_snapshot_reconciles_to_one_assistant_cell() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_started".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"message_id": "a1", "ops": [{"op": "append_content", "text": "hello"}]}),
        });
        app.handle_chat_event(assistant_added_event(&app, "a1", "hello"));
        app.handle_chat_event(snapshot_event(
            &app,
            vec![json!({
                "message_id": "a1",
                "role": "assistant",
                "content": "hello",
                "stream_finished": true,
            })],
        ));

        assert_eq!(assistant_cell_count(&app), 1);
        assert_eq!(assistant_text(&app), "hello");
    }

    #[test]
    fn repeated_identical_message_added_does_not_duplicate_assistant() {
        let mut app = App::new(project());
        let event = assistant_added_event(&app, "a1", "hello");

        app.handle_chat_event(event.clone());
        app.handle_chat_event(event);

        assert_eq!(assistant_cell_count(&app), 1);
        assert_eq!(assistant_text(&app), "hello");
    }

    #[test]
    fn new_chat_shows_codex_style_session_header() {
        let mut app = App::new(project());

        app.new_chat();

        let text = session_text(&app);
        assert_eq!(session_cell_count(&app), 1);
        assert!(text.contains(">_ refact"));
        assert!(text.contains("model: default · /model to change"));
        assert!(text.contains("directory: /tmp/demo"));
        assert!(text.contains("Tips: type /help for shortcuts"));
    }

    #[test]
    fn new_chat_session_header_updates_model_without_duplication() {
        let mut app = App::new(project());
        app.new_chat();

        app.apply_caps(&json!({
            "defaults": {
                "chat_default_model": "openai/gpt-demo",
                "chat_default_mode": "agent"
            },
            "chat_models": {
                "openai/gpt-demo": {"name": "GPT Demo"}
            }
        }));
        app.handle_chat_event(snapshot_event(&app, Vec::new()));

        let text = session_text(&app);
        assert_eq!(session_cell_count(&app), 1);
        assert!(text.contains("model: openai/gpt-demo · /model to change"));
        assert!(!text.contains("model: default"));
    }

    #[test]
    fn ask_questions_message_and_waiting_snapshot_keep_one_questions_cell() {
        let mut app = App::new(project());
        let questions = json!([
            {"id": "confirm", "type": "yes_no", "text": "Proceed?"}
        ]);
        let assistant_message = json!({
            "message_id": "a1",
            "role": "assistant",
            "stream_finished": true,
            "tool_calls": [{
                "id": "call-ask",
                "function": {
                    "name": "ask_questions",
                    "arguments": json!({"questions": questions}).to_string(),
                }
            }]
        });
        let tool_message = tool_result_message(
            "call-ask",
            &json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": questions,
            })
            .to_string(),
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": assistant_message.clone()}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": tool_message.clone()}),
        });
        app.handle_chat_event(waiting_user_input_event(&app));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "waiting_user_input"}, "messages": [assistant_message, tool_message]}),
        });

        assert_eq!(questions_cell_count(&app), 1);
        assert!(app.ask_questions_form().is_some());
    }

    #[test]
    fn active_ask_questions_cell_stays_compact() {
        let mut app = App::new(project());
        let questions = json!([
            {"id": "file", "type": "free_text", "text": "Which file should I edit?"}
        ]);
        let assistant_message = json!({
            "message_id": "a1",
            "role": "assistant",
            "stream_finished": true,
            "tool_calls": [{
                "id": "call-ask",
                "function": {
                    "name": "ask_questions",
                    "arguments": json!({"questions": questions}).to_string(),
                }
            }]
        });
        let tool_message = tool_result_message(
            "call-ask",
            &json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": questions,
            })
            .to_string(),
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": assistant_message}),
        });
        let tool_card = app
            .visible_transcript()
            .iter()
            .position(|item| matches!(item, TranscriptItem::Tool(card) if card.id == "call-ask"))
            .unwrap();
        app.selected_tool_index = Some(tool_card);
        app.toggle_selected_tool();
        assert!(
            matches!(app.visible_transcript().get(tool_card), Some(TranscriptItem::Tool(card)) if card.expanded)
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": tool_message}),
        });
        app.handle_chat_event(waiting_user_input_event(&app));

        let rendered = app
            .visible_transcript()
            .iter()
            .map(rendered_item_plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(app.ask_questions_form().is_some());
        assert!(
            matches!(app.visible_transcript().get(tool_card), Some(TranscriptItem::Tool(card)) if !card.expanded)
        );
        assert_eq!(rendered.matches("Which file should I edit?").count(), 1);
    }

    #[test]
    fn completed_tool_card_transitions_to_history_once() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);

        app.handle_chat_event(tool_call_delta_event(&app, "call-1"));
        app.handle_chat_event(tool_result_event(&app, "call-1", "done"));
        app.handle_chat_event(tool_result_event(&app, "call-1", "done"));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });

        assert_eq!(tool_cards(&app).len(), 0);
        assert_eq!(app.history_pending_count(), 1);
        let inserted_before = app.history_inserted_cell_count();
        let insertions = app.pending_history_insertions(80);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
        assert!(app.pending_history_insertions(80).is_empty());
        assert_eq!(app.history_inserted_cell_count() - inserted_before, 1);
    }

    #[test]
    fn tool_result_uses_structured_exec_metadata() {
        let mut app = App::new(project());
        app.handle_chat_event(tool_call_delta_event(&app, "call-1"));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "role": "tool",
                "tool_call_id": "call-1",
                "content": "The command was running 0.120s, finished with exit code 1",
                "tool_failed": false,
                "extra": {"exec": {"duration_ms": 8_000, "exit_code": 0}}
            }}),
        });

        let rendered = rendered_item_plain_text(&TranscriptItem::Tool(tool_cards(&app)[0].clone()));
        assert!(rendered.contains("exit 0 · 8.0s"));
        assert!(!rendered.contains("exit 1"));
    }

    #[test]
    fn native_new_chat_session_header_updates_model_without_duplication() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        app.new_chat();

        let inserted_before = app.history_inserted_cell_count();
        let insertions = app.pending_history_insertions(80);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(app.history_inserted_cell_count() - inserted_before, 1);

        app.apply_caps(&json!({
            "defaults": {"chat_default_model": "openai/gpt-demo"},
            "chat_models": {"openai/gpt-demo": {"name": "GPT Demo"}}
        }));
        assert!(app.resize_reflow_is_due());
        app.handle_chat_event(snapshot_event(&app, Vec::new()));

        assert_eq!(app.history_pending_count(), 0);
        let reflow = app.resize_reflow_insertions(80);
        let rendered = reflow
            .iter()
            .flat_map(|insertion| insertion.lines.iter())
            .map(|line| line_to_plain_string(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("model: openai/gpt-demo · /model to change"));
        assert!(!rendered.contains("model: default"));
    }

    #[test]
    fn identical_snapshot_after_insertions_adds_no_history_entries() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        let messages = vec![
            json!({"message_id": "u1", "role": "user", "content": "hello"}),
            json!({"message_id": "a1", "role": "assistant", "content": "hi", "stream_finished": true}),
        ];

        app.handle_chat_event(snapshot_event(&app, messages.clone()));
        assert_eq!(app.history_pending_count(), 2);
        let inserted_before = app.history_inserted_cell_count();
        let insertions = app.pending_history_insertions(80);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            2
        );
        let inserted_after_first = app.history_inserted_cell_count();

        app.handle_chat_event(snapshot_event(&app, messages));

        assert_eq!(app.history_pending_count(), 0);
        assert!(app.pending_history_insertions(80).is_empty());
        assert_eq!(app.history_inserted_cell_count(), inserted_after_first);
        assert_eq!(inserted_after_first - inserted_before, 2);
    }

    #[test]
    fn changed_snapshot_after_insertions_reflows_native_scrollback_region() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        let initial = vec![
            json!({"message_id": "u1", "role": "user", "content": "hello"}),
            json!({"message_id": "a1", "role": "assistant", "content": "stale answer", "stream_finished": true}),
        ];

        app.handle_chat_event(snapshot_event(&app, initial));
        assert!(app.resize_reflow_is_due());
        let first_reflow = app.resize_reflow_insertions(80);
        app.finish_resize_reflow(80, false);
        let first_text = first_reflow
            .iter()
            .flat_map(|insertion| insertion.lines.iter())
            .map(|line| line_to_plain_string(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(first_text.contains("stale answer"));

        let changed = vec![
            json!({"message_id": "u1", "role": "user", "content": "hello"}),
            json!({"message_id": "a1", "role": "assistant", "content": "fresh answer", "stream_finished": true}),
        ];
        app.handle_chat_event(snapshot_event(&app, changed));

        assert!(app.resize_reflow_is_due());
        let changed_reflow = app.resize_reflow_insertions(80);
        let changed_text = changed_reflow
            .iter()
            .flat_map(|insertion| insertion.lines.iter())
            .map(|line| line_to_plain_string(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(changed_text.contains("fresh answer"));
        assert!(!changed_text.contains("stale answer"));
        assert_eq!(app.history_pending_count(), 0);
    }

    #[test]
    fn reasoning_stream_finished_snapshot_reconciles_to_one_reasoning_cell() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_started".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"message_id": "a1", "ops": [{"op": "append_reasoning", "text": "think"}]}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        app.handle_chat_event(snapshot_event(
            &app,
            vec![json!({
                "message_id": "a1",
                "role": "assistant",
                "reasoning": "think",
                "stream_finished": true,
            })],
        ));

        assert_eq!(reasoning_cell_count(&app), 1);
        assert_eq!(reasoning_text(&app), "think");
    }

    #[test]
    fn in_progress_plan_snapshot_replay_preserves_history_and_live_stream_once() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        let messages = vec![json!({
            "message_id": "p1",
            "role": "plan",
            "content": "- one\n- two\n- three\n- four\n- five\n",
            "stream_finished": false,
            "extra": {"plan": {"mode": "agent", "version": 1}}
        })];

        app.handle_chat_event(snapshot_event(&app, messages.clone()));
        let visible_before = app.visible_transcript().len();
        let stream_text_before = plan_stream_text(&app);
        assert!(stream_text_before.contains("- five"));

        app.handle_chat_event(snapshot_event(&app, messages));

        assert_eq!(app.history_pending_count(), 0);
        assert_eq!(app.visible_transcript().len(), visible_before);
        assert_eq!(plan_stream_text(&app), stream_text_before);
        assert_eq!(
            app.visible_transcript()
                .iter()
                .filter(|item| matches!(item, TranscriptItem::PlanStream(_)))
                .count(),
            1
        );
    }

    #[test]
    fn ordinary_event_snapshot_replay_does_not_duplicate_events() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        let messages = vec![json!({
            "message_id": "e1",
            "role": "event",
            "content": "Process exited with code 0",
            "extra": {"event": {"subkind": "process_completed", "source": "exec.registry", "payload": {"exit_code": 0}}}
        })];

        app.handle_chat_event(snapshot_event(&app, messages.clone()));
        let events_before = app.events_pane().events().len();
        let pending_before = app.history_pending_count();

        app.handle_chat_event(snapshot_event(&app, messages));

        assert_eq!(app.events_pane().events().len(), events_before);
        assert_eq!(app.history_pending_count(), pending_before);
    }

    #[test]
    fn streaming_resize_rewraps_live_tail() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_started".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        app.note_terminal_resize_width(80);
        let source = "intro\nalpha beta gamma delta epsilon zeta eta theta";
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"message_id": "a1", "ops": [{"op": "append_content", "text": source}]}),
        });
        let wide_tail = stream_tail_plain_lines(&app);

        app.note_terminal_resize_width(16);
        let narrow_tail = stream_tail_plain_lines(&app);

        assert_eq!(
            wide_tail,
            vec!["alpha beta gamma delta epsilon zeta eta theta"]
        );
        assert!(narrow_tail.len() > wide_tail.len());
        assert_eq!(assistant_text(&app), source);
        app.apply_stream_commit_tick();
        assert_eq!(app.active_stream_committed(), "intro\n");
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({"message_id": "a1"}),
        });
        assert_eq!(assistant_text(&app), format!("{source}\n"));
    }

    #[test]
    fn native_scrollback_completed_tool_moves_to_history_once() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "shell", "arguments": "{\"cmd\":\"echo 1\"}"}}]}]}),
        });
        assert_eq!(tool_cards(&app).len(), 1);

        app.complete_tool("call-1", "tool", "done".to_string(), ToolStatus::Succeeded);

        assert_eq!(tool_cards(&app).len(), 1);
        assert_eq!(tool_cards(&app)[0].status, ToolStatus::Succeeded);
        assert_eq!(app.selected_tool_index(), Some(0));
        assert_eq!(app.history_pending_count(), 0);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });

        assert_eq!(tool_cards(&app).len(), 0);
        assert_eq!(app.selected_tool_index(), None);
        assert_eq!(app.history_pending_count(), 1);
        let inserted_before = app.history_inserted_cell_count();
        let insertions = app.pending_history_insertions(80);
        assert_eq!(
            insertions
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );
        let rendered_after_insert = app.history_render_count();
        app.begin_frame_render();
        assert_eq!(app.visible_transcript().len(), 0);
        assert!(app.pending_history_insertions(80).is_empty());
        assert_eq!(app.history_inserted_cell_count() - inserted_before, 1);
        assert_eq!(app.history_render_count(), rendered_after_insert);
    }

    #[test]
    fn native_snapshot_replaces_pending_changed_content_and_skips_identical_snapshot() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "a1", "role": "assistant", "content": "stale", "stream_finished": true}
            ]}),
        });
        let stale = app.pending_history_insertions(80);
        assert_eq!(
            stale
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "a1", "role": "assistant", "content": "corrected", "stream_finished": true}
            ]}),
        });
        let corrected = app.pending_history_insertions(80);
        let corrected_text = corrected
            .iter()
            .flat_map(|insertion| insertion.lines.iter())
            .map(|hl| line_to_plain_string(&hl.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(corrected_text.contains("corrected"));
        assert!(!corrected_text.contains("stale"));
        assert_eq!(
            corrected
                .iter()
                .map(|insertion| insertion.cell_ids.len())
                .sum::<usize>(),
            1
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "a1", "role": "assistant", "content": "corrected", "stream_finished": true}
            ]}),
        });
        assert!(app.pending_history_insertions(80).is_empty());
    }

    #[test]
    fn app_config_enables_vim_theme_and_custom_binding() {
        let mut app = App::new(project());
        app.test_apply_tui_config_content(
            r#"
vim_mode = true

[bindings]
send = "ctrl-s"

[theme]
name = "plain"
"#,
        );
        assert!(app.vim_enabled());
        assert_eq!(app.vim_mode(), VimMode::Normal);
        assert_eq!(app.theme().name(), "plain");
        app.composer.set_text("hello");
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "hello"
        ));
    }

    #[test]
    fn app_config_applies_notification_settings() {
        let mut app = App::new(project());
        app.test_apply_tui_config_content(
            r#"
[notifications]
enabled = true
bell = false
"#,
        );

        assert_eq!(app.notification_status_label(), "OSC9");
    }

    #[test]
    fn app_config_reports_invalid_multibyte_theme_color() {
        let mut app = App::new(project());
        app.test_apply_tui_config_content(
            r##"
[theme]
accent = "#ééé"
"##,
        );

        assert_eq!(app.theme().name(), "dark");
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("invalid theme accent color"))
        }));
    }

    #[test]
    fn app_notification_gate_disables_pending_notifications() {
        let mut app = App::new(project());
        app.test_set_notifications_config(NotificationConfig::new(false, true, Duration::ZERO));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });

        assert!(app.test_take_pending_notifications().is_empty());
        assert_eq!(app.notification_status_label(), "off");
    }

    #[test]
    fn stream_finished_and_approval_queue_notifications_when_unfocused() {
        let mut app = App::new(project());
        app.test_set_terminal_focus(false);
        app.test_set_notifications_config(NotificationConfig::new(true, true, Duration::ZERO));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));

        let pending = app.test_take_pending_notifications();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0], b"\x1b]9;Refact: response ready\x07\x07");
        assert_eq!(pending[1], b"\x1b]9;Refact: approval needed\x07\x07");
    }

    #[test]
    fn app_notifications_debounce_duplicate_turn_complete() {
        let mut app = App::new(project());
        app.test_set_terminal_focus(false);
        app.test_set_notifications_config(NotificationConfig::new(
            true,
            true,
            Duration::from_secs(60),
        ));

        for _ in 0..2 {
            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "stream_finished".to_string(),
                raw: json!({}),
            });
        }

        assert_eq!(app.test_take_pending_notifications().len(), 1);
    }

    #[test]
    fn notifications_suppress_when_focused() {
        let mut app = App::new(project());
        app.test_set_terminal_focus(true);
        app.test_set_notifications_config(NotificationConfig::new(true, true, Duration::ZERO));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));

        assert!(app.test_take_pending_notifications().is_empty());
    }

    #[test]
    fn app_dispatches_through_active_registry_binding() {
        let mut app = App::new(project());
        let keymap = KeymapRegistry::from_toml_str(
            r#"
[bindings]
new-chat = "ctrl-x"
"#,
        )
        .unwrap();
        app.test_set_keymap(keymap);
        let old_chat_id = app.chat_id().to_string();
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            AppAction::SubscribeCurrent
        );
        assert_ne!(app.chat_id(), old_chat_id);
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            AppAction::None
        );
    }

    #[test]
    fn recent_session_next_key_switches_to_next_chat() {
        let mut app = App::new(project());
        let current = app.chat_id().to_string();
        app.test_set_recent_sessions(vec![
            PickerItem {
                id: current.clone(),
                title: "Current".to_string(),
                description: "now".to_string(),
            },
            PickerItem {
                id: "chat-next".to_string(),
                title: "Next chat".to_string(),
                description: "recent".to_string(),
            },
        ]);

        let action = app.handle_key(key(KeyCode::F(7)));

        assert_eq!(action, AppAction::SubscribeCurrent);
        assert_eq!(app.chat_id(), "chat-next");
        assert_eq!(app.session_title(), Some("Next chat"));
        assert!(app
            .session_tabs()
            .iter()
            .any(|tab| { tab.id == "chat-next" && tab.is_current }));
    }

    #[test]
    fn recent_session_previous_key_wraps_and_switches() {
        let mut app = App::new(project());
        let current = app.chat_id().to_string();
        app.test_set_recent_sessions(vec![
            PickerItem {
                id: current,
                title: "Current".to_string(),
                description: "now".to_string(),
            },
            PickerItem {
                id: "chat-last".to_string(),
                title: "Last chat".to_string(),
                description: "recent".to_string(),
            },
        ]);

        let action = app.handle_key(key(KeyCode::F(6)));

        assert_eq!(action, AppAction::SubscribeCurrent);
        assert_eq!(app.chat_id(), "chat-last");
    }

    #[test]
    fn recent_session_key_with_empty_cache_requests_refresh() {
        let mut app = App::new(project());

        assert_eq!(
            app.handle_key(key(KeyCode::F(7))),
            AppAction::RefreshRecentSessions
        );
        assert_eq!(app.session_tabs().len(), 1);
    }

    #[test]
    fn app_vim_mode_basic_motions_and_delete_line() {
        let mut app = App::new(project());
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(app.vim_enabled());
        assert_eq!(app.vim_mode(), VimMode::Normal);
        app.composer.set_text("alpha beta");
        app.handle_key(key(KeyCode::Char('b')));
        assert_eq!(app.composer.cursor_char_offset(), 6);
        app.handle_key(key(KeyCode::Char('b')));
        assert_eq!(app.composer.cursor_char_offset(), 0);
        app.composer.set_text("alpha beta\ngamma");
        app.handle_key(key(KeyCode::Char('b')));
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(app.composer(), "alpha beta\n");
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.vim_mode(), VimMode::Insert);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.vim_mode(), VimMode::Normal);
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
    fn runtime_states_map_to_distinct_footer_states() {
        let mut app = App::new(project());
        let states = [
            ("idle", SessionState::Idle, "idle"),
            ("generating", SessionState::Generating, "generating"),
            (
                "executing_tools",
                SessionState::ExecutingTools,
                "running tools",
            ),
            ("paused", SessionState::Paused, "approval pending"),
            ("waiting_ide", SessionState::WaitingIde, "waiting for IDE…"),
            (
                "waiting_user_input",
                SessionState::WaitingUserInput,
                "waiting for input",
            ),
            ("completed", SessionState::Completed, "completed"),
            ("error", SessionState::Error, "error"),
        ];

        for (runtime_state, expected_state, expected_footer_label) in states {
            app.handle_chat_event(runtime_updated_event(&app, runtime_state));

            assert_eq!(app.session_state(), expected_state);
            let footer = crate::ui::footer::FooterData::from_app(&app);
            assert!(crate::ui::footer::footer_text(&footer).contains(expected_footer_label));
        }
    }

    #[test]
    fn waiting_ide_footer_is_blocked_and_abort_restores_idle() {
        let mut app = App::new(project());
        app.handle_chat_event(runtime_updated_event(&app, "waiting_ide"));

        let footer = crate::ui::footer::footer_text(&crate::ui::footer::FooterData::from_app(&app));
        assert!(footer.contains("waiting for IDE…"));
        assert!(footer.contains("Esc to abort"));
        assert!(!footer.contains("● idle"));
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::Abort);
        assert!(app.abort_in_flight);

        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: app.command_origin(),
                },
                Ok(()),
            ),
            AppAction::None
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
    }

    #[test]
    fn stream_finished_preserves_completed_runtime_state() {
        let mut app = App::new(project());
        app.handle_chat_event(runtime_updated_event(&app, "completed"));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });

        assert_eq!(app.session_state(), SessionState::Completed);
    }

    #[test]
    fn app_newline_keys_insert_and_enter_submits() {
        let mut app = App::new(project());
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty()));
        assert_eq!(app.composer(), "a\nb\nc");
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "a\nb\nc"
        ));
    }

    #[test]
    fn app_space_key_inserts_space_in_composer() {
        let mut app = App::new(project());

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));

        assert_eq!(app.composer(), "a b");
    }

    #[test]
    fn composer_submit_preserves_multiline_whitespace_and_history() {
        let mut app = App::new(project());
        let prompt = "  indented\nkeep trailing line\n";
        app.composer.set_text(prompt);

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(matches!(
            action,
            AppAction::SendMessage { prompt: sent, .. } if sent == prompt
        ));
        assert_eq!(
            app.composer_state().history_entries(),
            &[prompt.to_string()]
        );
    }

    #[test]
    fn ask_questions_tool_result_opens_form_and_submits_canonical_reply() {
        let mut app = App::new(project());
        app.handle_chat_event(ask_questions_tool_event(
            &app,
            "call-ask",
            json!([
                {"id": "confirm", "type": "yes_no", "text": "Proceed?"}
            ]),
        ));
        assert!(app.ask_questions_form().is_none());
        app.handle_chat_event(waiting_user_input_event(&app));

        assert!(app.ask_questions_form().is_some());
        assert_eq!(app.session_state(), SessionState::WaitingUserInput);
        app.handle_key(key(KeyCode::Char('n')));
        let action = app.handle_key(key(KeyCode::Enter));

        assert!(app.ask_questions_form().is_none());
        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. }
                if prompt == "[QA:call-ask]\n> [confirm] Proceed?\nNo"
        ));
        app.handle_chat_event(waiting_user_input_event(&app));
        assert!(app.ask_questions_form().is_none());
    }

    #[test]
    fn ask_questions_does_not_submit_unseen_yes_no_defaults() {
        let mut app = App::new(project());
        app.handle_chat_event(ask_questions_tool_event(
            &app,
            "call-ask",
            json!([
                {"id":"first","type":"yes_no","text":"First?"},
                {"id":"second","type":"yes_no","text":"Second?"},
                {"id":"third","type":"yes_no","text":"Third?"}
            ]),
        ));
        app.handle_chat_event(waiting_user_input_event(&app));

        assert_eq!(app.handle_key(key(KeyCode::Right)), AppAction::None);
        assert_eq!(app.handle_key(key(KeyCode::Right)), AppAction::None);
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);

        let form = app
            .ask_questions_form()
            .expect("incomplete form remains open");
        assert_eq!(form.current_index(), 0);
        assert_eq!(
            form.submission_error().as_deref(),
            Some("Answer outstanding questions: 1, 2, 3")
        );
        assert!(!form.format_answers().contains("\nYes"));
    }

    #[test]
    fn ask_questions_cancel_stays_canceled_and_manual_reply_is_canonical() {
        let mut app = App::new(project());
        let tool_message = json!({
            "role": "tool",
            "tool_call_id": "call-ask",
            "tool_failed": false,
            "content": json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": [
                    {"id": "notes", "type": "free_text", "text": "Notes?"}
                ]
            }).to_string()
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": tool_message.clone()}),
        });
        app.handle_chat_event(waiting_user_input_event(&app));

        assert!(app.ask_questions_form().is_some());
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);
        assert!(app.ask_questions_form().is_none());
        assert_eq!(app.session_state(), SessionState::WaitingUserInput);

        app.handle_chat_event(waiting_user_input_event(&app));
        assert!(app.ask_questions_form().is_none());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {}, "runtime": {"state": "waiting_user_input"}, "messages": [tool_message]}),
        });
        assert!(app.ask_questions_form().is_none());

        app.composer.set_text("manual answer");
        let action = app.handle_key(key(KeyCode::Enter));

        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. }
                if prompt == "[QA:call-ask]\n> [notes] Notes?\n(no answer)\n\n> [__additional__] Additional comments\nmanual answer"
        ));
    }

    #[test]
    fn queued_input_waits_for_idle_runtime_after_stream_finished() {
        let mut app = App::new(project());
        app.composer.set_text("first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { prompt, .. } if prompt == "first"
        ));
        app.composer.set_text("second");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        app.composer.set_text("third");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.input_queue().len(), 2);

        let action = app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });

        assert_eq!(action, AppAction::None);
        assert_eq!(app.input_queue().len(), 2);

        let action = app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: json!({"state": "executing_tools"}),
        });

        assert_eq!(action, AppAction::None);
        assert_eq!(app.input_queue().len(), 2);
        assert_eq!(app.session_state(), SessionState::ExecutingTools);

        let action = app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: json!({"state": "idle"}),
        });

        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "second"
        ));
        assert_eq!(app.input_queue().len(), 1);
        assert_eq!(app.input_queue().items()[0].text, "third");
        assert_eq!(app.session_state(), SessionState::Generating);
    }

    #[test]
    fn project_switch_handoff_prevents_cross_project_queue_dispatch() {
        let project_a = project();
        let mut app = App::new(project_a.clone());
        let chat_a = app.chat_id().to_string();
        app.input_queue
            .enqueue("first for A".to_string(), Value::Null);
        app.input_queue
            .enqueue("second for A".to_string(), Value::Null);
        app.composer.set_text("draft for A");

        app.set_project(project_b());

        assert!(app.input_queue().is_empty());
        assert!(app.composer().is_empty());
        assert_eq!(
            app.handle_chat_event(runtime_updated_event(&app, "idle")),
            AppAction::None
        );
        app.abort_in_flight = true;
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: app.command_origin(),
                },
                Ok(()),
            ),
            AppAction::None
        );

        app.set_project(project_a);

        assert_eq!(app.chat_id(), chat_a);
        assert_eq!(app.composer(), "draft for A");
        assert_eq!(app.input_queue().len(), 2);
        assert!(matches!(
            app.handle_chat_event(runtime_updated_event(&app, "idle")),
            AppAction::SendMessage { prompt, .. } if prompt == "first for A"
        ));
        assert_eq!(app.input_queue().items()[0].text, "second for A");
    }

    #[test]
    fn chat_switch_handoff_restores_each_chat_draft_and_queue() {
        let mut app = App::new(project());
        let chat_a = app.chat_id().to_string();
        app.input_queue
            .enqueue("queued for A".to_string(), Value::Null);
        app.composer.set_text("draft for A");

        app.resume_chat("chat-b".to_string(), "Chat B".to_string(), None);

        assert!(app.input_queue().is_empty());
        assert!(app.composer().is_empty());
        app.input_queue
            .enqueue("queued for B".to_string(), Value::Null);
        app.composer.set_text("draft for B");

        app.resume_chat(chat_a, "Chat A".to_string(), None);

        assert_eq!(app.composer(), "draft for A");
        assert_eq!(app.input_queue().items()[0].text, "queued for A");

        app.resume_chat("chat-b".to_string(), "Chat B".to_string(), None);

        assert_eq!(app.composer(), "draft for B");
        assert_eq!(app.input_queue().items()[0].text, "queued for B");
    }

    #[test]
    fn new_chat_handoff_preserves_old_session_queue_for_ctrl_n_and_new() {
        let mut app = App::new(project());
        let chat_a = app.chat_id().to_string();
        app.input_queue
            .enqueue("queued for A".to_string(), Value::Null);
        app.composer.set_text("draft for A");

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            AppAction::SubscribeCurrent
        );
        assert!(app.input_queue().is_empty());
        assert!(app.composer().is_empty());
        assert_eq!(
            app.handle_chat_event(runtime_updated_event(&app, "idle")),
            AppAction::None
        );

        app.resume_chat(chat_a.clone(), "Chat A".to_string(), None);

        assert_eq!(app.composer(), "draft for A");
        assert_eq!(app.input_queue().items()[0].text, "queued for A");

        app.composer.set_text("/new");
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SubscribeCurrent
        );
        assert!(app.input_queue().is_empty());
        assert!(app.composer().is_empty());
        assert_eq!(
            app.handle_chat_event(runtime_updated_event(&app, "idle")),
            AppAction::None
        );

        app.resume_chat(chat_a, "Chat A".to_string(), None);

        assert!(app.composer().is_empty());
        assert_eq!(app.input_queue().items()[0].text, "queued for A");
    }

    #[test]
    fn queued_input_dispatches_only_for_explicit_idle_runtime_state() {
        let invalid_states = [
            ("missing", json!({}), "omitted state"),
            ("null", json!({"state": null}), "omitted state"),
            (
                "unknown",
                json!({"state": "not_a_runtime_state"}),
                "Unknown runtime state",
            ),
        ];

        for (event_name, snapshot) in [("runtime update", false), ("snapshot", true)] {
            for (state_name, raw, notice) in &invalid_states {
                let mut app = app_with_queued_input();
                let action = app.handle_chat_event(ChatEvent {
                    chat_id: Some(app.chat_id().to_string()),
                    seq: None,
                    kind: if snapshot {
                        "snapshot".to_string()
                    } else {
                        "runtime_updated".to_string()
                    },
                    raw: if snapshot {
                        json!({"runtime": raw, "messages": []})
                    } else {
                        raw.clone()
                    },
                });

                assert_eq!(action, AppAction::None, "{event_name} {state_name}");
                assert_eq!(app.session_state(), SessionState::Generating);
                assert_eq!(app.input_queue().len(), 1);
                assert!(app.visible_transcript().iter().any(|item| {
                    matches!(item, TranscriptItem::Notice(text) if text.contains(notice))
                }));
            }

            let mut app = app_with_queued_input();
            let action = app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: if snapshot {
                    "snapshot".to_string()
                } else {
                    "runtime_updated".to_string()
                },
                raw: if snapshot {
                    json!({"runtime": {"state": "idle"}, "messages": []})
                } else {
                    json!({"state": "idle"})
                },
            });

            assert!(matches!(
                action,
                AppAction::SendMessage { prompt, .. } if prompt == "second"
            ));
            assert!(app.input_queue().is_empty());
            assert_eq!(app.session_state(), SessionState::Generating);
        }
    }

    #[test]
    fn unacknowledged_send_timeout_rolls_back_and_restores_prompt() {
        let mut app = App::new(project());
        app.composer.set_text("hello");
        let action = app.handle_key(key(KeyCode::Enter));
        let (prompt, params, correlation) = match action {
            AppAction::SendMessage {
                prompt,
                params,
                correlation,
            } => (prompt, params, correlation),
            other => panic!("unexpected action: {other:?}"),
        };
        app.composer.set_text("draft");

        let result = app.handle_command_finished(
            CommandContextTag::SendMessage {
                prompt: prompt.clone(),
                params: params.clone(),
                correlation: correlation.clone(),
            },
            Err("request timed out".to_string()),
        );

        assert_eq!(result, AppAction::None);
        assert_eq!(app.session_state(), SessionState::Idle);
        assert_eq!(app.composer(), "hello");
        assert_eq!(app.input_queue().len(), 1);
        assert_eq!(app.input_queue().items()[0].text, "draft");
        assert!(!app
            .transcript_state()
            .messages()
            .iter()
            .any(|message| message.role == TranscriptRole::Assistant));
        assert!(!app
            .visible_transcript()
            .iter()
            .any(|item| matches!(item, TranscriptItem::Assistant(_))));
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("request timed out"))
        }));

        let retry = app.handle_key(key(KeyCode::Enter));
        match retry {
            AppAction::SendMessage {
                prompt: retry_prompt,
                params: retry_params,
                correlation: retry_correlation,
            } => {
                assert_eq!(retry_prompt, prompt);
                assert_eq!(retry_params, params);
                assert_eq!(retry_correlation, correlation);
            }
            other => panic!("unexpected retry action: {other:?}"),
        }
        assert_eq!(app.input_queue().len(), 1);
        assert_eq!(app.input_queue().items()[0].text, "draft");
    }

    #[test]
    fn accepted_send_timeout_keeps_one_optimistic_turn() {
        let mut app = App::new(project());
        app.composer.set_text("hello");
        let (prompt, params, correlation) = match app.handle_key(key(KeyCode::Enter)) {
            AppAction::SendMessage {
                prompt,
                params,
                correlation,
            } => (prompt, params, correlation),
            other => panic!("unexpected action: {other:?}"),
        };

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "ack".to_string(),
            raw: json!({
                "client_request_id": correlation.client_request_id,
                "accepted": true,
            }),
        });
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::SendMessage {
                    prompt,
                    params,
                    correlation,
                },
                Err("request timed out".to_string()),
            ),
            AppAction::None
        );

        assert_eq!(app.session_state(), SessionState::Generating);
        assert_eq!(app.composer(), "");
        assert!(app.pending_send_retry.is_none());
        assert!(app.in_flight_send.is_none());
        assert_eq!(
            app.transcript_state()
                .messages()
                .iter()
                .filter(|message| message.role == TranscriptRole::User)
                .count(),
            1
        );
        assert!(!app.visible_transcript().iter().any(
            |item| matches!(item, TranscriptItem::Notice(text) if text.contains("timed out"))
        ));
    }

    #[test]
    fn matching_client_message_id_reconciles_optimistic_user_echo() {
        let mut app = App::new(project());
        app.composer.set_text("hello");
        let client_message_id = match app.handle_key(key(KeyCode::Enter)) {
            AppAction::SendMessage { correlation, .. } => correlation.client_message_id,
            other => panic!("unexpected action: {other:?}"),
        };

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "server-user-1",
                "role": "user",
                "content": "hello",
                "extra": {"client_message_id": client_message_id},
            }}),
        });

        let users = app
            .transcript_state()
            .messages()
            .iter()
            .filter(|message| message.role == TranscriptRole::User)
            .collect::<Vec<_>>();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].message_id.as_deref(), Some("server-user-1"));
        assert_eq!(
            users[0].client_message_id(),
            Some(client_message_id.as_str())
        );
        assert!(app
            .in_flight_send
            .as_ref()
            .is_some_and(|send| send.accepted));
    }

    #[test]
    fn message_added_reconciliation_uses_authoritative_index_in_alternate_and_native_modes() {
        for native_scrollback in [false, true] {
            let mut app = App::new(project());
            app.set_native_scrollback(native_scrollback);
            app.handle_chat_event(snapshot_event(
                &app,
                vec![
                    json!({"message_id": "a", "role": "assistant", "content": "a", "stream_finished": true}),
                    json!({"message_id": "b", "role": "assistant", "content": "b", "stream_finished": true}),
                    json!({"message_id": "c", "role": "assistant", "content": "c", "stream_finished": true}),
                ],
            ));
            if native_scrollback {
                app.pending_history_insertions(80);
            }

            for (index, message_id, content) in [
                (0, "c", "c updated"),
                (1, "a", "a updated"),
                (2, "a", "a final"),
            ] {
                app.handle_chat_event(ChatEvent {
                    chat_id: Some(app.chat_id().to_string()),
                    seq: None,
                    kind: "message_added".to_string(),
                    raw: json!({"index": index, "message": {
                        "message_id": message_id,
                        "role": "assistant",
                        "content": content,
                        "stream_finished": true,
                    }}),
                });
            }

            assert_eq!(
                app.transcript_state()
                    .messages()
                    .iter()
                    .map(|message| message.message_id.as_deref())
                    .collect::<Vec<_>>(),
                [Some("c"), Some("b"), Some("a")],
            );
            assert_eq!(
                app.transcript_state()
                    .messages()
                    .iter()
                    .filter(|message| message.message_id.as_deref() == Some("a"))
                    .count(),
                1,
            );
            let pending_before_snapshot = app.history_pending_count();
            app.handle_chat_event(snapshot_event(&app, vec![
                json!({"message_id": "c", "role": "assistant", "content": "c updated", "stream_finished": true}),
                json!({"message_id": "b", "role": "assistant", "content": "b", "stream_finished": true}),
                json!({"message_id": "a", "role": "assistant", "content": "a final", "stream_finished": true}),
            ]));
            assert_eq!(app.transcript_state().messages().len(), 3);
            if native_scrollback {
                assert_eq!(app.history_pending_count(), pending_before_snapshot);
                let insertions = app.resize_reflow_insertions(80);
                let text = insertions
                    .iter()
                    .flat_map(|insertion| insertion.lines.iter())
                    .map(|line| line_to_plain_string(&line.line))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("c updated"));
                assert!(text.contains("a final"));
                assert_eq!(text.matches("a final").count(), 1);
            } else {
                assert_eq!(assistant_cell_count(&app), 3);
                assert_eq!(assistant_text(&app), "c updatedba final");
            }
        }
    }

    #[test]
    fn optimistic_message_added_reconciliation_uses_authoritative_index() {
        for native_scrollback in [false, true] {
            let mut app = App::new(project());
            app.transcript_state.reset();
            app.set_native_scrollback(native_scrollback);
            app.transcript_state
                .push_optimistic_user_message("same", "client-message-1");
            app.transcript_state
                .add_message(&json!({"message_id": "later", "role": "user", "content": "later"}));
            app.rebuild_render_transcript_from_state();
            if native_scrollback {
                app.pending_history_insertions(80);
            }

            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_added".to_string(),
                raw: json!({"index": 1, "message": {
                    "message_id": "server-user-1",
                    "role": "user",
                    "content": "same",
                    "extra": {"client_message_id": "client-message-1"},
                }}),
            });

            assert_eq!(
                app.transcript_state()
                    .messages()
                    .iter()
                    .map(|message| message.message_id.as_deref())
                    .collect::<Vec<_>>(),
                [Some("later"), Some("server-user-1")],
            );
            if native_scrollback {
                let insertions = app.resize_reflow_insertions(80);
                let text = insertions
                    .iter()
                    .flat_map(|insertion| insertion.lines.iter())
                    .map(|line| line_to_plain_string(&line.line))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("same"));
                assert!(text.contains("later"));
                assert_eq!(text.matches("same").count(), 1);
            }
        }
    }

    #[test]
    fn message_added_reconciliation_keeps_the_active_assistant_stream() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_started".to_string(),
            raw: json!({"message_id": "active"}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"message_id": "active", "ops": [{"op": "append_content", "text": "before"}]}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"index": 0, "message": {
                "message_id": "active",
                "role": "assistant",
                "content": "before",
                "stream_finished": false,
            }}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"message_id": "active", "ops": [{"op": "append_content", "text": " after"}]}),
        });

        assert_eq!(app.transcript_state().messages()[0].content, "before after");
        assert!(assistant_text(&app).contains("before after"));
    }

    #[test]
    fn authoritative_insertions_rebuild_native_and_alternate_history_in_server_order() {
        for native_scrollback in [false, true] {
            let mut app = App::new(project());
            app.set_native_scrollback(native_scrollback);
            if native_scrollback {
                app.pending_history_insertions(80);
            }
            app.handle_chat_event(snapshot_event(
                &app,
                vec![
                    json!({"message_id": "a1", "role": "assistant", "content": "one", "stream_finished": true}),
                    json!({"message_id": "a3", "role": "assistant", "content": "three", "stream_finished": true}),
                ],
            ));
            if native_scrollback {
                app.pending_history_insertions(80);
                app.resize_reflow.clear_pending_reflow();
            }

            for (index, message_id, content) in
                [(1, "a2", "middle"), (0, "a0", "start"), (4, "a4", "end")]
            {
                app.handle_chat_event(ChatEvent {
                    chat_id: Some(app.chat_id().to_string()),
                    seq: None,
                    kind: "message_added".to_string(),
                    raw: json!({"index": index, "message": {
                        "message_id": message_id,
                        "role": "assistant",
                        "content": content,
                        "stream_finished": true,
                    }}),
                });
            }

            assert_eq!(
                app.transcript_state()
                    .messages()
                    .iter()
                    .map(|message| message.message_id.as_deref())
                    .collect::<Vec<_>>(),
                [Some("a0"), Some("a1"), Some("a2"), Some("a3"), Some("a4")],
            );
            if native_scrollback {
                assert!(app.resize_reflow_is_due());
                let text = app
                    .resize_reflow_insertions(80)
                    .into_iter()
                    .flat_map(|insertion| insertion.lines)
                    .map(|line| line_to_plain_string(&line.line))
                    .collect::<Vec<_>>()
                    .join("\n");
                for content in ["start", "one", "middle", "three", "end"] {
                    assert_eq!(text.matches(content).count(), 1, "{content}: {text}");
                }
            } else {
                assert_eq!(assistant_cell_count(&app), 5);
                assert_eq!(assistant_text(&app), "startonemiddlethreeend");
            }
        }
    }

    #[test]
    fn authoritative_correction_rebuilds_drained_native_and_alternate_history_once() {
        for native_scrollback in [false, true] {
            let mut app = App::new(project());
            app.set_native_scrollback(native_scrollback);
            if native_scrollback {
                app.pending_history_insertions(80);
            }
            app.handle_chat_event(snapshot_event(
                &app,
                vec![json!({
                    "message_id": "a1",
                    "role": "assistant",
                    "content": "stale",
                    "stream_finished": true,
                })],
            ));
            if native_scrollback {
                app.pending_history_insertions(80);
                app.resize_reflow.clear_pending_reflow();
            }

            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_updated".to_string(),
                raw: json!({"message_id": "a1", "message": {
                    "message_id": "a1",
                    "role": "assistant",
                    "content": "corrected",
                    "stream_finished": true,
                }}),
            });

            if native_scrollback {
                assert!(app.resize_reflow_is_due());
                let text = app
                    .resize_reflow_insertions(80)
                    .into_iter()
                    .flat_map(|insertion| insertion.lines)
                    .map(|line| line_to_plain_string(&line.line))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(text.matches("corrected").count(), 1);
                assert!(!text.contains("stale"));
            } else {
                assert_eq!(assistant_cell_count(&app), 1);
                assert_eq!(assistant_text(&app), "corrected");
            }
        }
    }

    #[test]
    fn identical_user_echoes_with_distinct_client_message_ids_remain_distinct() {
        let mut app = App::new(project());
        app.transcript_state
            .push_optimistic_user_message("same", "client-message-1");
        app.transcript_state
            .push_optimistic_user_message("same", "client-message-2");
        app.rebuild_render_transcript_from_state();

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "server-user-1",
                "role": "user",
                "content": "same",
                "extra": {"client_message_id": "client-message-1"},
            }}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "server-user-2",
                "role": "user",
                "content": "same",
                "extra": {"client_message_id": "client-message-2"},
            }}),
        });

        let users = app
            .transcript_state()
            .messages()
            .iter()
            .filter(|message| message.role == TranscriptRole::User)
            .collect::<Vec<_>>();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].message_id.as_deref(), Some("server-user-1"));
        assert_eq!(users[1].message_id.as_deref(), Some("server-user-2"));
    }

    #[test]
    fn user_echo_with_unknown_client_message_id_appends_normally() {
        let mut app = App::new(project());
        app.transcript_state
            .push_optimistic_user_message("same", "client-message-known");
        app.rebuild_render_transcript_from_state();

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "server-user-unknown",
                "role": "user",
                "content": "same",
                "extra": {"client_message_id": "client-message-unknown"},
            }}),
        });

        assert_eq!(
            app.transcript_state()
                .messages()
                .iter()
                .filter(|message| message.role == TranscriptRole::User)
                .count(),
            2
        );
    }

    #[test]
    fn user_echo_without_client_message_id_does_not_guess_by_content() {
        let mut app = App::new(project());
        app.transcript_state
            .push_optimistic_user_message("same", "client-message-known");
        app.rebuild_render_transcript_from_state();

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "legacy-server-user",
                "role": "user",
                "content": "same",
            }}),
        });

        let users = app
            .transcript_state()
            .messages()
            .iter()
            .filter(|message| message.role == TranscriptRole::User)
            .collect::<Vec<_>>();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].message_id, None);
        assert_eq!(users[1].message_id.as_deref(), Some("legacy-server-user"));
    }

    #[test]
    fn unrelated_ack_does_not_complete_current_send() {
        let mut app = App::new(project());
        app.composer.set_text("hello");
        let (prompt, params, correlation) = match app.handle_key(key(KeyCode::Enter)) {
            AppAction::SendMessage {
                prompt,
                params,
                correlation,
            } => (prompt, params, correlation),
            other => panic!("unexpected action: {other:?}"),
        };

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "ack".to_string(),
            raw: json!({"client_request_id": "stale-request", "accepted": true}),
        });
        app.handle_command_finished(
            CommandContextTag::SendMessage {
                prompt,
                params,
                correlation,
            },
            Err("request timed out".to_string()),
        );

        assert_eq!(app.session_state(), SessionState::Idle);
        assert_eq!(app.composer(), "hello");
        assert!(app.pending_send_retry.is_some());
    }

    #[test]
    fn new_logical_send_gets_a_new_correlation_pair() {
        let mut app = App::new(project());
        app.composer.set_text("first");
        let first = match app.handle_key(key(KeyCode::Enter)) {
            AppAction::SendMessage { correlation, .. } => correlation,
            other => panic!("unexpected action: {other:?}"),
        };
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.composer.set_text("second");
        let second = match app.handle_key(key(KeyCode::Enter)) {
            AppAction::SendMessage { correlation, .. } => correlation,
            other => panic!("unexpected action: {other:?}"),
        };

        assert_ne!(first.client_request_id, second.client_request_id);
        assert_ne!(first.client_message_id, second.client_message_id);
    }

    #[test]
    fn abort_success_dispatches_next_queued_input_once() {
        let mut app = App::new(project());
        app.composer.set_text("first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { prompt, .. } if prompt == "first"
        ));
        app.composer.set_text("second");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        app.composer.set_text("third");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.input_queue().len(), 2);
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::Abort);
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(app.abort_in_flight);

        let action = app.handle_command_finished(
            CommandContextTag::Abort {
                origin: app.command_origin(),
            },
            Ok(()),
        );

        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "second"
        ));
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(!app.abort_in_flight);
        assert_eq!(app.input_queue().len(), 1);
        assert_eq!(app.input_queue().items()[0].text, "third");
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: app.command_origin(),
                },
                Ok(()),
            ),
            AppAction::None
        );
        assert_eq!(app.input_queue().len(), 1);
    }

    #[test]
    fn abort_success_with_empty_queue_stays_idle() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::Abort);
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(app.abort_in_flight);

        let action = app.handle_command_finished(
            CommandContextTag::Abort {
                origin: app.command_origin(),
            },
            Ok(()),
        );

        assert_eq!(action, AppAction::None);
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        assert!(app.input_queue().is_empty());
    }

    #[test]
    fn queued_item_can_be_edited_and_removed() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        app.composer.set_text("draft");
        app.handle_key(key(KeyCode::Enter));
        app.composer.set_text("keep draft");
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.input_queue().selected_index(), Some(0));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.composer(), "draft");
        app.composer.set_text("edited");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.composer(), "keep draft");
        assert_eq!(app.input_queue().items()[0].text, "edited");

        app.composer.clear();
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.handle_key(key(KeyCode::Delete)), AppAction::None);
        assert!(app.input_queue().is_empty());
    }

    #[test]
    fn delete_with_draft_does_not_remove_selected_queue_item() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        app.composer.set_text("queued");
        app.handle_key(key(KeyCode::Enter));
        app.composer.set_text("draft");
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.input_queue().selected_index(), Some(0));

        assert_eq!(app.handle_key(key(KeyCode::Delete)), AppAction::None);

        assert_eq!(app.input_queue().len(), 1);
        assert_eq!(app.input_queue().items()[0].text, "queued");
        assert_eq!(app.composer(), "draf");
        assert_eq!(app.input_queue().selected_index(), None);
    }

    #[test]
    fn abort_retains_queue_and_restores_edit_draft() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        app.composer.set_text("queued");
        app.handle_key(key(KeyCode::Enter));
        app.composer.set_text("draft");
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.composer(), "queued");

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            AppAction::None
        );
        assert_eq!(app.composer(), "draft");
        assert_eq!(app.input_queue().len(), 1);

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            AppAction::Abort
        );
        assert_eq!(app.composer(), "draft");
        assert_eq!(app.input_queue().len(), 1);
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(app.abort_in_flight);
    }

    #[test]
    fn finishing_queue_edit_while_idle_dispatches_next_item() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        app.composer.set_text("queued");
        app.handle_key(key(KeyCode::Enter));
        app.composer.set_text("draft");
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));
        app.set_session_state(SessionState::Idle);
        app.composer.set_text("edited");

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(matches!(
            action,
            AppAction::SendMessage { prompt, .. } if prompt == "edited"
        ));
        assert!(app.input_queue().is_empty());
        assert_eq!(app.composer(), "draft");
    }

    #[test]
    fn queue_updated_events_are_passive_server_state() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "queue_updated".to_string(),
            raw: json!({"queue_size": 1, "queued_items": [{"preview": "server-side"}]}),
        });

        assert_eq!(app.server_queue_size(), 1);
        assert_eq!(app.server_queue_previews(), &["server-side".to_string()]);
        assert!(app.input_queue().is_empty());
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
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::LoadDaemonStatus
        );
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| matches!(item, TranscriptItem::Status(_, _))));
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
    fn misc_command_group_parses_and_dispatches() {
        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("events"),
            AppAction::RefreshWorkers
        );
        assert!(app.events_pane().open);
        assert_eq!(
            app.execute_command_name("events"),
            AppAction::RefreshWorkers
        );
        assert!(!app.events_pane().open);

        assert_eq!(app.execute_command_name("help"), AppAction::None);
        assert!(app.help_open());

        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("keymap"), AppAction::None);
        assert!(app.help_open());

        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("vim"), AppAction::None);
        assert!(app.vim_enabled());

        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("debug-config"), AppAction::None);
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Info(lines) if lines.iter().any(|line| line.contains("Commands:")))
        }));

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("quit"),
            AppAction::Quit {
                abort_active: false
            }
        );
        assert!(app.should_quit());
    }

    #[test]
    fn worker_list_with_null_ports_reaches_events_pane() {
        let mut app = App::new(project());
        let worker: WorkerInfo = serde_json::from_value(json!({
            "project_id": "stopping",
            "slug": "stopping-project",
            "root": "/tmp/stopping",
            "root_exists": false,
            "pinned": false,
            "last_active_ms": 0,
            "state": "stopping",
            "pid": null,
            "rss_bytes": null,
            "cpu_percent": null,
            "uptime_secs": null,
            "http_port": null,
            "lsp_port": null,
            "lsp_clients": 0,
            "busy_chats": 0,
            "exec_running": 0,
            "live_proxy_streams": 0,
            "cron_next_fire_ms": null,
            "idle_deadline_ms": null,
            "last_status_report_ms": null,
            "last_error": null,
            "log_path": "/tmp/stopping.log"
        }))
        .unwrap();

        app.set_workers(vec![worker]);

        assert_eq!(app.events_pane().workers().len(), 1);
        assert_eq!(app.events_pane().workers()[0].http_port, None);
        assert_eq!(app.events_pane().workers()[0].lsp_port, None);
    }

    #[test]
    fn theme_command_applies_theme_live() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("tui.toml");
        let mut app = App::new(project());
        app.test_set_tui_config_path(config_path.clone());
        assert_eq!(app.theme().name(), "dark");
        assert_eq!(app.execute_command_name("theme light"), AppAction::None);
        assert_eq!(app.theme().name(), "light");
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("Theme set to light"))
        }));
        assert!(std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("name = \"light\""));

        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("theme"), AppAction::None);
        let picker = app.modal_picker().unwrap();
        assert_eq!(picker.kind, PickerKind::Theme);
        assert_eq!(picker.filtered_items()[0].id, "dark");
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.theme().name(), "light");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.theme().name(), "light");
    }

    #[test]
    fn theme_picker_cancel_restores_previewed_theme() {
        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("theme"), AppAction::None);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.theme().name(), "light");

        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);

        assert!(app.modal_picker().is_none());
        assert_eq!(app.theme().name(), "dark");
    }

    #[test]
    fn theme_config_helpers_update_or_create_theme_section() {
        assert_eq!(
            surfaces::apply_theme_name_to_config("", "light"),
            "[theme]\nname = \"light\"\n"
        );
        assert_eq!(
            surfaces::apply_theme_name_to_config(
                "vim = true\n\n[theme]\naccent = \"cyan\"\n",
                "plain"
            ),
            "vim = true\n\n[theme]\naccent = \"cyan\"\nname = \"plain\"\n"
        );
        assert_eq!(
            surfaces::apply_theme_name_to_config(
                "[theme]\nname = \"dark\"\n\n[bindings]\nsend = \"ctrl-s\"\n",
                "light"
            ),
            "[theme]\nname = \"light\"\n\n[bindings]\nsend = \"ctrl-s\"\n"
        );
    }

    #[test]
    fn help_command_shows_generated_keymap_help() {
        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("help"), AppAction::None);
        assert!(app.help_open());
        let rows = app.keymap_help_rows();
        assert!(rows
            .iter()
            .any(|row| row.action == Some(KeyAction::ShowHelp)));
        assert!(rows
            .iter()
            .any(|row| row.description.contains("keymap help")));
    }

    #[test]
    fn read_only_view_commands_open_loading_overlays() {
        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("mcp"),
            AppAction::LoadReadOnlyView {
                view: ReadOnlyView::Mcp
            }
        );
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "MCP" && overlay.lines().join("\n").contains("Loading /mcp")
        }));

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("skills"),
            AppAction::LoadReadOnlyView {
                view: ReadOnlyView::Skills
            }
        );
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Skills" && overlay.lines().join("\n").contains("Loading /skills")
        }));

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("memories"),
            AppAction::LoadReadOnlyView {
                view: ReadOnlyView::Memories
            }
        );
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Memories"
                && overlay.lines().join("\n").contains("Loading /memories")
        }));

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("hooks"),
            AppAction::LoadReadOnlyView {
                view: ReadOnlyView::Hooks
            }
        );
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Hooks" && overlay.lines().join("\n").contains("Loading /hooks")
        }));

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("import"),
            AppAction::LoadCompetitorImportSources
        );
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Import" && overlay.lines().join("\n").contains("Loading /import")
        }));
    }

    #[test]
    fn board_command_is_feature_gated_and_uses_current_task() {
        let previous = std::env::var_os("REFACT_TUI_SURFACES");
        std::env::remove_var("REFACT_TUI_SURFACES");
        let mut app = App::new(project());
        app.test_set_task_context(Some("task-1".to_string()), Some("task_planner".to_string()));
        assert_eq!(app.execute_command_name("board"), AppAction::None);

        std::env::set_var("REFACT_TUI_SURFACES", "1");
        assert_eq!(
            app.execute_command_name("tasks"),
            AppAction::LoadTaskBoard {
                task_id: Some("task-1".to_string())
            }
        );
        match previous {
            Some(value) => std::env::set_var("REFACT_TUI_SURFACES", value),
            None => std::env::remove_var("REFACT_TUI_SURFACES"),
        }
    }

    #[test]
    fn raw_command_opens_copy_mode_overlay() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "hello"},
                {
                    "message_id": "a1",
                    "role": "assistant",
                    "content": "hi",
                    "tool_calls": [{"id": "call-1", "function": {"name": "t_process_start", "arguments": "{}"}}]
                }
            ]}),
        });
        assert_eq!(app.execute_command_name("raw"), AppAction::None);
        let overlay = app.transcript_overlay().unwrap();
        assert!(overlay.is_copy_mode());
        assert!(overlay.lines().join("\n").contains("t_process_start"));
    }

    #[test]
    fn read_only_view_loaded_opens_rendered_overlay() {
        let mut app = App::new(project());
        app.handle_skills_view_loaded(Ok(SlashCommandsListResponse {
            commands: Vec::new(),
            skills: vec![crate::client::SkillInfo {
                name: "explain".to_string(),
                description: "Explain code".to_string(),
                user_invocable: Some(true),
                source: "project_refact".to_string(),
            }],
        }));

        let overlay = app.transcript_overlay().unwrap();
        assert_eq!(overlay.title(), "Skills");
        assert!(overlay.lines().join("\n").contains("/explain"));
    }

    #[test]
    fn hooks_view_loaded_opens_rendered_overlay() {
        let mut app = App::new(project());
        app.handle_hooks_view_loaded(Ok(HooksResponse {
            hooks: vec![crate::client::HookInfo {
                event: "PreToolUse".to_string(),
                matcher: Some("Bash".to_string()),
                command: "./check.sh".to_string(),
                timeout: Some(30),
            }],
            raw_content: "hooks: {}".to_string(),
            file_path: "/repo/.refact/hooks.yaml".to_string(),
        }));

        let overlay = app.transcript_overlay().unwrap();
        assert_eq!(overlay.title(), "Hooks");
        assert!(overlay.lines().join("\n").contains("./check.sh"));
    }

    #[test]
    fn logout_command_dispatches_arg_or_picker_load() {
        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("logout"),
            AppAction::LoadProviderLogoutChoices
        );

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("logout openai_codex"),
            AppAction::ProviderOAuthLogout {
                provider: "openai_codex".to_string()
            }
        );
    }

    #[test]
    fn logout_provider_picker_filters_oauth_credentials() {
        let mut app = App::new(project());
        app.handle_provider_logout_choices_loaded(Ok(ProviderListResponse {
            providers: vec![
                crate::client::ProviderListItem {
                    name: "openai_codex".to_string(),
                    base_provider: "openai_codex".to_string(),
                    display_name: "OpenAI Codex".to_string(),
                    enabled: Some(true),
                    readonly: Some(false),
                    has_credentials: Some(true),
                    status: "configured".to_string(),
                    model_count: Some(1),
                },
                crate::client::ProviderListItem {
                    name: "openrouter".to_string(),
                    base_provider: "openrouter".to_string(),
                    display_name: "OpenRouter".to_string(),
                    enabled: Some(true),
                    readonly: Some(false),
                    has_credentials: Some(true),
                    status: "configured".to_string(),
                    model_count: Some(1),
                },
            ],
        }));

        let picker = app.modal_picker().unwrap();
        assert_eq!(picker.kind, PickerKind::ProviderLogout);
        assert_eq!(picker.filtered_items().len(), 1);
        assert_eq!(picker.filtered_items()[0].id, "openai_codex");
    }

    #[test]
    fn provider_logout_finished_reports_status() {
        let mut app = App::new(project());
        app.handle_provider_logout_finished(
            "openai_codex".to_string(),
            Ok(ProviderOAuthLogoutResponse {
                success: Some(true),
                auth_status: "No credentials found".to_string(),
            }),
        );

        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("Logged out openai_codex"))
        }));
    }

    #[test]
    fn import_command_dispatches_list_and_run_forms() {
        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("import"),
            AppAction::LoadCompetitorImportSources
        );

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("import claude_code global"),
            AppAction::RunCompetitorImport {
                source: Some("claude_code".to_string()),
                scope: "global".to_string(),
            }
        );

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("import all"),
            AppAction::RunCompetitorImport {
                source: None,
                scope: "project".to_string(),
            }
        );
    }

    #[test]
    fn import_sources_loaded_opens_picker_and_overlay() {
        let mut app = App::new(project());
        app.handle_competitor_import_sources_loaded(Ok(CompetitorImportInfoResponse {
            sources: vec![crate::client::CompetitorImportSourceInfo {
                id: "claude_code".to_string(),
                label: "Claude Code".to_string(),
                roots: vec!["~/.claude".to_string()],
            }],
        }));

        let picker = app.modal_picker().unwrap();
        assert_eq!(picker.kind, PickerKind::CompetitorImport);
        assert!(picker
            .filtered_items()
            .iter()
            .any(|item| item.id == "claude_code"));
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Import" && overlay.lines().join("\n").contains("claude_code")
        }));
    }

    #[test]
    fn import_finished_reports_notice_and_overlay() {
        let mut status_counts = std::collections::BTreeMap::new();
        status_counts.insert(crate::client::ImportStatus::Created, 1);
        let mut app = App::new(project());
        app.handle_competitor_import_finished(Ok(CompetitorImportRunResponse {
            scope: "project".to_string(),
            source: Some("claude_code".to_string()),
            report: crate::client::ImportReport {
                completed_at: None,
                reported_sources: Vec::new(),
                discovered_candidates: Some(1),
                status_counts,
                competitor_counts: std::collections::BTreeMap::new(),
                kind_counts: std::collections::BTreeMap::new(),
                top_issues: Vec::new(),
            },
        }));

        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("/import claude_code project complete"))
        }));
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Import" && overlay.lines().join("\n").contains("created 1")
        }));
    }

    #[test]
    fn read_only_view_failure_reports_explicit_notice_and_overlay() {
        let mut app = App::new(project());
        app.handle_memories_view_loaded(Err("boom".to_string()));

        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("Failed to load /memories: boom"))
        }));
        assert!(app.transcript_overlay().is_some_and(|overlay| {
            overlay.title() == "Memories" && overlay.lines().join("\n").contains("boom")
        }));
    }

    #[test]
    fn copy_command_emits_last_assistant_as_osc52_payload() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "hello"},
                {"message_id": "a1", "role": "assistant", "content": "old"},
                {"message_id": "a2", "role": "assistant", "content": "new"}
            ]}),
        });

        let action = app.execute_command_name("copy");

        let AppAction::CopyToClipboard { text, source } = action else {
            panic!("expected copy action");
        };
        assert_eq!(source, ClipboardCopySource::LastAssistant);
        assert_eq!(text, "new");
        let mut output = Vec::new();
        let report = crate::clipboard::write_osc52_copy(&mut output, &text, false).unwrap();
        assert_eq!(output, b"\x1b]52;c;bmV3\x07");
        assert_eq!(report.copied_bytes, 3);
        assert!(!report.truncated);
    }

    #[test]
    fn overlay_y_yanks_visible_raw_text() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "hello"},
                {"message_id": "a1", "role": "assistant", "content": "hi"}
            ]}),
        });
        app.execute_command_name("raw");
        app.set_transcript_overlay_visible_height(2);

        let action = app.handle_key(key(KeyCode::Char('y')));

        assert!(matches!(
            action,
            AppAction::CopyToClipboard { text, source }
                if source == ClipboardCopySource::OverlayVisible && text == "## user u1\nhello"
        ));
    }

    #[test]
    fn permissions_command_uses_multi_select_picker() {
        let mut app = App::new(project());
        app.test_open_permissions_picker();
        let picker = app.modal_picker().unwrap();
        assert!(picker.is_multi());
        assert_eq!(picker.kind, PickerKind::Permissions);
        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SetParams {
                patch: json!({"auto_approve_editing_tools": true, "auto_approve_dangerous_commands": true})
            }
        );
        assert_eq!(
            app.permission_policy(),
            session::PermissionPolicy {
                auto_approve_editing_tools: true,
                auto_approve_dangerous_commands: true,
            }
        );
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("TUI sends Allow Once") && text.contains("server enforces"))
        }));
    }

    #[test]
    fn session_command_group_parses_and_dispatches() {
        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("new"), AppAction::SubscribeCurrent);
        assert_eq!(app.execute_command_name("resume"), AppAction::LoadSessions);
        assert_eq!(app.execute_command_name("model"), AppAction::LoadModels);
        assert_eq!(app.execute_command_name("mode"), AppAction::LoadModes);
        assert_eq!(app.execute_command_name("permissions"), AppAction::None);
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Notice(text)) if text == "/permissions requires REFACT_TUI_SURFACES=1"
        ));

        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "gpt-demo": {"reasoning_effort_options": ["low", "medium", "high"]}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-demo"}, "runtime": {"state": "idle"}, "messages": []}),
        });
        assert_eq!(app.execute_command_name("reasoning"), AppAction::None);
        assert!(app
            .modal_picker()
            .is_some_and(|picker| picker.kind == PickerKind::Reasoning));

        let mut app = App::new(project());
        assert_eq!(
            app.execute_command_name("status"),
            AppAction::LoadDaemonStatus
        );
        assert!(app
            .visible_transcript()
            .iter()
            .any(|item| matches!(item, TranscriptItem::Status(_, _))));

        let mut app = App::new(project());
        assert!(matches!(
            app.execute_command_name("init"),
            AppAction::SendMessage { prompt, .. } if prompt.contains("bootstrap this project")
        ));
    }

    #[test]
    fn model_command_opens_picker_after_caps_load() {
        let mut app = App::new(project());
        assert_eq!(app.execute_command_name("model"), AppAction::LoadModels);
        app.open_model_picker(json!({"chat_models": {"m1": {"name": "Model One"}}}));
        let picker = app.modal_picker().unwrap();
        assert_eq!(picker.kind, PickerKind::Model);
        assert_eq!(picker.filtered_items()[0].id, "m1");
    }

    #[test]
    fn caps_loaded_default_model_populates_footer_model_and_mode() {
        let mut app = App::new(project());

        app.apply_caps(&json!({
            "defaults": {
                "chat_default_model": "openai/gpt-demo",
                "chat_default_mode": "agent"
            },
            "chat_models": {
                "openai/gpt-demo": {"name": "GPT Demo", "n_ctx": 128_000}
            }
        }));

        assert_eq!(app.model(), Some("openai/gpt-demo"));
        assert_eq!(app.mode(), Some("agent"));
        let footer = crate::ui::footer::FooterData::from_app(&app);
        assert_eq!(footer.model, "openai/gpt-demo");
        assert_ne!(footer.model, "default");
        assert_eq!(footer.mode, "agent");
    }

    #[test]
    fn empty_snapshot_keeps_caps_default_but_thread_updates_still_win() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "defaults": {"chat_default_model": "openai/gpt-demo"},
            "chat_models": {"openai/gpt-demo": {"name": "GPT Demo"}}
        }));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "", "mode": ""}, "messages": []}),
        });
        assert_eq!(app.model(), Some("openai/gpt-demo"));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "thread_updated".to_string(),
            raw: json!({"params": {"model": "openai/gpt-other", "mode": "chat"}}),
        });
        assert_eq!(app.model(), Some("openai/gpt-other"));
        assert_eq!(app.mode(), Some("chat"));
    }

    #[test]
    fn model_and_mode_pickers_preselect_current_values() {
        let mut app = App::new(project());
        app.handle_thread_updated(&json!({"model": "openai/gpt-small", "mode": "task_agent"}));

        app.open_model_picker(json!({"chat_models": {
            "openai/gpt-demo": {"name": "GPT Demo"},
            "openai/gpt-small": {"name": "GPT Small"}
        }}));
        assert_eq!(
            app.modal_picker().unwrap().selected_item().unwrap().id,
            "openai/gpt-small"
        );

        app.test_open_mode_picker(json!({"modes": [
            {"id": "agent", "title": "Agent"},
            {"id": "task_agent", "title": "Task Agent"}
        ]}));
        assert_eq!(
            app.modal_picker().unwrap().selected_item().unwrap().id,
            "task_agent"
        );
    }

    #[test]
    fn status_command_snapshot_includes_daemon_worker_session_and_usage() {
        let mut app = App::new(project());
        app.test_set_daemon_status(
            DaemonStatus {
                pid: Some(7),
                version: Some("1.2.3".to_string()),
                port: Some(8488),
                started_at_ms: Some(10),
                uptime_secs: Some(20),
                workers: Some(1),
                cron_pending: Some(HashMap::new()),
            },
            "http://127.0.0.1:8488",
        );
        app.set_workers(vec![WorkerInfo {
            project_id: "p1".to_string(),
            pid: Some(42),
            http_port: Some(9000),
            lsp_port: Some(9001),
            state: Value::String("ready".to_string()),
            last_error: None,
            ..WorkerInfo::default()
        }]);
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({
                "thread": {
                    "title": "Status chat",
                    "model": "gpt-demo",
                    "mode": "agent",
                    "auto_approve_editing_tools": true,
                    "auto_approve_dangerous_commands": false
                },
                "runtime": {
                    "state": "idle",
                    "usage": {"prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150}
                },
                "messages": []
            }),
        });
        app.apply_caps(&json!({"chat_models": {"gpt-demo": {"n_ctx": 1000}}}));
        assert_eq!(
            app.execute_command_name("status"),
            AppAction::LoadDaemonStatus
        );
        let status = app
            .visible_transcript()
            .iter()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Status(snapshot, theme) => Some((snapshot, theme)),
                _ => None,
            })
            .unwrap();
        let text = crate::ui::status_card::render_lines(100, status.0, status.1)
            .iter()
            .map(line_to_plain_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("refact (v"));
        assert!(text.contains("Daemon:"));
        assert!(text.contains("v1.2.3 on port 8488"));
        assert!(text.contains("Worker:"));
        assert!(text.contains("ready · pid 42 · http 9000 · lsp 9001"));
        assert!(text.contains("Model:"));
        assert!(text.contains("gpt-demo"));
        assert!(text.contains("Mode:"));
        assert!(text.contains("agent"));
        assert!(text.contains("Reasoning:"));
        assert!(text.contains("off"));
        assert!(text.contains("Directory:"));
        assert!(text.contains("/tmp/demo"));
        assert!(text.contains("Permissions:"));
        assert!(text.contains("auto_approve_editing_tools=true"));
        assert!(text.contains("auto_approve_dangerous_commands=false"));
        assert!(text.contains("Token usage:"));
        assert!(text.contains("150 total (100 input + 50 output)"));
        assert!(text.contains("Context window:"));
        assert!(text.contains("85% left (150/1K)"));
        assert_eq!(
            app.permission_policy(),
            session::PermissionPolicy {
                auto_approve_editing_tools: true,
                auto_approve_dangerous_commands: false,
            }
        );
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
            matches!(item, TranscriptItem::Plan(data) if data.content.contains("base plan") && data.content.contains("delta one"))
        }));
    }

    #[test]
    fn goal_command_shows_current_goal_cell() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {"role": "goal", "content": "base goal", "extra": {"goal": {"version": 1}}}}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {"role": "event", "content": "delta one", "extra": {"event": {"subkind": "goal_delta", "payload": {"seq": 1}}}}}),
        });

        assert_eq!(app.execute_command_name("goal"), AppAction::None);
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Goal(data) if data.content.contains("base goal") && data.content.contains("delta one"))
        }));
    }

    #[test]
    fn goal_commands_dispatch_all_four_wrappers_and_controls_respect_status() {
        let _surface_lock = goal_dock::test_surface_lock();
        std::env::set_var("REFACT_TUI_SURFACES", "1");
        let mut app = App::new(project());
        assert!(matches!(
            app.execute_command_name("goal set Ship it"),
            AppAction::GoalCommand {
                kind: GoalCommandKind::Set,
                content: Some(content),
                budget: None,
            } if content == "Ship it"
        ));
        assert!(matches!(
            app.execute_command_name("goal update Tests added"),
            AppAction::GoalCommand {
                kind: GoalCommandKind::Update,
                content: Some(content),
                budget: None,
            } if content == "Tests added"
        ));
        assert!(matches!(
            app.execute_command_name("goal budget 3 5 1000 42 2"),
            AppAction::GoalCommand {
                kind: GoalCommandKind::SetBudget,
                budget: Some(budget),
                ..
            } if budget.max_turns == Some(3) && budget.max_tokens == Some(1000)
        ));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({
                "runtime": {"state": "idle"},
                "messages": [{
                    "role": "goal",
                    "content": "Ship it",
                    "extra": {"goal": {"status": "active", "active": true}}
                }]
            }),
        });
        assert_eq!(
            app.execute_command_name("goal pause"),
            AppAction::GoalControl {
                action: GoalControlAction::Pause
            }
        );
        assert_eq!(
            app.execute_command_name("goal stop"),
            AppAction::GoalControl {
                action: GoalControlAction::Stop
            }
        );
        assert_eq!(app.execute_command_name("goal resume"), AppAction::None);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: json!({"state": "idle", "goal_status": "paused", "goal_active": false}),
        });
        assert_eq!(
            app.execute_command_name("goal resume"),
            AppAction::GoalControl {
                action: GoalControlAction::Resume
            }
        );
        std::env::remove_var("REFACT_TUI_SURFACES");
    }

    #[test]
    fn goal_overlay_uses_goal_key_context_and_dispatches_controls() {
        let _surface_lock = goal_dock::test_surface_lock();
        std::env::set_var("REFACT_TUI_SURFACES", "1");
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({
                "runtime": {"state": "idle"},
                "messages": [{
                    "role": "goal",
                    "content": "Ship it",
                    "extra": {"goal": {"status": "active", "active": true}}
                }]
            }),
        });
        assert_eq!(app.execute_command_name("goal"), AppAction::None);
        assert!(app.test_goal_overlay_open());
        assert_eq!(app.focused_key_context(), KeyContext::Goal);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('p'))),
            AppAction::GoalControl {
                action: GoalControlAction::Pause
            }
        );
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);
        assert!(!app.test_goal_overlay_open());
        std::env::remove_var("REFACT_TUI_SURFACES");
    }

    #[tokio::test]
    async fn goal_commands_post_all_four_goal_wire_commands() {
        let _surface_lock = goal_dock::test_surface_lock();
        std::env::set_var("REFACT_TUI_SURFACES", "1");
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        let (tx, mut rx) = mpsc::channel(4);
        let mut subscriptions = SubscriptionManager::new();
        let mut daemon_events = DaemonEventSubscription::new();

        for action in [
            app.execute_command_name("goal set Ship it"),
            app.execute_command_name("goal update Add tests"),
            app.execute_command_name("goal budget"),
        ] {
            run_action(
                &mut app,
                action,
                &client,
                &tx,
                &mut subscriptions,
                &mut daemon_events,
            )
            .await;
            assert!(matches!(
                rx.recv().await,
                Some(RuntimeEvent::CommandFinished { result: Ok(()), .. })
            ));
        }
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({
                "runtime": {"state": "idle"},
                "messages": [{
                    "role": "goal",
                    "content": "Ship it",
                    "extra": {"goal": {"status": "active", "active": true}}
                }]
            }),
        });
        let pause_action = app.execute_command_name("goal pause");
        run_action(
            &mut app,
            pause_action,
            &client,
            &tx,
            &mut subscriptions,
            &mut daemon_events,
        )
        .await;
        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::CommandFinished { result: Ok(()), .. })
        ));

        assert!(state.find_command("set_goal").is_some());
        assert!(state.find_command("update_goal").is_some());
        assert!(state.find_command("set_goal_budget").is_some());
        assert_eq!(
            state
                .find_command("goal_control")
                .and_then(|command| command.get("action").cloned()),
            Some(json!("pause"))
        );
        std::env::remove_var("REFACT_TUI_SURFACES");
    }

    #[test]
    fn review_and_compact_insert_structured_prompts() {
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
    fn reasoning_command_emits_set_params_and_updates_footer_state() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "openai/gpt-demo": {"reasoning_effort_options": ["low", "medium", "high"]}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "openai/gpt-demo", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        });

        let action = app.execute_command_name("reasoning high");

        assert_eq!(
            action,
            AppAction::SetParams {
                patch: json!({"boost_reasoning": true, "reasoning_effort": "high", "thinking_budget": null})
            }
        );
        assert_eq!(app.reasoning_effort_label(), "high");
        let footer = crate::ui::footer::footer_text(&crate::ui::footer::FooterData::from_app(&app));
        assert!(footer.contains("reason:high"));
    }

    #[test]
    fn reasoning_command_reports_unsupported_model_without_set_params() {
        let mut app = App::new(project());
        app.apply_caps(&json!({"chat_models": {"gpt-basic": {"reasoning_effort_options": null}}}));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-basic", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        });

        assert_eq!(app.execute_command_name("reasoning high"), AppAction::None);
        assert_eq!(app.reasoning_effort_label(), "off");
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("Reasoning capability is not confirmed"))
        }));
    }

    #[test]
    fn reasoning_command_is_guarded_mid_turn() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "gpt-demo": {"reasoning_effort_options": ["high"]}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-demo", "mode": "agent"}, "runtime": {"state": "generating"}, "messages": []}),
        });

        assert_eq!(app.execute_command_name("reasoning high"), AppAction::None);
        assert_eq!(app.reasoning_effort_label(), "off");
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("between turns only"))
        }));
    }

    #[test]
    fn reasoning_command_supports_boost_only_models() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "gpt-boost": {"supports_thinking_budget": true}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-boost", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        });

        let action = app.execute_command_name("reasoning on");

        assert_eq!(
            action,
            AppAction::SetParams {
                patch: json!({"boost_reasoning": true, "reasoning_effort": null, "thinking_budget": null})
            }
        );
        assert_eq!(app.reasoning_effort_label(), "on");
    }

    #[test]
    fn reasoning_command_failure_rolls_back_optimistic_state() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "gpt-demo": {"reasoning_effort_options": ["low", "high"]}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-demo", "mode": "agent", "boost_reasoning": true, "reasoning_effort": "low"}, "runtime": {"state": "idle"}, "messages": []}),
        });
        let action = app.execute_command_name("reasoning high");
        let AppAction::SetParams { patch } = action else {
            panic!("expected set params action");
        };
        assert_eq!(app.reasoning_effort_label(), "high");
        let context = app.set_params_context(&patch);

        let result = app.handle_command_finished(context, Err("backend rejected".to_string()));

        assert_eq!(result, AppAction::None);
        assert_eq!(app.reasoning_effort_label(), "low");
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("/reasoning failed") && text.contains("backend rejected"))
        }));
    }

    #[test]
    fn reasoning_picker_lists_only_supported_effort_levels() {
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "gpt-demo": {"reasoning_effort_options": ["low", "high", "xhigh"]}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-demo", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        });

        assert_eq!(app.execute_command_name("reasoning"), AppAction::None);

        let ids = app
            .modal_picker()
            .unwrap()
            .filtered_items()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["off", "on", "low", "high"]);
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
    fn resume_picker_accept_switches_chat_and_requests_snapshot() {
        let mut app = App::new(project());
        let previous = app.chat_id().to_string();
        app.open_session_picker(vec![PickerItem {
            id: "chat-resume".to_string(),
            title: "Saved chat".to_string(),
            description: "1h ago · gpt-demo · agent".to_string(),
        }]);

        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SubscribeCurrent
        );

        assert_eq!(app.chat_id(), "chat-resume");
        assert_ne!(app.chat_id(), previous);
        assert_eq!(app.session_title(), Some("Saved chat"));
        assert!(matches!(
            app.visible_transcript().first(),
            Some(TranscriptItem::Session { title, .. }) if title == "Saved chat"
        ));
        assert!(session_text(&app).contains("directory: /tmp/demo"));
        assert!(app
            .session_tabs()
            .iter()
            .any(|tab| { tab.id == "chat-resume" && tab.is_current }));
    }

    #[test]
    fn fork_command_emits_branch_from_chat_action() {
        let mut app = App::new(project());
        let source = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(source.clone()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"id": source, "title": "Base", "model": "gpt-demo", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "hello"},
                {"message_id": "a1", "role": "assistant", "content": "hi"}
            ]}),
        });

        let action = app.execute_command_name("fork");

        assert!(matches!(
            action,
            AppAction::ForkChat { source_chat_id, up_to_message_id, .. }
                if source_chat_id == source && up_to_message_id == "a1"
        ));
        assert_eq!(app.chat_id(), source);
    }

    #[test]
    fn rename_and_archive_commands_emit_session_actions() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();

        assert_eq!(
            app.execute_command_name("rename Better title"),
            AppAction::RenameChat {
                title: "Better title".to_string()
            }
        );
        assert_eq!(app.session_title(), None);

        let action = app.execute_command_name("archive");
        assert!(matches!(
            action,
            AppAction::ArchiveChat { chat_id: archived, .. } if archived == chat_id
        ));
        assert_eq!(app.chat_id(), chat_id);
    }

    #[test]
    fn command_finished_from_stale_generation_is_ignored() {
        let mut app = App::new(project());
        let mut subscriptions = SubscriptionManager::new();
        subscriptions.generation = 3;
        let old_chat_id = app.chat_id().to_string();
        let stale_title = "stale title".to_string();
        let origin = app.command_origin();

        assert!(subscriptions
            .apply_command_finished(
                &mut app,
                2,
                CommandContextTag::Rename {
                    origin: origin.clone(),
                    title: stale_title.clone(),
                },
                Ok(()),
            )
            .is_none());
        assert_eq!(app.session_title(), None);

        assert!(subscriptions
            .apply_command_finished(
                &mut app,
                3,
                CommandContextTag::Rename {
                    origin,
                    title: stale_title,
                },
                Ok(()),
            )
            .is_some());
        assert_eq!(app.chat_id(), old_chat_id);
        assert_eq!(app.session_title(), Some("stale title"));
    }

    #[test]
    fn daemon_event_from_stale_generation_is_ignored() {
        let mut app = App::new(project());
        let mut daemon_events = DaemonEventSubscription::new();
        daemon_events.generation = 2;
        let stale = DaemonEventRecord {
            ts_ms: Some(1),
            kind: "worker_ready".to_string(),
            project_id: Some("old".to_string()),
            payload: Value::Null,
        };
        let fresh = DaemonEventRecord {
            ts_ms: Some(2),
            kind: "worker_ready".to_string(),
            project_id: Some("p1".to_string()),
            payload: Value::Null,
        };

        assert!(!daemon_events.apply_event(&mut app, 1, stale));
        assert!(app.events_pane().events().is_empty());
        assert!(daemon_events.apply_event(&mut app, 2, fresh));
        assert_eq!(app.events_pane().events().len(), 1);
    }

    #[test]
    fn daemon_event_disconnect_notice_reports_bounded_reconnect() {
        let mut app = App::new(project());

        app.handle_daemon_events_disconnected("network down".to_string(), true);
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("bounded backoff"))
        }));

        app.handle_daemon_events_disconnected("network down".to_string(), false);
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Notice(text) if text.contains("reconnect limit reached"))
        }));
    }

    #[test]
    fn ctrl_c_abort_failure_restores_approval_and_ask_form() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-approval", "shell"));
        let request = AskQuestionsRequest::from_tool_content(
            r#"{"type":"ask_questions","tool_call_id":"call-ask","questions":[{"id":"confirm","type":"yes_no","text":"Proceed?"}]}"#,
            None,
        )
        .unwrap();
        app.test_set_ask_questions_form(AskQuestionsForm::new(request));
        assert!(app.approval_modal().is_some());
        assert!(app.ask_questions_form().is_some());

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert!(app.approval_modal().is_some());
        assert!(app.ask_questions_form().is_some());

        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: app.command_origin(),
                },
                Err("backend unavailable".to_string()),
            ),
            AppAction::None
        );
        assert!(!app.abort_in_flight);
        assert!(app.approval_modal().is_some());
        assert!(app.ask_questions_form().is_some());
    }

    #[test]
    fn stale_command_completions_are_silent_and_current_origin_applies() {
        let mut app = App::new(project());
        let origin = app.command_origin();
        app.abort_in_flight = true;
        app.set_session_state(SessionState::Generating);
        app.set_project(project_b());
        app.abort_in_flight = true;
        app.set_session_state(SessionState::Generating);
        let before = app.visible_transcript().len();

        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: origin.clone(),
                },
                Ok(()),
            ),
            AppAction::None
        );
        assert!(app.abort_in_flight);
        assert_eq!(app.session_state(), SessionState::Generating);
        assert_eq!(app.visible_transcript().len(), before);
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: origin.clone(),
                },
                Err("stale abort".to_string()),
            ),
            AppAction::None
        );
        assert!(app.abort_in_flight);
        assert_eq!(app.visible_transcript().len(), before);

        let stale_title = "Stale".to_string();
        for result in [Ok(()), Err("stale rename".to_string())] {
            assert_eq!(
                app.handle_command_finished(
                    CommandContextTag::Rename {
                        origin: origin.clone(),
                        title: stale_title.clone(),
                    },
                    result,
                ),
                AppAction::None
            );
        }
        assert_eq!(app.session_title(), None);

        let stale_fork = "stale-fork".to_string();
        for result in [Ok(()), Err("stale fork".to_string())] {
            assert_eq!(
                app.handle_command_finished(
                    CommandContextTag::Fork {
                        origin: origin.clone(),
                        target_chat_id: stale_fork.clone(),
                        title: None,
                    },
                    result,
                ),
                AppAction::None
            );
        }
        assert_ne!(app.chat_id(), stale_fork);

        let stale_archive = "stale-archive".to_string();
        for result in [Ok(()), Err("stale archive".to_string())] {
            assert_eq!(
                app.handle_command_finished(
                    CommandContextTag::Archive {
                        origin: origin.clone(),
                        chat_id: stale_archive.clone(),
                    },
                    result,
                ),
                AppAction::None
            );
        }
        assert_ne!(app.chat_id(), stale_archive);
        assert_eq!(app.visible_transcript().len(), before);

        let current = app.command_origin();
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: current.clone(),
                },
                Ok(()),
            ),
            AppAction::None
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);

        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Rename {
                    origin: current,
                    title: "Current".to_string(),
                },
                Ok(()),
            ),
            AppAction::None
        );
        assert_eq!(app.session_title(), Some("Current"));
    }

    #[test]
    fn failed_session_mutations_do_not_commit_local_state() {
        let mut app = App::new(project());
        let source_chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(source_chat_id.clone()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"id": source_chat_id, "title": "Base", "model": "gpt-demo", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "hello"},
                {"message_id": "a1", "role": "assistant", "content": "hi"}
            ]}),
        });
        let action = app.execute_command_name("fork");
        let (target_chat_id, title) = match action {
            AppAction::ForkChat {
                target_chat_id,
                title,
                ..
            } => (target_chat_id, title),
            other => panic!("unexpected action: {other:?}"),
        };
        assert_eq!(app.chat_id(), source_chat_id);
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Fork {
                    origin: app.command_origin(),
                    target_chat_id: target_chat_id.clone(),
                    title: title.clone(),
                },
                Err("branch failed".to_string()),
            ),
            AppAction::None
        );
        assert_eq!(app.chat_id(), source_chat_id);

        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Fork {
                    origin: app.command_origin(),
                    target_chat_id: target_chat_id.clone(),
                    title,
                },
                Ok(()),
            ),
            AppAction::SubscribeCurrent
        );
        assert_eq!(app.chat_id(), target_chat_id);

        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        assert_eq!(
            app.execute_command_name("rename Better title"),
            AppAction::RenameChat {
                title: "Better title".to_string()
            }
        );
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Rename {
                    origin: app.command_origin(),
                    title: "Better title".to_string(),
                },
                Err("rename failed".to_string()),
            ),
            AppAction::None
        );
        assert_eq!(app.session_title(), None);

        let action = app.execute_command_name("archive");
        let new_chat_id = match action {
            AppAction::ArchiveChat { new_chat_id, .. } => new_chat_id,
            other => panic!("unexpected action: {other:?}"),
        };
        assert_eq!(app.chat_id(), chat_id);
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Archive {
                    origin: app.command_origin(),
                    chat_id: new_chat_id.clone(),
                },
                Err("archive failed".to_string()),
            ),
            AppAction::None
        );
        assert_eq!(app.chat_id(), chat_id);
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Archive {
                    origin: app.command_origin(),
                    chat_id: new_chat_id.clone(),
                },
                Ok(()),
            ),
            AppAction::SubscribeCurrent
        );
        assert_eq!(app.chat_id(), new_chat_id);
    }

    #[test]
    fn project_and_chat_switch_clear_pending_params() {
        let mut app = App::new(project());
        app.pending_model = Some("model-a".to_string());
        app.pending_mode = Some("mode-a".to_string());
        app.pending_send_retry = Some(PendingSendRetry {
            prompt: "retry".to_string(),
            params: json!({"model": "old"}),
            correlation: ClientMessageCorrelation {
                client_request_id: "old-request".to_string(),
                client_message_id: "old-message".to_string(),
            },
        });

        app.new_chat();
        app.composer.set_text("hello");
        let action = app.handle_key(key(KeyCode::Enter));
        match action {
            AppAction::SendMessage { params, .. } => {
                assert_eq!(params.get("model"), None);
                assert_eq!(
                    params.get("mode"),
                    Some(&Value::String("agent".to_string()))
                );
            }
            other => panic!("unexpected action: {other:?}"),
        }

        app.pending_model = Some("model-b".to_string());
        app.pending_mode = Some("mode-b".to_string());
        app.pending_send_retry = Some(PendingSendRetry {
            prompt: "retry".to_string(),
            params: json!({"model": "old"}),
            correlation: ClientMessageCorrelation {
                client_request_id: "old-request".to_string(),
                client_message_id: "old-message".to_string(),
            },
        });
        app.set_project(OpenProjectResponse {
            project_id: "p2".to_string(),
            slug: "demo2".to_string(),
            root: PathBuf::from("/tmp/demo2"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        });
        app.composer.set_text("next");
        let action = app.handle_key(key(KeyCode::Enter));
        match action {
            AppAction::SendMessage { params, .. } => {
                assert_eq!(params.get("model"), None);
                assert_eq!(
                    params.get("tool_use"),
                    Some(&Value::String("agent".to_string()))
                );
            }
            other => panic!("unexpected action: {other:?}"),
        }
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
        let mut daemon_events = DaemonEventSubscription::new();
        run_action(
            &mut app,
            action,
            &client,
            &tx,
            &mut subscriptions,
            &mut daemon_events,
        )
        .await;
        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::CommandFinished {
                generation: 0,
                context: CommandContextTag::Other,
                result: Ok(())
            })
        ));

        let command = state.find_command("set_params").unwrap();
        assert_eq!(command["patch"]["mode"], "agent");
        assert_eq!(command["patch"]["tool_use"], "agent");
    }

    #[tokio::test]
    async fn send_message_action_posts_its_client_request_id() {
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        app.composer.set_text("hello");
        let action = app.handle_key(key(KeyCode::Enter));
        let correlation = match &action {
            AppAction::SendMessage { correlation, .. } => correlation.clone(),
            other => panic!("unexpected action: {other:?}"),
        };
        let (tx, mut rx) = mpsc::channel(1);
        let mut subscriptions = SubscriptionManager::new();
        let mut daemon_events = DaemonEventSubscription::new();

        run_action(
            &mut app,
            action,
            &client,
            &tx,
            &mut subscriptions,
            &mut daemon_events,
        )
        .await;

        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::CommandFinished {
                context: CommandContextTag::SendMessage { correlation: sent, .. },
                result: Ok(()),
                ..
            }) if sent == correlation
        ));
        let command = state.find_command("user_message").unwrap();
        assert_eq!(command["client_request_id"], correlation.client_request_id);
        assert_eq!(command["client_message_id"], correlation.client_message_id);
    }

    #[tokio::test]
    async fn reasoning_command_posts_set_params() {
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        app.apply_caps(&json!({
            "chat_models": {
                "gpt-demo": {"reasoning_effort_options": ["low", "medium", "high"]}
            }
        }));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"thread": {"model": "gpt-demo", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        });
        let action = app.execute_command_name("reasoning high");
        let (tx, mut rx) = mpsc::channel(1);
        let mut subscriptions = SubscriptionManager::new();
        let mut daemon_events = DaemonEventSubscription::new();
        run_action(
            &mut app,
            action,
            &client,
            &tx,
            &mut subscriptions,
            &mut daemon_events,
        )
        .await;
        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::CommandFinished {
                generation: 0,
                context: CommandContextTag::Reasoning { .. },
                result: Ok(())
            })
        ));

        let command = state.find_command("set_params").unwrap();
        assert_eq!(command["patch"]["boost_reasoning"], true);
        assert_eq!(command["patch"]["reasoning_effort"], "high");
        assert!(command["patch"]["thinking_budget"].is_null());
    }

    #[test]
    fn browser_events_drive_browser_state_and_reconnect_snapshot() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({
                "type": "snapshot",
                "thread": {},
                "runtime": {"state": "idle"},
                "messages": [],
                "browser": {
                    "runtime_id": "browser-1",
                    "connected": true,
                    "active_tab": "tab-1",
                    "url": "https://example.test",
                    "title": "Example",
                    "tabs": [{"tab_id": "tab-1", "url": "https://example.test", "title": "Example"}]
                }
            }),
        });
        assert!(app.browser_state().is_open);
        assert_eq!(
            app.browser_state().current_url.as_deref(),
            Some("https://example.test")
        );
        assert_eq!(app.browser_state().tabs.len(), 1);

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_frame".to_string(),
            raw: json!({
                "type": "browser_frame",
                "tab_id": "tab-1",
                "mime": "image/jpeg",
                "data": "frame-data",
                "changed_text": "Saved"
            }),
        });
        assert_eq!(
            app.browser_state()
                .latest_frame
                .as_ref()
                .map(|frame| frame.changed_text.as_deref()),
            Some(Some("Saved"))
        );

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_timeline".to_string(),
            raw: json!({
                "type": "browser_timeline",
                "events": [{"type": "click", "summary": "Saved"}]
            }),
        });
        assert_eq!(app.browser_state().timeline.len(), 1);

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_toolbar_action".to_string(),
            raw: json!({"type": "browser_toolbar_action", "action": "screenshot"}),
        });
        assert_eq!(
            app.browser_state().last_toolbar_action.as_deref(),
            Some("screenshot")
        );

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_status".to_string(),
            raw: json!({
                "type": "browser_status",
                "runtime_id": "browser-1",
                "connected": true,
                "active_tab": "tab-2",
                "url": "https://second.example.test",
                "title": "Second",
                "tabs": [{"tab_id": "tab-2", "url": "https://second.example.test", "title": "Second"}]
            }),
        });
        assert_eq!(app.browser_state().active_tab.as_deref(), Some("tab-2"));
        assert_eq!(app.browser_state().current_title.as_deref(), Some("Second"));

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_context_oversize".to_string(),
            raw: json!({
                "type": "browser_context_oversize",
                "total_bytes": 1000,
                "action_count": 2,
                "action_bytes": 200,
                "console_count": 3,
                "console_bytes": 300,
                "network_count": 4,
                "network_bytes": 400,
                "mutation_bytes": 100,
                "pending_message_id": "pending-1"
            }),
        });
        assert_eq!(
            app.browser_state()
                .context_prompt
                .as_ref()
                .map(|prompt| prompt.event.pending_message_id.as_str()),
            Some("pending-1")
        );

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "browser_closed".to_string(),
            raw: json!({
                "type": "browser_closed",
                "runtime_id": "browser-1",
                "reason": "user_closed"
            }),
        });
        assert!(!app.browser_state().is_open);
        assert!(app.browser_state().current_url.is_none());
        assert!(app.browser_state().context_prompt.is_none());
        assert_eq!(app.browser_state().timeline.len(), 1);
        assert_eq!(
            app.browser_state()
                .last_closed
                .as_ref()
                .map(|closed| closed.reason.as_str()),
            Some("user_closed")
        );
    }

    #[test]
    fn browser_snapshot_replaces_stale_event_state_on_reconnect() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_frame".to_string(),
            raw: json!({
                "type": "browser_frame",
                "tab_id": "tab-1",
                "mime": "image/jpeg",
                "data": "frame-data"
            }),
        });
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_timeline".to_string(),
            raw: json!({"type": "browser_timeline", "events": [{"type": "click"}]}),
        });
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id.clone()),
            seq: None,
            kind: "browser_context_oversize".to_string(),
            raw: json!({
                "type": "browser_context_oversize",
                "pending_message_id": "pending-1"
            }),
        });

        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({
                "type": "snapshot",
                "thread": {},
                "runtime": {"state": "idle"},
                "messages": [],
                "browser": {
                    "runtime_id": "browser-2",
                    "connected": true,
                    "active_tab": "tab-2",
                    "url": "https://reconnected.example.test",
                    "title": "Reconnected",
                    "tabs": []
                }
            }),
        });

        assert_eq!(app.browser_state().runtime_id.as_deref(), Some("browser-2"));
        assert!(app.browser_state().latest_frame.is_none());
        assert!(app.browser_state().timeline.is_empty());
        assert!(app.browser_state().context_prompt.is_none());
    }

    #[tokio::test]
    async fn browser_context_prompt_sends_decision_and_restores_after_failure() {
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.apply_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "browser_context_oversize".to_string(),
            raw: json!({
                "type": "browser_context_oversize",
                "total_bytes": 1000,
                "pending_message_id": "pending-1"
            }),
        });

        let action = app.submit_browser_context_decision(BrowserContextDecisionOptions {
            include_actions: true,
            include_console: false,
            include_network: true,
            include_mutations: false,
            include_screenshot: true,
            last_n_actions: Some(2),
            last_n_console: None,
            last_n_network: Some(3),
        });
        assert!(app.browser_state().context_prompt.is_none());
        let (tx, mut rx) = mpsc::channel(1);
        let mut subscriptions = SubscriptionManager::new();
        let mut daemon_events = DaemonEventSubscription::new();
        run_action(
            &mut app,
            action,
            &client,
            &tx,
            &mut subscriptions,
            &mut daemon_events,
        )
        .await;

        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::CommandFinished {
                context: CommandContextTag::BrowserContextDecision { .. },
                result: Ok(()),
                ..
            })
        ));
        let command = state.find_command("browser_context_decision").unwrap();
        assert_eq!(command["pending_message_id"], "pending-1");
        assert_eq!(command["include_actions"], true);
        assert_eq!(command["include_console"], false);
        assert_eq!(command["include_network"], true);
        assert_eq!(command["include_mutations"], false);
        assert_eq!(command["include_screenshot"], true);
        assert_eq!(command["last_n_actions"], 2);
        assert_eq!(command["last_n_network"], 3);

        let failed =
            app.submit_browser_context_decision(BrowserContextDecisionOptions::include_all());
        assert_eq!(failed, AppAction::None);
        app.apply_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "browser_context_oversize".to_string(),
            raw: json!({
                "type": "browser_context_oversize",
                "total_bytes": 1000,
                "pending_message_id": "pending-2"
            }),
        });
        let action =
            app.submit_browser_context_decision(BrowserContextDecisionOptions::include_all());
        let AppAction::SendBrowserContextDecision { prompt, .. } = action else {
            panic!("expected browser context action");
        };
        let origin = app.command_origin();
        app.handle_command_finished(
            CommandContextTag::BrowserContextDecision { origin, prompt },
            Err("offline".to_string()),
        );
        assert_eq!(
            app.browser_state()
                .context_prompt
                .as_ref()
                .map(|prompt| prompt.event.pending_message_id.as_str()),
            Some("pending-2")
        );
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
    fn esc_esc_backtrack_loads_user_message_and_retries_from_index() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "first"},
                {"message_id": "a1", "role": "assistant", "content": "one"},
                {"message_id": "u2", "role": "user", "content": "second"},
                {"message_id": "a2", "role": "assistant", "content": "two"}
            ]}),
        });

        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);
        assert_eq!(app.selected_backtrack_index(), None);
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);
        assert_eq!(app.selected_backtrack_index(), Some(2));
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);
        assert_eq!(app.selected_backtrack_index(), Some(0));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert_eq!(app.composer(), "first");
        app.composer.set_text("edited first");

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(matches!(
            action,
            AppAction::RetryFromIndex { index: 0, content }
                if content == json!("edited first")
        ));
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(matches!(
            app.transcript_state().messages().get(0),
            Some(message) if message.role == TranscriptRole::User && message.content == "edited first"
        ));
        assert_eq!(app.transcript_state().messages().len(), 2);
    }

    #[test]
    fn retry_from_index_failure_restores_transcript_and_leaves_generating() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "first"},
                {"message_id": "a1", "role": "assistant", "content": "one"},
                {"message_id": "u2", "role": "user", "content": "second"},
                {"message_id": "a2", "role": "assistant", "content": "two"}
            ]}),
        });
        let before = app.transcript_state().messages().to_vec();

        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Enter));
        app.composer.set_text("edited first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::RetryFromIndex { .. }
        ));
        assert_eq!(app.session_state(), SessionState::Generating);

        app.handle_command_finished(
            CommandContextTag::RetryFromIndex {
                rollback: app.pending_backtrack_rollback.clone(),
            },
            Err("retry rejected".to_string()),
        );

        assert_eq!(
            &app.transcript_state().messages()[..before.len()],
            before.as_slice()
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert_eq!(app.composer(), "edited first");
    }

    #[test]
    fn ctrl_t_overlay_opens_searches_and_enters_copy_mode() {
        let mut app = App::new(project());
        let chat_id = app.chat_id().to_string();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(chat_id),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "idle"}, "messages": [
                {"message_id": "u1", "role": "user", "content": "find the thing"},
                {"message_id": "a1", "role": "assistant", "content": "the thing is here"}
            ]}),
        });

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(app.transcript_overlay().is_some());
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.transcript_overlay().unwrap().query(), "th");
        assert!(app.transcript_overlay().unwrap().match_count() > 0);
        app.handle_key(key(KeyCode::Char('c')));
        assert!(app.transcript_overlay().unwrap().is_copy_mode());
        app.handle_key(key(KeyCode::Esc));
        assert!(app.transcript_overlay().is_none());
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows artifact runners crash when this test spawns cmd"
    )]
    fn editor_round_trip_appends_with_fake_editor_script() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let editor = "cmd /C echo extra>>".to_string();
        #[cfg(not(windows))]
        let editor = {
            let path = dir.path().join("fake-editor.sh");
            std::fs::write(&path, "#!/bin/sh\necho extra >> \"$1\"\n").unwrap();
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                perms.set_mode(0o755);
                std::fs::set_permissions(&path, perms).unwrap();
            }
            path.to_string_lossy().to_string()
        };

        let text = edit_text_with_editor_command(&editor, "base\n".to_string()).unwrap();

        assert_eq!(text.replace("\r\n", "\n"), "base\nextra\n");
    }

    #[test]
    fn editor_command_parser_handles_flags_quotes_and_empty_command() {
        let parsed = parse_editor_command("nano --wait 'two words'").unwrap();
        assert_eq!(parsed.program, "nano");
        assert_eq!(parsed.args, vec!["--wait", "two words"]);
        assert!(parse_editor_command("   ").is_err());
    }

    #[test]
    fn editor_command_selection_prefers_editor_then_visual_then_vi() {
        assert_eq!(
            external_editor_command_from_values(Some(" nvim --wait "), Some("code"), |_| false)
                .unwrap(),
            "nvim --wait"
        );
        assert_eq!(
            external_editor_command_from_values(Some(" "), Some("code --reuse-window"), |_| false)
                .unwrap(),
            "code --reuse-window"
        );
        assert_eq!(
            external_editor_command_from_values(None, None, |command| command == "vi").unwrap(),
            "vi"
        );
        assert!(external_editor_command_from_values(None, None, |_| false).is_err());
    }

    #[test]
    fn app_fast_typing_submits_on_enter() {
        let mut app = App::new(project());
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, AppAction::SendMessage { prompt, .. } if prompt == "ab"));
        assert!(app.composer().is_empty());
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
        let request = app.take_pending_history_save().unwrap();
        execute_history_save_request(request).unwrap();
        app.handle_history_save_result(Ok(()));
        let mut app = App::with_history_path(project(), Some(path));
        app.composer.set_text("draft");
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.composer(), "first");
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.composer(), "draft");
    }

    #[test]
    fn history_save_failure_emits_one_notice() {
        let dir = tempfile::tempdir().unwrap();
        let parent_file = dir.path().join("not-a-dir");
        std::fs::write(&parent_file, "blocked").unwrap();
        let path = parent_file.join("history.json");
        let mut app = App::with_history_path(project(), Some(path));

        app.composer.set_text("first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { .. }
        ));
        let request = app.take_pending_history_save().unwrap();
        let result = execute_history_save_request(request);
        app.handle_history_save_result(result);

        app.set_session_state(SessionState::Idle);
        app.composer.set_text("second");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { .. }
        ));
        let request = app.take_pending_history_save().unwrap();
        let result = execute_history_save_request(request);
        app.handle_history_save_result(result);

        let notices = app
            .visible_transcript()
            .iter()
            .filter(|item| {
                matches!(item, TranscriptItem::Notice(text) if text.contains("Failed to save composer history"))
            })
            .count();
        assert_eq!(notices, 1);
    }

    #[test]
    fn app_ctrl_r_reverse_history_search_accepts_and_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut app = App::with_history_path(project(), Some(path.clone()));
        app.composer.set_text("alpha one");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { .. }
        ));
        let request = app.take_pending_history_save().unwrap();
        execute_history_save_request(request).unwrap();
        app.handle_history_save_result(Ok(()));
        let mut app = App::with_history_path(project(), Some(path.clone()));
        app.composer.set_text("beta two");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { .. }
        ));
        let request = app.take_pending_history_save().unwrap();
        execute_history_save_request(request).unwrap();
        app.handle_history_save_result(Ok(()));

        let mut app = App::with_history_path(project(), Some(path));
        app.composer.set_text("draft");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(app.composer(), "beta two");
        for ch in "alpha".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()));
        }
        assert_eq!(app.composer(), "alpha one");
        assert_eq!(app.composer_history_search().unwrap().query, "alpha");
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.composer(), "draft");

        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.composer(), "beta two");
        assert!(app.composer_history_search().is_none());
    }

    #[test]
    fn app_kill_yank_undo_redo_bindings_edit_composer() {
        let mut app = App::new(project());
        app.composer.set_text("alpha beta\ngamma");
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.composer(), "alpha beta\n");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(app.composer(), "alpha beta\ngamma");
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(app.composer(), "alpha beta\n");
        app.handle_key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(app.composer(), "alpha beta\ngamma");
    }

    #[test]
    fn app_cancel_and_double_ctrl_c_behaviour() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            AppAction::Abort
        );
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(app.abort_in_flight);
        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: app.command_origin(),
                },
                Ok(()),
            ),
            AppAction::None
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
    fn normal_key_between_idle_ctrl_c_presses_resets_quit_arming() {
        let mut app = App::new(project());

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(app.last_ctrl_c.is_some());
        assert!(!app.should_quit());

        assert_eq!(app.handle_key(key(KeyCode::Char('x'))), AppAction::None);
        assert_eq!(app.composer(), "x");
        assert!(app.last_ctrl_c.is_none());

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(!app.should_quit());
        assert!(app.last_ctrl_c.is_some());
    }

    #[test]
    fn ctrl_c_during_generation_aborts_without_arming_quit_countdown() {
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert_eq!(app.session_state(), SessionState::Generating);
        assert!(app.abort_in_flight);
        assert!(!app.should_quit());
        assert!(app.last_ctrl_c.is_none());

        assert_eq!(
            app.handle_command_finished(
                CommandContextTag::Abort {
                    origin: app.command_origin(),
                },
                Ok(()),
            ),
            AppAction::None
        );
        assert_eq!(app.session_state(), SessionState::Idle);

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(!app.should_quit());
    }

    #[test]
    fn ctrl_c_after_completed_turn_still_aborts_active_generation() {
        let mut app = App::new(project());
        app.composer.set_text("first");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { prompt, .. } if prompt == "first"
        ));
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.composer.set_text("second");
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::SendMessage { prompt, .. } if prompt == "second"
        ));

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert!(app.abort_in_flight);
        assert!(!app.should_quit());
    }

    #[test]
    fn ctrl_c_is_global_for_overlay_picker_approval_and_ask_form() {
        let mut app = App::new(project());
        app.open_transcript_overlay();
        app.set_session_state(SessionState::Generating);
        assert!(app.transcript_overlay().is_some());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert!(app.transcript_overlay().is_none());

        let mut app = App::new(project());
        app.open_model_picker(json!({"chat_models": {"m1": {"name": "Model One"}}}));
        assert!(app.modal_picker().is_some());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(app.modal_picker().is_none());
        assert!(!app.should_quit());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::None
        );
        assert!(app.should_quit());

        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));
        assert!(app.approval_modal().is_some());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert!(app.approval_modal().is_some());

        let mut app = App::new(project());
        app.handle_chat_event(ask_questions_tool_event(
            &app,
            "call-ask",
            json!([
                {"id": "confirm", "type": "yes_no", "text": "Proceed?"}
            ]),
        ));
        app.handle_chat_event(waiting_user_input_event(&app));
        assert!(app.ask_questions_form().is_some());
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            AppAction::Abort
        );
        assert!(app.ask_questions_form().is_some());
    }

    #[tokio::test]
    async fn active_generation_quit_sends_abort_command() {
        let state = CommandState::default();
        let base_url = spawn_command_server(state.clone());
        let client = DaemonClient::new(base_url, None).unwrap();
        let mut app = App::new(project());
        app.set_session_state(SessionState::Generating);
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert_eq!(action, AppAction::Quit { abort_active: true });
        assert!(!app.should_quit());
        assert_eq!(app.session_state(), SessionState::Generating);

        let (tx, _rx) = mpsc::channel(1);
        let mut subscriptions = SubscriptionManager::new();
        let mut daemon_events = DaemonEventSubscription::new();
        run_action(
            &mut app,
            action,
            &client,
            &tx,
            &mut subscriptions,
            &mut daemon_events,
        )
        .await;

        assert!(state.wait_for("abort"));
        assert!(app.should_quit());
        assert_eq!(app.session_state(), SessionState::Idle);
    }

    #[test]
    fn project_picker_filters_and_selects() {
        let mut picker = surfaces::ProjectPickerState::new(vec![
            ProjectEntry {
                id: "a".to_string(),
                slug: "alpha".to_string(),
                root: PathBuf::from("/tmp/alpha"),
                pinned: Some(false),
                last_active_ms: Some(0),
                settings: Value::Null,
            },
            ProjectEntry {
                id: "b".to_string(),
                slug: "beta".to_string(),
                root: PathBuf::from("/tmp/beta"),
                pinned: Some(false),
                last_active_ms: Some(0),
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
        assert_tool_decision_action(
            &action,
            vec![ToolDecision {
                tool_call_id: "call-1".to_string(),
                accepted: true,
            }],
            None,
        );
    }

    #[test]
    fn denied_approval_marks_matching_tool_denied() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{
                "id": "call-denied",
                "function": {"name": "shell", "arguments": "{}"}
            }]}]}),
        });
        app.handle_chat_event(pause_event(&app, "call-denied", "shell"));
        assert_eq!(tool_cards(&app)[0].status, ToolStatus::AwaitingApproval);

        assert!(matches!(
            app.handle_key(key(KeyCode::Char('n'))),
            AppAction::SendToolDecisions { .. }
        ));

        let card = tool_cards(&app)[0];
        assert_eq!(card.status, ToolStatus::Denied);
        assert!(rendered_item_plain_text(&TranscriptItem::Tool(card.clone())).contains("⊘ denied"));
    }

    #[test]
    fn cleared_approval_marks_matching_tool_cancelled() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{
                "id": "call-cancelled",
                "function": {"name": "shell", "arguments": "{}"}
            }]}]}),
        });
        app.handle_chat_event(pause_event(&app, "call-cancelled", "shell"));

        app.clear_approvals();

        assert_eq!(tool_cards(&app)[0].status, ToolStatus::Cancelled);
    }

    #[test]
    fn tool_decision_failure_restores_approval_queue() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));
        let action = app.handle_key(key(KeyCode::Char('y')));
        let AppAction::SendToolDecisions { rollback, .. } = action else {
            panic!("expected tool decision action");
        };
        assert!(app.approval_modal().is_none());

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: "decision-1".to_string(),
                rollback,
            },
            Err("decision rejected".to_string()),
        );

        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-1"]
        ));
        assert_eq!(app.approval_pending_clear_count(), 0);
    }

    #[test]
    fn native_scrollback_tool_decision_failure_restores_pending_history() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.handle_chat_event(tool_call_delta_event(&app, "call-1"));
        app.complete_tool("call-1", "tool", "done".to_string(), ToolStatus::Succeeded);
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));

        let AppAction::SendToolDecisions { rollback, .. } = app.handle_key(key(KeyCode::Char('y')))
        else {
            panic!("expected tool decision action");
        };
        assert_eq!(
            app.history_tool_status_for_test("call-1"),
            Some(ToolStatus::ApprovedOnce)
        );

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: "decision-1".to_string(),
                rollback,
            },
            Err("decision rejected".to_string()),
        );

        assert_eq!(
            app.history_tool_status_for_test("call-1"),
            Some(ToolStatus::AwaitingApproval)
        );
        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-1"]
        ));
        assert!(!app.resize_reflow.has_pending_reflow());
    }

    #[test]
    fn native_scrollback_tool_decision_failure_reflows_drained_history() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.handle_chat_event(tool_call_delta_event(&app, "call-1"));
        app.complete_tool("call-1", "tool", "done".to_string(), ToolStatus::Succeeded);
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));

        let AppAction::SendToolDecisions { rollback, .. } = app.handle_key(key(KeyCode::Char('y')))
        else {
            panic!("expected tool decision action");
        };
        app.pending_history_insertions(80);
        assert_eq!(app.history_pending_count(), 0);
        app.resize_reflow.clear_pending_reflow();

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: "decision-1".to_string(),
                rollback,
            },
            Err("decision rejected".to_string()),
        );

        assert_eq!(
            app.history_tool_status_for_test("call-1"),
            Some(ToolStatus::AwaitingApproval)
        );
        assert!(app.resize_reflow.has_pending_reflow());
    }

    #[test]
    fn native_scrollback_out_of_order_failure_restores_only_its_scope() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        app.pending_history_insertions(80);
        for tool_call_id in ["call-a", "call-b"] {
            app.handle_chat_event(tool_call_delta_event(&app, tool_call_id));
            app.complete_tool(
                tool_call_id,
                "tool",
                "done".to_string(),
                ToolStatus::Succeeded,
            );
        }
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_finished".to_string(),
            raw: json!({}),
        });
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));
        app.handle_chat_event(pause_event(&app, "call-b", "cat"));

        let AppAction::SendToolDecisions {
            client_request_id: first_id,
            rollback: first_rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('y')))
        else {
            panic!("expected first tool decision action");
        };
        let AppAction::SendToolDecisions {
            client_request_id: second_id,
            rollback: second_rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('n')))
        else {
            panic!("expected second tool decision action");
        };

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: second_id,
                rollback: second_rollback,
            },
            Ok(()),
        );
        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: first_id,
                rollback: first_rollback,
            },
            Err("first decision rejected".to_string()),
        );

        assert_eq!(
            app.history_tool_status_for_test("call-a"),
            Some(ToolStatus::AwaitingApproval)
        );
        assert_eq!(
            app.history_tool_status_for_test("call-b"),
            Some(ToolStatus::Denied)
        );
        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-a"]
        ));
    }

    #[test]
    fn tool_decision_failure_restores_only_its_own_approval_after_newer_success() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));
        app.handle_chat_event(pause_event(&app, "call-b", "cat"));

        let AppAction::SendToolDecisions {
            client_request_id: first_id,
            rollback: first_rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('y')))
        else {
            panic!("expected first tool decision action");
        };
        let AppAction::SendToolDecisions {
            client_request_id: second_id,
            rollback: second_rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('n')))
        else {
            panic!("expected second tool decision action");
        };

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: second_id,
                rollback: second_rollback,
            },
            Ok(()),
        );
        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: first_id,
                rollback: first_rollback,
            },
            Err("first decision rejected".to_string()),
        );

        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-a"]
        ));
        assert_eq!(app.approval_pending_clear_count(), 1);
    }

    #[test]
    fn newer_tool_decision_failure_preserves_older_success() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));
        app.handle_chat_event(pause_event(&app, "call-b", "cat"));

        let AppAction::SendToolDecisions {
            client_request_id: first_id,
            rollback: first_rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('y')))
        else {
            panic!("expected first tool decision action");
        };
        let AppAction::SendToolDecisions {
            client_request_id: second_id,
            rollback: second_rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('n')))
        else {
            panic!("expected second tool decision action");
        };

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: second_id,
                rollback: second_rollback,
            },
            Err("second decision rejected".to_string()),
        );
        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id: first_id,
                rollback: first_rollback,
            },
            Ok(()),
        );

        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-b"]
        ));
        assert_eq!(app.approval_pending_clear_count(), 1);
    }

    #[test]
    fn stale_tool_decision_failure_preserves_newer_approval() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));
        let AppAction::SendToolDecisions {
            client_request_id,
            rollback,
            ..
        } = app.handle_key(key(KeyCode::Char('y')))
        else {
            panic!("expected tool decision action");
        };
        app.handle_chat_event(pause_event(&app, "call-b", "cat"));

        app.handle_command_finished(
            CommandContextTag::ToolDecisions {
                client_request_id,
                rollback,
            },
            Err("first decision rejected".to_string()),
        );

        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-a"] && modal.pending_after() == 1
        ));
        let action = app.handle_key(key(KeyCode::Char('n')));
        assert!(matches!(
            action,
            AppAction::SendToolDecisions { decisions, .. }
                if decisions == vec![ToolDecision {
                    tool_call_id: "call-a".to_string(),
                    accepted: false,
                }]
        ));
        assert!(matches!(
            app.approval_modal(),
            Some(modal) if modal.tool_call_ids() == ["call-b"]
        ));
    }

    #[test]
    fn multi_tool_pause_yields_per_tool_decisions() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "pause_required".to_string(),
            raw: json!({"reasons": [
                {"type": "confirmation", "tool_name": "shell", "command": "echo one", "rule": "*", "tool_call_id": "call-1"},
                {"type": "confirmation", "tool_name": "cat", "command": "cat Cargo.toml", "rule": "*", "tool_call_id": "call-2"}
            ]}),
        });

        assert_tool_decision_action(
            &app.handle_key(key(KeyCode::Char('n'))),
            vec![
                ToolDecision {
                    tool_call_id: "call-1".to_string(),
                    accepted: false,
                },
                ToolDecision {
                    tool_call_id: "call-2".to_string(),
                    accepted: false,
                },
            ],
            None,
        );
    }

    #[test]
    fn approval_detail_toggle_uses_escape_as_back_not_deny() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));

        assert!(!app.approval_modal().unwrap().details_open());
        assert_eq!(app.handle_key(key(KeyCode::Char('v'))), AppAction::None);
        assert!(app.approval_modal().unwrap().details_open());
        assert_eq!(app.handle_key(key(KeyCode::Down)), AppAction::None);
        assert_eq!(app.approval_modal().unwrap().detail_scroll(), 1);
        assert_eq!(app.handle_key(key(KeyCode::Esc)), AppAction::None);
        let modal = app.approval_modal().unwrap();
        assert!(!modal.details_open());
        assert_eq!(modal.detail_scroll(), 0);
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
        assert_tool_decision_action(
            &first_action,
            vec![ToolDecision {
                tool_call_id: "call-1".to_string(),
                accepted: true,
            }],
            None,
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
        assert_tool_decision_action(
            &second_action,
            vec![ToolDecision {
                tool_call_id: "call-2".to_string(),
                accepted: false,
            }],
            None,
        );
        assert!(app.approval_modal().is_none());
    }

    #[test]
    fn decided_approval_snapshot_replay_skips_decided_scope_and_keeps_pending() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));
        app.handle_chat_event(pause_event(&app, "call-b", "cat"));

        let first_action = app.handle_key(key(KeyCode::Char('y')));
        assert_tool_decision_action(
            &first_action,
            vec![ToolDecision {
                tool_call_id: "call-a".to_string(),
                accepted: true,
            }],
            None,
        );
        assert_eq!(app.approval_pending_clear_count(), 1);
        assert_eq!(
            app.approval_modal().unwrap().reasons()[0].tool_call_id,
            "call-b"
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "paused", "pause_reasons": [pause_reason_value("call-a", "shell")]}, "messages": []}),
        });
        assert_eq!(app.approval_pending_clear_count(), 1);
        assert_eq!(
            app.approval_modal().unwrap().reasons()[0].tool_call_id,
            "call-b"
        );

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "pause_cleared".to_string(),
            raw: json!({}),
        });
        assert_eq!(app.approval_pending_clear_count(), 0);
        assert_eq!(
            app.approval_modal().unwrap().reasons()[0].tool_call_id,
            "call-b"
        );

        let second_action = app.handle_key(key(KeyCode::Char('n')));
        assert_tool_decision_action(
            &second_action,
            vec![ToolDecision {
                tool_call_id: "call-b".to_string(),
                accepted: false,
            }],
            None,
        );
        assert!(app.approval_modal().is_none());
    }

    #[test]
    fn snapshot_brand_new_pause_preserves_existing_pending_order() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: json!({"runtime": {"state": "paused", "pause_reasons": [pause_reason_value("call-c", "cat")]}, "messages": []}),
        });

        let first = app.approval_modal().unwrap();
        assert_eq!(first.reasons()[0].tool_call_id, "call-a");
        assert_eq!(first.pending_after(), 1);

        let first_action = app.handle_key(key(KeyCode::Char('y')));
        assert_tool_decision_action(
            &first_action,
            vec![ToolDecision {
                tool_call_id: "call-a".to_string(),
                accepted: true,
            }],
            None,
        );
        assert_eq!(
            app.approval_modal().unwrap().reasons()[0].tool_call_id,
            "call-c"
        );
    }

    #[test]
    fn pause_cleared_with_empty_approval_queue_is_noop() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "pause_cleared".to_string(),
            raw: json!({}),
        });

        assert!(app.approval_modal().is_none());
        assert_eq!(app.approval_pending_clear_count(), 0);
    }

    #[test]
    fn pause_cleared_with_tool_call_id_clears_matching_scope_only() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-a", "shell"));
        app.handle_chat_event(pause_event(&app, "call-b", "cat"));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "pause_cleared".to_string(),
            raw: json!({"tool_call_id": "call-b"}),
        });

        assert_eq!(
            app.approval_modal().unwrap().reasons()[0].tool_call_id,
            "call-a"
        );
        assert_eq!(app.approval_modal().unwrap().pending_after(), 0);
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
    fn unfinished_plan_message_streams_then_finalizes_to_plan_cell() {
        let mut app = App::new(project());

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "p1",
                "role": "plan",
                "content": "## Plan\n",
                "stream_finished": false,
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_updated".to_string(),
            raw: json!({"message_id": "p1", "message": {
                "message_id": "p1",
                "role": "plan",
                "content": "## Plan\n- one\n",
                "stream_finished": false,
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }}),
        });

        let streamed = plan_stream_text(&app);
        assert!(streamed.contains("Proposed Plan"));
        assert!(streamed.contains("## Plan"));
        assert!(streamed.contains("- one"));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_updated".to_string(),
            raw: json!({"message_id": "p1", "message": {
                "message_id": "p1",
                "role": "plan",
                "content": "## Plan\n- one\n",
                "stream_finished": true,
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }}),
        });

        assert!(plan_stream_text(&app).is_empty());
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Plan(data) if data.content.contains("- one"))
        }));
    }

    #[test]
    fn native_plan_stream_commits_non_final_plan_cells() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {
                "message_id": "p1",
                "role": "plan",
                "content": "- one\n- two\n- three\n- four\n- five\n- six\n- seven\n- eight\n",
                "stream_finished": false,
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }}),
        });

        app.apply_stream_commit_tick();
        assert!(app.history_pending_count() > 0);
        let pending_text = app
            .pending_history_insertions(80)
            .into_iter()
            .flat_map(|insertion| insertion.lines)
            .map(|line| line_to_plain_string(&line.line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(pending_text.contains("Proposed Plan"));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_updated".to_string(),
            raw: json!({"message_id": "p1", "message": {
                "message_id": "p1",
                "role": "plan",
                "content": "- one\n- two\n- three\n- four\n- five\n- six\n- seven\n- eight\n",
                "stream_finished": true,
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }}),
        });

        assert!(plan_stream_text(&app).is_empty());
        assert!(app.resize_reflow_is_due());
        app.resize_reflow_insertions(80);
        assert_eq!(app.history_pending_count(), 0);
        assert!(app.visible_transcript().iter().any(|item| {
            matches!(item, TranscriptItem::Plan(data) if data.content.contains("- eight"))
        }));
    }

    #[test]
    fn authoritative_mutation_rebuilds_active_plan_stream_once_and_continues() {
        for native_scrollback in [false, true] {
            let mut app = App::new(project());
            app.set_native_scrollback(native_scrollback);
            if native_scrollback {
                app.pending_history_insertions(80);
            }
            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_added".to_string(),
                raw: json!({"message": {
                    "message_id": "p1",
                    "role": "plan",
                    "content": "## Plan\n- base\n",
                    "stream_finished": false,
                    "extra": {"plan": {"mode": "agent", "version": 1}},
                }}),
            });
            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_added".to_string(),
                raw: json!({"message": {
                    "message_id": "d1",
                    "role": "event",
                    "content": "- first delta\n",
                    "stream_finished": false,
                    "extra": {"event": {"subkind": "plan_delta", "payload": {"seq": 1}}},
                }}),
            });
            if native_scrollback {
                app.apply_stream_commit_tick();
                app.pending_history_insertions(80);
            }

            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_added".to_string(),
                raw: json!({"index": 0, "message": {
                    "message_id": "u1",
                    "role": "user",
                    "content": "inserted",
                }}),
            });

            let rebuilt = plan_stream_text(&app);
            for content in ["## Plan", "- base", "- first delta"] {
                assert_eq!(rebuilt.matches(content).count(), 1, "{content}: {rebuilt}");
            }
            assert_eq!(
                app.visible_transcript()
                    .iter()
                    .filter(|item| matches!(item, TranscriptItem::PlanStream(_)))
                    .count(),
                1
            );

            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_updated".to_string(),
                raw: json!({"message_id": "d1", "message": {
                    "message_id": "d1",
                    "role": "event",
                    "content": "- first delta\n- continued\n",
                    "stream_finished": false,
                    "extra": {"event": {"subkind": "plan_delta", "payload": {"seq": 1}}},
                }}),
            });
            let continued = plan_stream_text(&app);
            for content in ["- base", "- first delta", "- continued"] {
                assert_eq!(
                    continued.matches(content).count(),
                    1,
                    "{content}: {continued}"
                );
            }

            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_updated".to_string(),
                raw: json!({"message_id": "p1", "message": {
                    "message_id": "p1",
                    "role": "plan",
                    "content": "## Plan\n- base\n",
                    "stream_finished": true,
                    "extra": {"plan": {"mode": "agent", "version": 1}},
                }}),
            });
            app.handle_chat_event(ChatEvent {
                chat_id: Some(app.chat_id().to_string()),
                seq: None,
                kind: "message_updated".to_string(),
                raw: json!({"message_id": "d1", "message": {
                    "message_id": "d1",
                    "role": "event",
                    "content": "- first delta\n- continued\n",
                    "stream_finished": true,
                    "extra": {"event": {"subkind": "plan_delta", "payload": {"seq": 1}}},
                }}),
            });

            assert!(plan_stream_text(&app).is_empty());
            let plans = app
                .visible_transcript()
                .iter()
                .filter_map(|item| match item {
                    TranscriptItem::Plan(data) => Some(data),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(plans.len(), 1);
            for content in ["- base", "- first delta", "- continued"] {
                assert_eq!(
                    plans[0].content.matches(content).count(),
                    1,
                    "{content}: {}",
                    plans[0].content
                );
            }
        }
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
        app.complete_tool("call-1", "tool", "done".to_string(), ToolStatus::Succeeded);
        if let [card] = tool_cards(&app).as_slice() {
            assert_eq!(card.status, ToolStatus::Succeeded);
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
        assert_eq!(cards[0].status, ToolStatus::Succeeded);
        assert_eq!(cards[0].result, "done");
        assert!(cards[0].expanded);
        assert!(cards[0].args_preview.contains("echo 2"));
    }

    #[test]
    fn working_indicator_ticks_and_tracks_current_tool_detail() {
        let mut app = App::new(project());
        app.apply_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: json!({"state": "generating"}),
        });
        assert_eq!(app.working_tick(), 0);
        app.apply_stream_commit_tick();
        assert_eq!(app.working_tick(), 1);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "shell", "arguments": "{\"cmd\":\"echo 1\"}"}}]}]}),
        });
        assert!(app
            .working_detail()
            .is_some_and(|detail| detail.contains("shell") && detail.contains("echo 1")));

        app.apply_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: json!({"state": "idle"}),
        });
        assert_eq!(app.working_tick(), 0);
        assert_eq!(app.working_elapsed_ms(), 0);
        assert_eq!(app.working_detail(), None);
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
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
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
    fn model_text_escape_sanitization_reaches_live_transcript_and_render_cells() {
        let mut app = App::new(project());
        let injected = injected_model_text();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [
                {"op": "append_reasoning", "text": injected},
                {"op": "append_content", "text": injected}
            ]}),
        });

        let assistant = assistant_text(&app);
        let reasoning = reasoning_text(&app);
        assert_escape_inert(&assistant);
        assert_escape_inert(&reasoning);
        assert_model_text_survives(&assistant);
        assert_model_text_survives(&reasoning);
        for item in app.visible_transcript() {
            if matches!(
                item,
                TranscriptItem::Assistant(_) | TranscriptItem::Reasoning(_, _)
            ) {
                assert_rendered_item_escape_inert(item);
            }
        }
    }

    #[test]
    fn inbound_user_message_escape_text_stays_inert_in_rendered_cells() {
        let mut app = App::new(project());
        let injected = injected_model_text();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "message_added".to_string(),
            raw: json!({"message": {"role": "user", "message_id": "u1", "content": injected}}),
        });

        let user = app
            .transcript_state()
            .messages()
            .iter()
            .find(|message| message.role == TranscriptRole::User)
            .unwrap();
        assert_escape_inert(&user.content);
        assert_model_text_survives(&user.content);
        for item in app.visible_transcript() {
            if matches!(item, TranscriptItem::User(_)) {
                assert_rendered_item_escape_inert(item);
            }
        }
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
        assert_eq!(cards[0].status, ToolStatus::Succeeded);
        assert_eq!(cards[0].result, "done");
        assert!(cards[0].expanded);
    }

    #[test]
    fn subchat_update_attaches_bounded_progress_to_parent_tool() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "tool_subagent", "arguments": "{}"}}]}]}),
        });
        let attached = (0..20)
            .map(|idx| format!("src/file_{idx}.rs"))
            .collect::<Vec<_>>();
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "subchat_update".to_string(),
            raw: json!({
                "tool_call_id": "call-1",
                "subchat_id": "1/2: search({\"query\":\"needle\"})",
                "attached_files": attached,
                "depth": 9
            }),
        });

        let cards = tool_cards(&app);
        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].subchat_log,
            vec!["1/2: search({\"query\":\"needle\"})"]
        );
        assert_eq!(cards[0].attached_files.len(), MAX_SUBCHAT_ATTACHED_FILES);
        assert_eq!(cards[0].subchat_depth, MAX_SUBCHAT_DEPTH);
        assert!(cards[0].subchat_active);
        assert!(cards[0].subchat_truncated);
    }

    #[test]
    fn subchat_update_file_notice_does_not_replace_latest_progress_and_clear_resets() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "tool_subagent", "arguments": "{}"}}]}]}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "subchat_update".to_string(),
            raw: json!({"tool_call_id": "call-1", "subchat_id": "thinking", "attached_files": []}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "subchat_update".to_string(),
            raw: json!({"tool_call_id": "call-1", "subchat_id": "/tool:files", "attached_files": ["README.md"]}),
        });
        assert_eq!(tool_cards(&app)[0].subchat_log, vec!["thinking"]);
        assert_eq!(tool_cards(&app)[0].attached_files, vec!["README.md"]);

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "subchat_update".to_string(),
            raw: json!({"tool_call_id": "call-1", "subchat_id": "", "attached_files": []}),
        });
        assert!(tool_cards(&app)[0].subchat_log.is_empty());
        assert!(tool_cards(&app)[0].attached_files.is_empty());
        assert!(!tool_cards(&app)[0].subchat_active);
    }

    #[test]
    fn subagents_command_lists_live_activity_and_empty_state() {
        let mut app = App::new(project());
        app.execute_command_name("subagents");
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Info(lines)) if lines.iter().any(|line| line == "No active subagents")
        ));

        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "tool_subagent", "arguments": "{}"}}]}]}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "subchat_update".to_string(),
            raw: json!({"tool_call_id": "call-1", "subchat_id": "collecting context", "attached_files": ["src/lib.rs"]}),
        });
        app.execute_command_name("multi-agents");
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Info(lines))
                if lines.join("\n").contains("tool_subagent [call-1]")
                    && lines.join("\n").contains("collecting context")
        ));
    }

    #[test]
    fn subagents_command_hides_internal_tool_prefixes() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [{"id": "call-1", "function": {"name": "t_process_start", "arguments": "{}"}}]}]}),
        });
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "subchat_update".to_string(),
            raw: json!({"tool_call_id": "call-1", "subchat_id": "collecting context", "attached_files": []}),
        });
        app.execute_command_name("subagents");

        let rendered = match app.visible_transcript().last() {
            Some(TranscriptItem::Info(lines)) => lines.join("\n"),
            _ => panic!("expected subagents card"),
        };
        assert!(rendered.contains("process_start [call-1]"));
        assert!(!rendered.contains("t_process_start"));
    }

    #[test]
    fn approval_decision_appends_record_cell() {
        let mut app = App::new(project());
        app.handle_chat_event(pause_event(&app, "call-1", "shell"));
        let action = app.handle_key(key(KeyCode::Char('y')));
        assert!(matches!(action, AppAction::SendToolDecisions { .. }));
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Approval(_, Some(ToolStatus::ApprovedOnce)))
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
            args: None,
            diff: None,
        }]));
        let action = app.handle_key(key(KeyCode::Char('a')));
        assert_tool_decision_action(
            &action,
            vec![ToolDecision {
                tool_call_id: "call-1".to_string(),
                accepted: true,
            }],
            Some(json!({"auto_approve_editing_tools": true})),
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
    fn modal_picker_space_still_toggles_multi_select() {
        let mut app = App::new(project());

        app.test_open_permissions_picker();
        assert_eq!(app.modal_picker().unwrap().selected_count(), 0);
        assert_eq!(app.handle_key(key(KeyCode::Char(' '))), AppAction::None);

        let picker = app.modal_picker().unwrap();
        assert_eq!(picker.selected_count(), 1);
        assert!(picker.is_selected("editing_tools"));
    }

    #[test]
    fn tab_cycles_tool_selection_and_enter_toggles_selected_tool() {
        let mut app = App::new(project());
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "set_tool_calls", "tool_calls": [
                {"id": "call-1", "function": {"name": "shell", "arguments": "{}"}},
                {"id": "call-2", "function": {"name": "cat", "arguments": "{}"}}
            ]}]}),
        });

        assert_eq!(app.selected_tool_index(), Some(2));
        assert_eq!(app.handle_key(key(KeyCode::Tab)), AppAction::None);
        assert_eq!(app.selected_tool_index(), Some(1));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), AppAction::None);
        assert!(tool_cards(&app)[0].expanded);
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

        app.record_chat_disconnected(
            "request failed with status 503: worker starting",
            true,
            false,
        );
        assert_eq!(app.subscription_status(), SubscriptionStatus::Waking);
        assert_eq!(app.retry_hint(), Some("worker waking; retrying"));
        assert!(app.daemon_online());

        app.record_chat_disconnected("connection refused", true, false);
        assert_eq!(app.subscription_status(), SubscriptionStatus::Waking);
        assert!(app.daemon_online());

        app.record_chat_resubscribe("request failed with status 429: retry-after: 2s");
        assert_eq!(app.subscription_status(), SubscriptionStatus::Waking);
        assert_eq!(app.retry_hint(), Some("rate limited; retry after 2s"));
    }

    #[test]
    fn exhausted_reconnect_is_visible_and_retry_is_bound() {
        let mut app = App::new(project());

        app.record_subscription_exhausted();

        assert_eq!(app.subscription_status(), SubscriptionStatus::Offline);
        assert!(!app.daemon_online());
        assert_eq!(app.session_state(), SessionState::Error);
        assert_eq!(
            app.retry_hint(),
            Some("SSE reconnect retries exhausted; retry the subscription")
        );
        assert_eq!(
            app.handle_key(KeyEvent::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
            AppAction::SubscribeCurrent
        );
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
        assert_eq!(decision, ChatSeqDecision::Suppress);
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
            reconnect_attempts: 0,
            join,
        });
        let stale = chat_event(
            &app,
            1,
            "stream_delta",
            json!({"ops": [{"op": "append_content", "text": "stale"}]}),
        );
        assert!(subscriptions.apply_chat_event(&mut app, 1, stale).is_none());
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
        assert!(subscriptions.apply_chat_event(&mut app, 2, fresh).is_some());
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
            reconnect_attempts: 0,
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
