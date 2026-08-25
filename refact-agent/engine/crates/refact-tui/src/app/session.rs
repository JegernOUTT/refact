use crate::commands::session as command_session;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Generating,
    ExecutingTools,
    Paused,
    WaitingIde,
    WaitingUserInput,
    Completed,
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
            SessionState::WaitingIde => "waiting for IDE",
            SessionState::WaitingUserInput => "waiting input",
            SessionState::Completed => "completed",
            SessionState::Error => "error",
        }
    }

    pub fn shows_working_indicator(self) -> bool {
        matches!(
            self,
            SessionState::Generating | SessionState::ExecutingTools
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSummary {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ReasoningModelCaps {
    effort_options: Option<Vec<String>>,
    supports_thinking_budget: Option<bool>,
    supports_adaptive_thinking_budget: Option<bool>,
}

impl ReasoningModelCaps {
    fn has_reasoning_support(&self) -> bool {
        self.effort_options
            .as_ref()
            .is_some_and(|options| !options.is_empty())
            || self.supports_thinking_budget == Some(true)
            || self.supports_adaptive_thinking_budget == Some(true)
    }

    fn reasoning_support_is_confirmed_absent(&self) -> bool {
        self.effort_options.as_ref().is_some_and(Vec::is_empty)
            && self.supports_thinking_budget == Some(false)
            && self.supports_adaptive_thinking_budget == Some(false)
    }

    fn supports_effort(&self, level: command_session::ReasoningLevel) -> bool {
        self.effort_options
            .iter()
            .flatten()
            .any(|option| option == level.as_str())
    }

    fn merge_from(&mut self, higher_precedence: &Self) {
        if higher_precedence.effort_options.is_some() {
            self.effort_options = higher_precedence.effort_options.clone();
        }
        if higher_precedence.supports_thinking_budget.is_some() {
            self.supports_thinking_budget = higher_precedence.supports_thinking_budget;
        }
        if higher_precedence
            .supports_adaptive_thinking_budget
            .is_some()
        {
            self.supports_adaptive_thinking_budget =
                higher_precedence.supports_adaptive_thinking_budget;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardCopySource {
    LastAssistant,
    OverlayVisible,
}

impl UsageSummary {
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        let prompt_tokens = token_count(value, &["prompt_tokens", "input_tokens", "prompt"]);
        let completion_tokens =
            token_count(value, &["completion_tokens", "output_tokens", "completion"]);
        let total_tokens = token_count(value, &["total_tokens", "total"]).or_else(|| {
            prompt_tokens
                .zip(completion_tokens)
                .and_then(|(prompt, completion)| prompt.checked_add(completion))
        });
        if prompt_tokens.is_none() && completion_tokens.is_none() && total_tokens.is_none() {
            None
        } else {
            Some(Self {
                prompt_tokens,
                completion_tokens,
                total_tokens,
            })
        }
    }

    pub fn display(self) -> String {
        match (
            self.total_tokens,
            self.prompt_tokens,
            self.completion_tokens,
        ) {
            (Some(total), _, _) => format!("{total} tok"),
            (None, Some(prompt), Some(completion)) => format!("{prompt} in · {completion} out"),
            (None, Some(prompt), None) => format!("{prompt} in"),
            (None, None, Some(completion)) => format!("{completion} out"),
            (None, None, None) => "usage unavailable".to_string(),
        }
    }

    pub fn tokens_used(self) -> Option<u64> {
        self.total_tokens
    }
}

impl App {
    pub fn apply_caps(&mut self, caps: &Value) {
        self.model_context_windows = model_context_windows(caps);
        self.model_reasoning_caps = model_reasoning_caps(caps);
        self.default_context_window_tokens =
            default_context_window(caps, &self.model_context_windows);
        let mut changed = false;
        if empty_optional_str(self.model.as_deref()) {
            if let Some(model) = resolved_default_chat_model(caps) {
                self.model = Some(model);
                changed = true;
            }
        }
        if empty_optional_str(self.mode.as_deref()) {
            if let Some(mode) = default_chat_mode(caps) {
                self.mode = Some(mode.to_string());
                changed = true;
            }
        }
        if changed {
            self.refresh_session_header_item();
        }
    }

    pub(super) fn set_recent_sessions(&mut self, mut items: Vec<PickerItem>) {
        items.retain(|item| !item.id.trim().is_empty());
        if !items.iter().any(|item| item.id == self.chat_id) {
            items.insert(
                0,
                PickerItem {
                    id: self.chat_id.clone(),
                    title: self.session_header_title(),
                    description: self.session_header_subtitle(),
                },
            );
        }
        let mut seen = HashSet::new();
        items.retain(|item| seen.insert(item.id.clone()));
        self.recent_sessions = items;
    }

    pub(super) fn sync_current_session_in_recent(&mut self) {
        let title = self.session_header_title();
        let description = self.session_header_subtitle();
        if let Some(item) = self
            .recent_sessions
            .iter_mut()
            .find(|item| item.id == self.chat_id)
        {
            item.title = title;
            item.description = description;
        } else {
            self.recent_sessions.insert(
                0,
                PickerItem {
                    id: self.chat_id.clone(),
                    title,
                    description,
                },
            );
        }
    }

    pub(super) fn switch_recent_session(&mut self, delta: isize) -> AppAction {
        self.sync_current_session_in_recent();
        if self.recent_sessions.len() <= 1 {
            self.add_notice("No other recent chats for this project yet");
            return AppAction::RefreshRecentSessions;
        }
        let len = self.recent_sessions.len();
        let current = self
            .recent_sessions
            .iter()
            .position(|item| item.id == self.chat_id)
            .unwrap_or(0);
        let next = if delta < 0 {
            current.checked_sub(1).unwrap_or(len - 1)
        } else {
            (current + 1) % len
        };
        let item = self.recent_sessions[next].clone();
        self.resume_chat(item.id, item.title, Some(item.description))
    }

    pub(super) fn execute_session_command(
        &mut self,
        command: command_session::SessionCommand,
        args: &str,
    ) -> AppAction {
        match command {
            command_session::SessionCommand::New => {
                if self.composer.text().trim() == "/new" {
                    self.composer.clear();
                }
                self.start_new_chat()
            }
            command_session::SessionCommand::Resume => {
                self.composer.clear();
                self.start_session_lookup()
            }
            command_session::SessionCommand::Fork => {
                self.composer.clear();
                self.fork_chat()
            }
            command_session::SessionCommand::Rename => self.rename_chat(args),
            command_session::SessionCommand::Archive => {
                self.composer.clear();
                self.archive_chat()
            }
            command_session::SessionCommand::Model => {
                self.composer.clear();
                AppAction::LoadModels
            }
            command_session::SessionCommand::Mode => {
                self.composer.clear();
                AppAction::LoadModes
            }
            command_session::SessionCommand::Reasoning => {
                self.composer.clear();
                if self.is_chat_active() {
                    self.add_notice(
                        "/reasoning is available between turns only; retry after the current turn finishes",
                    );
                    return AppAction::None;
                }
                match command_session::parse_reasoning_level(args) {
                    Ok(Some(level)) => self.set_reasoning_level(level),
                    Ok(None) => {
                        self.open_reasoning_picker();
                        AppAction::None
                    }
                    Err(error) => {
                        self.add_notice(format!("/reasoning {error}"));
                        AppAction::None
                    }
                }
            }
            command_session::SessionCommand::Permissions => {
                self.composer.clear();
                self.open_permissions_picker();
                AppAction::None
            }
            command_session::SessionCommand::Status => {
                self.composer.clear();
                self.show_status_card();
                AppAction::LoadDaemonStatus
            }
            command_session::SessionCommand::Init => {
                self.submit_structured_prompt(command_session::init_prompt())
            }
        }
    }

    pub(super) fn set_reasoning_level(
        &mut self,
        level: command_session::ReasoningLevel,
    ) -> AppAction {
        if self.is_chat_active() {
            self.add_notice(
                "/reasoning is available between turns only; retry after the current turn finishes",
            );
            return AppAction::None;
        }
        if !self.reasoning_level_supported(level) {
            self.add_reasoning_unsupported_notice();
            return AppAction::None;
        }
        let previous = self.reasoning_snapshot();
        let patch = command_session::reasoning_patch(level);
        self.apply_reasoning_level(level);
        self.pending_reasoning_rollback = Some(PendingReasoningRollback {
            patch: patch.clone(),
            previous: previous.clone(),
        });
        self.add_notice(format!(
            "Reasoning set to {} for subsequent turns",
            level.as_str()
        ));
        AppAction::SetParams { patch }
    }

    pub(super) fn reasoning_snapshot(&self) -> ReasoningStateSnapshot {
        ReasoningStateSnapshot {
            boost_reasoning: self.boost_reasoning,
            reasoning_effort: self.reasoning_effort.clone(),
        }
    }

    pub(super) fn restore_reasoning_snapshot(&mut self, snapshot: ReasoningStateSnapshot) {
        self.boost_reasoning = snapshot.boost_reasoning;
        self.reasoning_effort = snapshot.reasoning_effort;
    }

    pub(super) fn apply_reasoning_level(&mut self, level: command_session::ReasoningLevel) {
        match level {
            command_session::ReasoningLevel::Off => {
                self.clear_reasoning_level();
            }
            command_session::ReasoningLevel::On => {
                self.boost_reasoning = true;
                self.reasoning_effort = None;
            }
            _ => {
                self.boost_reasoning = true;
                self.reasoning_effort = Some(level.as_str().to_string());
            }
        }
    }

    pub(super) fn clear_reasoning_level(&mut self) {
        self.boost_reasoning = false;
        self.reasoning_effort = None;
    }

    pub(super) fn reasoning_level_supported(&self, level: command_session::ReasoningLevel) -> bool {
        if level == command_session::ReasoningLevel::Off {
            return true;
        }
        self.current_reasoning_caps()
            .is_some_and(|caps| match level {
                command_session::ReasoningLevel::On => caps.has_reasoning_support(),
                command_session::ReasoningLevel::Off => true,
                _ => caps.supports_effort(level),
            })
    }

    pub(super) fn supported_reasoning_levels(&self) -> Vec<command_session::ReasoningLevel> {
        let Some(caps) = self.current_reasoning_caps() else {
            return Vec::new();
        };
        if !caps.has_reasoning_support() {
            return Vec::new();
        }
        let mut levels = vec![
            command_session::ReasoningLevel::Off,
            command_session::ReasoningLevel::On,
        ];
        levels.extend(
            command_session::REASONING_LEVELS
                .into_iter()
                .filter(|level| {
                    !matches!(
                        level,
                        command_session::ReasoningLevel::Off | command_session::ReasoningLevel::On
                    ) && caps.supports_effort(*level)
                }),
        );
        levels
    }

    pub(super) fn current_reasoning_caps(&self) -> Option<&ReasoningModelCaps> {
        self.model
            .as_deref()
            .and_then(|model| reasoning_caps_for_model(&self.model_reasoning_caps, model))
    }

    pub(super) fn add_reasoning_unsupported_notice(&mut self) {
        let model = self.model().unwrap_or("current model");
        let notice = self
            .current_reasoning_caps()
            .filter(|caps| caps.reasoning_support_is_confirmed_absent())
            .map(|_| {
                format!(
                    "Reasoning effort is not available for {model}. Choose a reasoning-capable model first."
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "Reasoning capability is not confirmed for {model}. Choose a reasoning-capable model first."
                )
            });
        self.add_notice(notice);
    }

    pub(super) fn show_status_card(&mut self) {
        self.push_history_item(TranscriptItem::Status(
            command_session::status_snapshot(
                self.daemon_online,
                self.daemon_status.as_ref(),
                self.daemon_base_url.clone(),
                workers::worker_status_line(self.current_worker()),
                self.current_project()
                    .map(|project| project.slug.clone())
                    .unwrap_or_else(|| "-".to_string()),
                self.current_project()
                    .map(|project| project.root.display().to_string()),
                self.model().unwrap_or("default").to_string(),
                self.mode().unwrap_or("agent").to_string(),
                self.reasoning_effort_label().to_string(),
                self.permission_policy,
                self.chat_id.clone(),
                self.usage().map(|usage| command_session::StatusUsage {
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                    total_tokens: usage.tokens_used(),
                    context_window_tokens: self.context_window_tokens(),
                }),
                self.retry_hint.clone(),
            ),
            self.theme.clone(),
        ));
    }

    pub(super) fn set_project(&mut self, project: OpenProjectResponse) {
        self.cancel_backtrack();
        self.abort_in_flight = false;
        self.transcript_overlay = None;
        self.save_local_input_handoff();
        self.history_path = Some(history_path_for_root(&project.root));
        let history_entries = self
            .history_path
            .as_deref()
            .map(load_history)
            .unwrap_or_default();
        self.server_queue_size = 0;
        self.server_queue_previews.clear();
        self.current_project = Some(project.clone());
        self.chat_id = self
            .last_chat_by_project
            .get(&project.project_id)
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.restore_local_input_handoff(history_entries);
        self.session_title = None;
        self.recent_sessions.clear();
        self.show_session_header = true;
        self.set_session_state(SessionState::Idle);
        self.replace_with_notice(format!(
            "Switched to project {} at {}",
            project.slug,
            project.root.display()
        ));
        self.clear_stream_controllers();
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.clear();
        self.composer_mode = ComposerMode::Chat;
        self.modal_picker = None;
        self.clear_approvals();
        self.selected_tool_index = None;
        self.usage = None;
        self.model = None;
        self.mode = None;
        self.clear_pending_target_params();
        self.clear_reasoning_level();
        self.model_context_windows.clear();
        self.model_reasoning_caps.clear();
        self.clear_ask_questions_state();
        self.default_context_window_tokens = None;
        self.retry_hint = None;
        self.sync_current_session_in_recent();
    }

    pub(super) fn new_chat(&mut self) {
        self.open_chat_shell(
            uuid::Uuid::new_v4().to_string(),
            None,
            "New chat started".to_string(),
        );
    }

    pub(super) fn start_new_chat(&mut self) -> AppAction {
        self.new_chat();
        AppAction::SubscribeCurrent
    }

    pub(super) fn open_chat_shell(
        &mut self,
        chat_id: String,
        title: Option<String>,
        _notice: String,
    ) {
        self.cancel_backtrack();
        self.abort_in_flight = false;
        self.transcript_overlay = None;
        let history_entries = self.composer.history_entries().to_vec();
        self.save_local_input_handoff();
        self.chat_id = chat_id;
        self.restore_local_input_handoff(history_entries);
        self.session_title = title;
        self.show_session_header = true;
        self.server_queue_size = 0;
        self.server_queue_previews.clear();
        self.model = None;
        self.mode = None;
        self.clear_pending_target_params();
        self.clear_reasoning_level();
        self.replace_with_session(
            self.session_header_title(),
            Some(self.session_header_subtitle()),
        );
        self.set_session_state(SessionState::Idle);
        self.clear_stream_controllers();
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.truncate(1);
        self.clear_approvals();
        self.selected_tool_index = None;
        self.usage = None;
        self.clear_ask_questions_state();
        self.retry_hint = None;
        self.sync_current_session_in_recent();
    }

    pub(super) fn resume_chat(
        &mut self,
        chat_id: String,
        title: String,
        _subtitle: Option<String>,
    ) -> AppAction {
        self.cancel_backtrack();
        self.abort_in_flight = false;
        self.transcript_overlay = None;
        let history_entries = self.composer.history_entries().to_vec();
        self.save_local_input_handoff();
        self.chat_id = chat_id;
        self.restore_local_input_handoff(history_entries);
        self.session_title = Some(title.clone());
        self.show_session_header = true;
        self.server_queue_size = 0;
        self.server_queue_previews.clear();
        self.model = None;
        self.mode = None;
        self.clear_reasoning_level();
        self.clear_pending_target_params();
        self.replace_with_session(
            self.session_header_title(),
            Some(self.session_header_subtitle()),
        );
        self.set_session_state(SessionState::Idle);
        self.clear_stream_controllers();
        self.rendered_state_cursor = 0;
        self.rendered_state_keys.truncate(1);
        self.clear_approvals();
        self.selected_tool_index = None;
        self.usage = None;
        self.clear_ask_questions_state();
        self.retry_hint = None;
        self.sync_current_session_in_recent();
        AppAction::SubscribeCurrent
    }

    pub(super) fn fork_chat(&mut self) -> AppAction {
        self.cancel_backtrack();
        self.transcript_overlay = None;
        let Some(up_to_message_id) = last_branch_message_id(self.transcript_state.messages())
        else {
            self.add_notice(
                "/fork unavailable until the resumed chat snapshot contains message ids",
            );
            return AppAction::None;
        };
        let target_chat_id = uuid::Uuid::new_v4().to_string();
        let title = self
            .session_title
            .as_ref()
            .map(|title| format!("Fork of {title}"));
        let source_chat_id = self.chat_id.clone();
        self.add_notice("Forking chat…");
        AppAction::ForkChat {
            target_chat_id,
            source_chat_id,
            up_to_message_id,
            title,
        }
    }

    pub(super) fn open_forked_chat(
        &mut self,
        target_chat_id: String,
        title: Option<String>,
    ) -> AppAction {
        self.open_chat_shell(target_chat_id, title, "Forked chat started".to_string());
        AppAction::SubscribeCurrent
    }

    pub(super) fn rename_chat(&mut self, args: &str) -> AppAction {
        if !args.is_empty() {
            self.composer.set_text(args);
        }
        let title = self.composer.text().trim().to_string();
        self.composer.clear();
        if title.is_empty() {
            self.add_notice("/rename needs the new title in the composer first");
            return AppAction::None;
        }
        self.add_notice(format!("Renaming chat to {title}"));
        AppAction::RenameChat { title }
    }

    pub(super) fn apply_renamed_chat(&mut self, title: String) {
        self.session_title = Some(title);
        self.show_session_header = true;
        self.sync_current_session_in_recent();
    }

    pub(super) fn archive_chat(&mut self) -> AppAction {
        let chat_id = self.chat_id.clone();
        let new_chat_id = uuid::Uuid::new_v4().to_string();
        self.add_notice("Archiving current chat from recent sessions");
        AppAction::ArchiveChat {
            chat_id,
            new_chat_id,
        }
    }

    pub(super) fn apply_archived_chat(&mut self, new_chat_id: String) -> AppAction {
        self.open_chat_shell(new_chat_id, None, "New chat started".to_string());
        AppAction::SubscribeCurrent
    }

    pub(super) fn clear_pending_target_params(&mut self) {
        self.pending_model = None;
        self.pending_mode = None;
        self.in_flight_send = None;
        self.pending_send_retry = None;
        self.pending_reasoning_rollback = None;
        self.pending_backtrack_rollback = None;
    }

    pub(super) fn save_local_input_handoff(&mut self) {
        let Some(owner) = self.input_queue_owner.take() else {
            return;
        };
        self.last_chat_by_project
            .insert(owner.project_id.clone(), owner.chat_id.clone());
        self.local_input_handoffs.insert(
            owner,
            LocalInputHandoff {
                composer: std::mem::replace(&mut self.composer, ComposerState::new(Vec::new())),
                input_queue: std::mem::take(&mut self.input_queue),
            },
        );
    }

    pub(super) fn restore_local_input_handoff(&mut self, history_entries: Vec<String>) {
        let owner = self.current_local_input_owner();
        let handoff = owner
            .as_ref()
            .and_then(|owner| self.local_input_handoffs.remove(owner));
        match handoff {
            Some(handoff) => {
                self.composer = handoff.composer;
                self.input_queue = handoff.input_queue;
            }
            None => {
                self.composer = ComposerState::new(history_entries);
                self.input_queue = InputQueue::new();
            }
        }
        self.input_queue_owner = owner;
    }

    pub(super) fn input_queue_matches_current_session(&self) -> bool {
        self.input_queue_owner.as_ref() == self.current_local_input_owner().as_ref()
    }

    fn current_local_input_owner(&self) -> Option<LocalInputOwner> {
        self.current_project
            .as_ref()
            .map(|project| LocalInputOwner {
                project_id: project.project_id.clone(),
                chat_id: self.chat_id.clone(),
            })
    }

    pub(super) fn clear_ask_questions_state(&mut self) {
        self.ask_questions_form = None;
        self.pending_manual_ask_questions = None;
        self.handled_ask_questions_tool_ids.clear();
    }

    pub(super) fn clear_active_ask_questions(&mut self) {
        self.ask_questions_form = None;
        self.pending_manual_ask_questions = None;
    }

    pub(super) fn take_pending_params(&mut self) -> Value {
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
}

pub(super) fn token_count(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value.get(*key)?.as_u64())
}

pub(super) fn model_context_windows(caps: &Value) -> HashMap<String, u64> {
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

pub(super) fn model_reasoning_caps(caps: &Value) -> HashMap<String, ReasoningModelCaps> {
    let mut out = HashMap::new();
    if let Some(models) = caps.get("chat_models") {
        collect_model_reasoning_caps(models, &mut out);
    }
    if let Some(models) = caps.get("models").and_then(|models| models.get("chat")) {
        collect_model_reasoning_caps(models, &mut out);
    }
    if let Some(models) = caps.get("available_models") {
        collect_model_reasoning_caps(models, &mut out);
    }
    out
}

pub(super) fn collect_model_context_windows(models: &Value, windows: &mut HashMap<String, u64>) {
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

pub(super) fn collect_model_reasoning_caps(
    models: &Value,
    reasoning: &mut HashMap<String, ReasoningModelCaps>,
) {
    match models {
        Value::Object(map) => {
            for (id, model) in map {
                insert_model_reasoning_caps(id, model, reasoning);
            }
        }
        Value::Array(items) => {
            for model in items {
                if let Some(id) = model.get("id").and_then(Value::as_str) {
                    insert_model_reasoning_caps(id, model, reasoning);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn insert_model_context_window(
    id: &str,
    model: &Value,
    windows: &mut HashMap<String, u64>,
) {
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

pub(super) fn insert_model_reasoning_caps(
    id: &str,
    model: &Value,
    reasoning: &mut HashMap<String, ReasoningModelCaps>,
) {
    let caps = reasoning_caps_from_model(model);
    if !id.is_empty() {
        merge_model_reasoning_caps(reasoning, id, &caps);
    }
    if let Some(model_id) = model
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        merge_model_reasoning_caps(reasoning, model_id, &caps);
    }
}

pub(super) fn merge_model_reasoning_caps(
    reasoning: &mut HashMap<String, ReasoningModelCaps>,
    id: &str,
    caps: &ReasoningModelCaps,
) {
    reasoning
        .entry(id.to_string())
        .or_default()
        .merge_from(caps);
}

pub(super) fn reasoning_caps_from_model(model: &Value) -> ReasoningModelCaps {
    ReasoningModelCaps {
        effort_options: model
            .get("reasoning_effort_options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            }),
        supports_thinking_budget: bool_field(model, "supports_thinking_budget"),
        supports_adaptive_thinking_budget: bool_field(model, "supports_adaptive_thinking_budget"),
    }
}

pub(super) fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

pub(super) fn context_window_from_model(model: &Value) -> Option<u64> {
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

pub(super) fn default_context_window(caps: &Value, windows: &HashMap<String, u64>) -> Option<u64> {
    default_chat_model(caps)
        .and_then(|model| context_window_for_model(windows, model))
        .or_else(|| {
            (windows.len() == 1)
                .then(|| windows.values().next().copied())
                .flatten()
        })
}

pub(super) fn resolved_default_chat_model(caps: &Value) -> Option<String> {
    default_chat_model(caps)
        .map(|model| resolve_chat_model_id(caps, model).unwrap_or_else(|| model.to_string()))
        .or_else(|| {
            let ids = chat_model_ids(caps);
            (ids.len() == 1).then(|| ids[0].clone())
        })
}

pub(super) fn default_chat_model(caps: &Value) -> Option<&str> {
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

pub(super) fn default_chat_mode(caps: &Value) -> Option<&str> {
    caps.get("defaults")
        .and_then(|defaults| {
            string_field(
                defaults,
                &[
                    "chat_default_mode",
                    "default_chat_mode",
                    "chat_mode",
                    "mode",
                    "tool_use",
                ],
            )
        })
        .or_else(|| {
            string_field(
                caps,
                &[
                    "chat_default_mode",
                    "default_chat_mode",
                    "chat_mode",
                    "mode",
                    "tool_use",
                ],
            )
        })
}

pub(super) fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_str())
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn empty_optional_str(value: Option<&str>) -> bool {
    match value {
        Some(value) => value.trim().is_empty(),
        None => true,
    }
}

pub(super) fn resolve_chat_model_id(caps: &Value, model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let ids = chat_model_ids(caps);
    if ids.iter().any(|id| id == model) {
        return Some(model.to_string());
    }
    let mut matches = ids
        .into_iter()
        .filter(|id| id.rsplit('/').next().is_some_and(|suffix| suffix == model));
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}

pub(super) fn chat_model_ids(caps: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(models) = caps.get("chat_models") {
        collect_chat_model_ids(models, &mut ids);
    }
    if let Some(models) = caps.get("models").and_then(|models| models.get("chat")) {
        collect_chat_model_ids(models, &mut ids);
    }
    if let Some(models) = caps.get("available_models") {
        collect_chat_model_ids(models, &mut ids);
    }
    ids
}

pub(super) fn collect_chat_model_ids(models: &Value, ids: &mut Vec<String>) {
    match models {
        Value::Object(map) => {
            for (id, model) in map {
                push_unique_model_id(ids, id);
                if let Some(model_id) = model.get("id").and_then(Value::as_str) {
                    push_unique_model_id(ids, model_id);
                }
            }
        }
        Value::Array(items) => {
            for model in items {
                if let Some(model_id) = model.get("id").and_then(Value::as_str) {
                    push_unique_model_id(ids, model_id);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn push_unique_model_id(ids: &mut Vec<String>, id: &str) {
    let id = id.trim();
    if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
        ids.push(id.to_string());
    }
}

pub(super) fn context_window_for_model(windows: &HashMap<String, u64>, model: &str) -> Option<u64> {
    windows.get(model).copied().or_else(|| {
        let mut matches = windows
            .iter()
            .filter(|(id, _)| id.rsplit('/').next().is_some_and(|suffix| suffix == model));
        let (_, window) = matches.next()?;
        matches.next().is_none().then_some(*window)
    })
}

pub(super) fn reasoning_caps_for_model<'a>(
    reasoning: &'a HashMap<String, ReasoningModelCaps>,
    model: &str,
) -> Option<&'a ReasoningModelCaps> {
    reasoning.get(model).or_else(|| {
        let mut matches = reasoning
            .iter()
            .filter(|(id, _)| id.rsplit('/').next().is_some_and(|suffix| suffix == model));
        let (_, caps) = matches.next()?;
        matches.next().is_none().then_some(caps)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn next_project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p2".to_string(),
            slug: "next".to_string(),
            root: PathBuf::from("/tmp/next"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    #[test]
    fn new_chat_replaces_the_transcript_with_a_session_header() {
        let mut app = App::new(project());

        app.new_chat();

        assert!(matches!(
            app.visible_transcript().first(),
            Some(TranscriptItem::Session { title, .. }) if title == "New chat"
        ));
        assert_eq!(app.session_state(), SessionState::Idle);
    }

    #[test]
    fn recent_session_switch_selects_the_requested_chat() {
        let mut app = App::new(project());
        let current = app.chat_id().to_string();
        app.set_recent_sessions(vec![
            PickerItem {
                id: current,
                title: "Current".to_string(),
                description: "now".to_string(),
            },
            PickerItem {
                id: "chat-next".to_string(),
                title: "Next chat".to_string(),
                description: "recent".to_string(),
            },
        ]);

        assert_eq!(app.switch_recent_session(1), AppAction::SubscribeCurrent);
        assert_eq!(app.chat_id(), "chat-next");
        assert_eq!(app.session_title(), Some("Next chat"));
    }

    #[test]
    fn resuming_a_chat_restores_its_queued_prompts() {
        let mut app = App::new(project());
        app.input_queue
            .enqueue("first queued prompt".to_string(), Value::Null);
        app.input_queue
            .enqueue("second queued prompt".to_string(), Value::Null);
        app.composer.set_text("saved draft");
        let original_chat_id = app.chat_id().to_string();

        app.resume_chat("chat-next".to_string(), "Next chat".to_string(), None);

        assert!(app.input_queue.is_empty());
        assert!(app.composer.is_empty());

        app.resume_chat(original_chat_id, "Original chat".to_string(), None);

        assert_eq!(app.input_queue.len(), 2);
        assert_eq!(app.input_queue.items()[0].text, "first queued prompt");
        assert_eq!(app.input_queue.items()[1].text, "second queued prompt");
        assert_eq!(app.composer.text(), "saved draft");
    }

    #[test]
    fn switching_projects_recovers_the_original_composer_draft() {
        let mut app = App::new(project());
        app.composer.set_text("keep this draft");

        app.set_project(next_project());

        assert!(app.composer.is_empty());

        app.set_project(project());

        assert_eq!(app.composer(), "keep this draft");
    }

    #[test]
    fn project_switch_clears_abort_in_flight_before_stale_completion() {
        let mut app = App::new(project());
        let origin = app.command_origin();
        app.abort_in_flight = true;
        app.set_session_state(SessionState::Generating);

        app.set_project(next_project());

        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        let before = app.visible_transcript().len();
        assert_eq!(
            app.handle_command_finished(CommandContextTag::Abort { origin }, Ok(())),
            AppAction::None
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        assert_eq!(app.visible_transcript().len(), before);
    }

    #[test]
    fn resume_chat_clears_abort_in_flight_before_stale_completion() {
        let mut app = App::new(project());
        let origin = app.command_origin();
        app.abort_in_flight = true;
        app.set_session_state(SessionState::Generating);

        app.resume_chat("chat-next".to_string(), "Next chat".to_string(), None);

        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        let before = app.visible_transcript().len();
        assert_eq!(
            app.handle_command_finished(CommandContextTag::Abort { origin }, Ok(())),
            AppAction::None
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        assert_eq!(app.visible_transcript().len(), before);
    }

    #[test]
    fn new_chat_clears_abort_in_flight_before_stale_completion() {
        let mut app = App::new(project());
        let origin = app.command_origin();
        app.abort_in_flight = true;
        app.set_session_state(SessionState::Generating);

        app.new_chat();

        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        let before = app.visible_transcript().len();
        assert_eq!(
            app.handle_command_finished(CommandContextTag::Abort { origin }, Ok(())),
            AppAction::None
        );
        assert_eq!(app.session_state(), SessionState::Idle);
        assert!(!app.abort_in_flight);
        assert_eq!(app.visible_transcript().len(), before);
    }

    #[test]
    fn usage_summary_preserves_partial_empty_and_complete_reports() {
        let partial = UsageSummary::from_value(&serde_json::json!({"prompt_tokens": 12})).unwrap();
        assert_eq!(partial.prompt_tokens, Some(12));
        assert_eq!(partial.completion_tokens, None);
        assert_eq!(partial.total_tokens, None);
        assert_eq!(partial.tokens_used(), None);
        assert_eq!(partial.display(), "12 in");

        assert_eq!(UsageSummary::from_value(&serde_json::json!({})), None);

        let complete = UsageSummary::from_value(&serde_json::json!({
            "prompt_tokens": 12,
            "completion_tokens": 8,
        }))
        .unwrap();
        assert_eq!(complete.total_tokens, Some(20));
        assert_eq!(complete.tokens_used(), Some(20));
        assert_eq!(complete.display(), "20 tok");
    }

    #[test]
    fn reasoning_capability_flags_preserve_unknown_and_gate_controls() {
        for key in [
            "supports_thinking_budget",
            "supports_adaptive_thinking_budget",
        ] {
            for value in [
                serde_json::json!({}),
                serde_json::json!({(key): null}),
                serde_json::json!({(key): "true"}),
            ] {
                let caps = reasoning_caps_from_model(&value);
                assert_eq!(bool_field(&value, key), None);
                assert!(!caps.has_reasoning_support());
            }
        }
        for key in [
            "supports_thinking_budget",
            "supports_adaptive_thinking_budget",
        ] {
            let supported = serde_json::json!({(key): true});
            assert_eq!(bool_field(&supported, key), Some(true));
            assert!(reasoning_caps_from_model(&supported).has_reasoning_support());

            let unsupported = serde_json::json!({(key): false});
            assert_eq!(bool_field(&unsupported, key), Some(false));
            assert!(!reasoning_caps_from_model(&unsupported).has_reasoning_support());
        }

        let mut app = App::new(project());
        app.model = Some("unknown".to_string());
        app.model_reasoning_caps.insert(
            "unknown".to_string(),
            reasoning_caps_from_model(&serde_json::json!({})),
        );
        assert!(!app.reasoning_level_supported(command_session::ReasoningLevel::On));

        app.model_reasoning_caps.insert(
            "unknown".to_string(),
            reasoning_caps_from_model(&serde_json::json!({"supports_thinking_budget": true})),
        );
        assert!(app.reasoning_level_supported(command_session::ReasoningLevel::On));

        app.model_reasoning_caps.insert(
            "unknown".to_string(),
            reasoning_caps_from_model(&serde_json::json!({"supports_thinking_budget": false})),
        );
        assert!(!app.reasoning_level_supported(command_session::ReasoningLevel::On));
    }

    #[test]
    fn reasoning_capability_duplicate_ids_merge_sparse_fields() {
        let caps = model_reasoning_caps(&serde_json::json!({
            "chat_models": {
                "provider/model": {
                "reasoning_effort_options": ["low"],
                "supports_thinking_budget": true,
                },
            },
            "models": {"chat": [{
                "id": "provider/model",
                "supports_adaptive_thinking_budget": true,
            }]},
            "available_models": [{"id": "provider/model", "name": "Model"}],
        }));

        assert_eq!(
            caps["provider/model"].effort_options,
            Some(vec!["low".to_string()])
        );
        assert_eq!(caps["provider/model"].supports_thinking_budget, Some(true));
        assert_eq!(
            caps["provider/model"].supports_adaptive_thinking_budget,
            Some(true)
        );
    }

    #[test]
    fn reasoning_capability_source_precedence_overrides_explicit_conflicts() {
        let caps = model_reasoning_caps(&serde_json::json!({
            "available_models": [{
                "id": "provider/model",
                "reasoning_effort_options": ["low"],
                "supports_thinking_budget": true,
                "supports_adaptive_thinking_budget": true,
            }],
            "models": {"chat": [{
                "id": "provider/model",
                "supports_thinking_budget": false,
            }]},
            "chat_models": {
                "provider/model": {"reasoning_effort_options": ["high"]},
            },
        }));

        assert_eq!(
            caps["provider/model"].effort_options,
            Some(vec!["low".to_string()])
        );
        assert_eq!(caps["provider/model"].supports_thinking_budget, Some(true));
        assert_eq!(
            caps["provider/model"].supports_adaptive_thinking_budget,
            Some(true)
        );
    }

    #[test]
    fn caps_lookup_requires_unique_suffix_and_prefers_exact_ids() {
        let caps = serde_json::json!({
            "chat_models": {
                "provider-a/demo": {
                    "n_ctx": 10,
                    "reasoning_effort_options": ["low"],
                },
                "provider-b/demo": {
                    "n_ctx": 20,
                    "reasoning_effort_options": ["high"],
                },
            },
        });
        let reasoning = model_reasoning_caps(&caps);
        let windows = model_context_windows(&caps);

        assert_eq!(resolve_chat_model_id(&caps, "demo"), None);
        assert_eq!(
            resolve_chat_model_id(&caps, "provider-a/demo"),
            Some("provider-a/demo".to_string())
        );
        assert_eq!(context_window_for_model(&windows, "demo"), None);
        assert_eq!(
            context_window_for_model(&windows, "provider-a/demo"),
            Some(10)
        );
        assert_eq!(reasoning_caps_for_model(&reasoning, "demo"), None);
        assert!(reasoning_caps_for_model(&reasoning, "provider-a/demo")
            .unwrap()
            .supports_effort(command_session::ReasoningLevel::Low));
    }
}
