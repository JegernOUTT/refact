pub(crate) mod board;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::client::{
    CompetitorImportInfoResponse, CompetitorImportRunResponse, HooksResponse,
    KnowledgeGraphResponse, McpViewData, ProviderListResponse, ProviderOAuthLogoutResponse,
    SlashCommandsListResponse,
};
use crate::commands::{command_picker_items, misc, session, CommandContext, CommandPicker};
use crate::overlay::PagerOverlay;
use crate::pickers::{
    model_items_from_caps, mode_items_from_response, PickerAccept, PickerItem, PickerKind,
    PickerState,
};
use crate::read_only_views::{
    hooks_overlay, import_run_notice, import_run_overlay, import_sources_overlay, mcp_overlay,
    memories_overlay, skills_overlay, ReadOnlyView, ViewOverlay,
};
use crate::render::highlight;
use crate::sessions::{session_items_from_trajectories, TrajectoryMeta};
use crate::theme::TuiTheme;

mod history;

use super::transcript::{
    citation_item, collapsed_unknown_payload, line_to_plain_string, server_content_block_item,
    thinking_block_items, value_to_compact_string, visible_message_content,
};
use super::*;

pub(super) mod activity;
mod settings;

pub(crate) use settings::{
    model_settings_caps, settings_caps_for_model, ModelSettingsCapabilities, SettingsRow,
    SettingsState,
};
mod worktree;
pub(super) use worktree::{WorktreeAction, WorktreeMergeConfirmation};

#[derive(Debug, Clone)]
pub struct ProjectPickerState {
    projects: Vec<ProjectEntry>,
    pub filter: String,
    pub selected: usize,
}

impl ProjectPickerState {
    pub(super) fn new(projects: Vec<ProjectEntry>) -> Self {
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

    pub(super) fn selected_project(&self) -> Option<ProjectEntry> {
        self.filtered_projects().get(self.selected).cloned()
    }

    pub(super) fn clamp_selection(&mut self) {
        let len = self.filtered_projects().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ThemePickerSnapshot {
    theme: TuiTheme,
    syntax_theme: syntect::highlighting::Theme,
}

pub use history::{HistoryAction, HistorySurface};

impl App {
    pub(in crate::app) fn execute_misc_command(
        &mut self,
        command: misc::MiscCommand,
        args: &str,
    ) -> AppAction {
        match command {
            misc::MiscCommand::Theme => {
                self.composer.clear();
                if args.trim().is_empty() {
                    self.open_theme_picker();
                } else {
                    self.apply_theme_name(args.trim(), true);
                }
                AppAction::None
            }
            misc::MiscCommand::ToggleVim => {
                self.composer.clear();
                let enabled = self.vim.toggle();
                let label = if enabled { "enabled" } else { "disabled" };
                self.add_notice(format!("Composer vim mode {label}"));
                AppAction::None
            }
            misc::MiscCommand::DebugConfig => {
                self.composer.clear();
                self.show_debug_config_card();
                AppAction::None
            }
            misc::MiscCommand::CopyLastAssistant => self.copy_last_assistant_message(),
            misc::MiscCommand::RawTranscript => {
                self.composer.clear();
                self.open_raw_transcript_overlay()
            }
            misc::MiscCommand::Subagents => {
                self.composer.clear();
                if activity::surfaces_enabled() {
                    self.open_activity_surface()
                } else {
                    self.show_subagents_card();
                    AppAction::None
                }
            }
            misc::MiscCommand::Mcp => self.open_read_only_view(ReadOnlyView::Mcp),
            misc::MiscCommand::Skills => self.open_read_only_view(ReadOnlyView::Skills),
            misc::MiscCommand::Memories => self.open_read_only_view(ReadOnlyView::Memories),
            misc::MiscCommand::Hooks => self.open_read_only_view(ReadOnlyView::Hooks),
            misc::MiscCommand::Logout => self.start_provider_logout(args),
            misc::MiscCommand::Import => self.start_competitor_import(args),
            misc::MiscCommand::Settings => {
                self.composer.clear();
                self.open_settings_surface();
                AppAction::None
            }
            misc::MiscCommand::Board => self.open_task_board(),
            misc::MiscCommand::Worktrees => self.start_worktree_command(args),
        }
    }

    pub(crate) fn settings_surface_enabled() -> bool {
        settings_surface_enabled_from_value(std::env::var("REFACT_TUI_SURFACES").ok().as_deref())
    }

    pub(crate) fn open_settings_surface(&mut self) {
        if !Self::settings_surface_enabled() {
            self.add_notice("/settings requires REFACT_TUI_SURFACES=1");
            return;
        }
        self.settings_surface = Some(SettingsState::new(
            &self.thread_params,
            settings_caps_for_model(&self.model_settings_caps, self.model.as_deref()),
        ));
    }

    pub(crate) fn refresh_settings_surface(&mut self) {
        if let Some(settings) = self.settings_surface.as_mut() {
            settings.refresh(
                &self.thread_params,
                settings_caps_for_model(&self.model_settings_caps, self.model.as_deref()),
            );
        }
    }

    pub(crate) fn update_thread_params(&mut self, params: &Value) {
        let Some(update) = params.as_object() else {
            return;
        };
        let thread = self
            .thread_params
            .as_object_mut()
            .expect("thread params object");
        for (key, value) in update {
            thread.insert(key.clone(), value.clone());
        }
        self.refresh_settings_surface();
    }

    pub(crate) fn settings_rows(&self) -> Vec<SettingsRow> {
        self.settings_surface
            .as_ref()
            .map(SettingsState::rows)
            .unwrap_or_default()
    }

    pub(crate) fn settings_selected(&self) -> usize {
        self.settings_surface
            .as_ref()
            .map(SettingsState::selected)
            .unwrap_or_default()
    }

    pub(crate) fn settings_surface_open(&self) -> bool {
        self.settings_surface.is_some()
    }

    #[cfg(test)]
    pub(crate) fn test_open_settings_surface(&mut self) {
        self.settings_surface = Some(SettingsState::new(
            &self.thread_params,
            settings_caps_for_model(&self.model_settings_caps, self.model.as_deref()),
        ));
    }

    pub fn set_transcript_overlay_visible_height(&mut self, height: usize) {
        self.transcript_overlay_visible_height = Some(height);
    }

    pub(super) fn open_project_picker(&mut self, projects: Vec<ProjectEntry>) {
        self.picker = ProjectPickerState::new(projects);
        self.composer_mode = ComposerMode::ProjectPicker;
        self.modal_picker = None;
    }

    pub(super) fn open_model_picker(&mut self, caps: Value) {
        self.apply_caps(&caps);
        let items = model_items_from_caps(&caps);
        if items.is_empty() {
            self.add_notice("No models returned by caps");
        } else {
            let mut picker = PickerState::new(PickerKind::Model, items);
            if let Some(model) = self
                .model
                .as_deref()
                .filter(|model| !model.trim().is_empty())
            {
                let selected =
                    resolve_chat_model_id(&caps, model).unwrap_or_else(|| model.to_string());
                picker.select_item_id(&selected);
            }
            self.modal_picker = Some(picker);
            self.composer_mode = ComposerMode::Chat;
        }
    }

    pub(super) fn open_mode_picker(&mut self, modes: Value) {
        if !Self::settings_surface_enabled() {
            self.add_notice("/mode requires REFACT_TUI_SURFACES=1");
            return;
        }
        let items = mode_items_from_response(&modes);
        if items.is_empty() {
            self.add_notice("No modes returned by worker");
        } else {
            self.mode_records = items.clone();
            let current = self
                .mode
                .as_deref()
                .filter(|mode| !mode.trim().is_empty())
                .unwrap_or("agent");
            let picker = PickerState::modes(items, Some(current));
            self.modal_picker = Some(picker);
            self.composer_mode = ComposerMode::Chat;
        }
    }

    #[cfg(test)]
    pub(crate) fn test_open_mode_picker(&mut self, modes: Value) {
        let items = mode_items_from_response(&modes);
        if items.is_empty() {
            return;
        }
        self.mode_records = items.clone();
        let current = self
            .mode
            .as_deref()
            .filter(|mode| !mode.trim().is_empty())
            .unwrap_or("agent");
        self.modal_picker = Some(PickerState::modes(items, Some(current)));
        self.composer_mode = ComposerMode::Chat;
    }

    pub(super) fn open_slash_command_picker(&mut self) {
        let items = command_picker_items(CommandContext {
            active_turn: self.is_chat_active(),
        });
        self.modal_picker = Some(PickerState::new(PickerKind::SlashCommand, items));
        self.composer_mode = ComposerMode::Chat;
    }

    pub(super) fn open_file_mention_picker(&mut self, items: Vec<PickerItem>) {
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

    pub(super) fn open_session_picker(&mut self, items: Vec<PickerItem>) {
        self.set_recent_sessions(items.clone());
        if items.is_empty() {
            self.add_notice("No recent chats for this project yet. Continue this new chat or press Ctrl-N for another fresh one.");
            self.modal_picker = None;
            self.composer_mode = ComposerMode::Chat;
        } else {
            self.modal_picker = Some(PickerState::new(PickerKind::Session, items));
            self.composer_mode = ComposerMode::Chat;
        }
    }

    pub fn open_history_surface(&mut self, trajectories: Vec<TrajectoryMeta>) {
        self.history_surface = Some(HistorySurface::new(trajectories));
        self.modal_picker = None;
        self.composer_mode = ComposerMode::Chat;
    }

    pub(super) fn open_session_picker_from_trajectories(
        &mut self,
        trajectories: Vec<TrajectoryMeta>,
    ) {
        if history::surfaces_enabled_from_env() {
            self.history_surface = Some(HistorySurface::new(trajectories));
            self.modal_picker = None;
            self.composer_mode = ComposerMode::Chat;
        } else {
            let items = session_items_from_trajectories(trajectories, chrono::Utc::now());
            self.open_session_picker(items);
        }
    }

    pub(super) fn refresh_recent_sessions_from_trajectories(
        &mut self,
        trajectories: Vec<TrajectoryMeta>,
    ) {
        let items = session_items_from_trajectories(trajectories, chrono::Utc::now());
        self.set_recent_sessions(items);
    }

    pub(super) fn open_permissions_picker(&mut self) {
        if !Self::settings_surface_enabled() {
            self.add_notice("/permissions requires REFACT_TUI_SURFACES=1");
            return;
        }
        self.open_permissions_picker_state();
    }

    fn open_permissions_picker_state(&mut self) {
        let mut picker = PickerState::multi_with_selected_editable(
            PickerKind::Permissions,
            session::permission_picker_items(),
            session::selected_permission_ids(self.permission_policy),
            session::permission_picker_editable_ids(),
        );
        picker.select_item_id("editing_tools");
        self.modal_picker = Some(picker);
        self.composer_mode = ComposerMode::Chat;
    }

    #[cfg(test)]
    pub(crate) fn test_open_permissions_picker(&mut self) {
        self.open_permissions_picker_state();
    }

    pub(super) fn open_reasoning_picker(&mut self) {
        let levels = self.supported_reasoning_levels();
        if levels.is_empty() {
            self.add_reasoning_unsupported_notice();
            return;
        }
        self.modal_picker = Some(PickerState::new(
            PickerKind::Reasoning,
            session::reasoning_picker_items(&levels),
        ));
        self.composer_mode = ComposerMode::Chat;
    }

    pub(super) fn open_theme_picker(&mut self) {
        self.theme_picker_snapshot = Some(ThemePickerSnapshot {
            theme: self.theme.clone(),
            syntax_theme: highlight::current_syntax_theme(),
        });
        let theme_home = self.tui_theme_home();
        let mut picker = PickerState::new(
            PickerKind::Theme,
            misc::theme_picker_items(theme_home.as_deref()),
        );
        if let Some(index) = picker
            .filtered_items()
            .iter()
            .position(|item| item.id == self.theme.name())
        {
            picker.selected = index;
        }
        self.modal_picker = Some(picker);
        self.composer_mode = ComposerMode::Chat;
    }

    pub(super) fn set_theme(&mut self, theme: TuiTheme) {
        let syntax_name = theme.syntax_theme_name().to_string();
        let warning = highlight::set_theme_override(Some(syntax_name), self.tui_theme_home());
        self.history.set_theme(theme.clone());
        self.theme = theme;
        if let Some(warning) = warning {
            self.add_notice(warning);
        }
    }

    pub(super) fn preview_theme_name(&mut self, name: &str) {
        if let Some(theme) = TuiTheme::named_or_syntax(name, self.tui_theme_home().as_deref()) {
            self.set_theme(theme);
        }
    }

    pub(super) fn tui_theme_home(&self) -> Option<PathBuf> {
        self.tui_config_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
    }

    pub(super) fn preview_current_theme_picker_selection(&mut self) {
        let Some(picker) = self.modal_picker.as_ref() else {
            return;
        };
        if picker.kind != PickerKind::Theme {
            return;
        }
        let Some(item) = picker.selected_item() else {
            return;
        };
        self.preview_theme_name(&item.id);
    }

    pub(super) fn cancel_modal_picker(&mut self) {
        if self
            .modal_picker
            .as_ref()
            .is_some_and(|picker| picker.kind == PickerKind::Theme)
        {
            self.restore_theme_picker_snapshot();
        }
        self.modal_picker = None;
    }

    pub(super) fn restore_theme_picker_snapshot(&mut self) {
        if let Some(snapshot) = self.theme_picker_snapshot.take() {
            let syntax_name = snapshot.theme.syntax_theme_name().to_string();
            self.theme = snapshot.theme;
            let _ = highlight::set_theme_override(Some(syntax_name), self.tui_theme_home());
            highlight::set_syntax_theme(snapshot.syntax_theme);
        }
    }

    pub(super) fn update_slash_picker_filter(&mut self) {
        let Some(picker) = self.modal_picker.as_ref() else {
            return;
        };
        if picker.kind != PickerKind::SlashCommand {
            return;
        }
        if !self.composer.text().starts_with('/') {
            self.modal_picker = None;
            return;
        }
        let filter = self
            .composer
            .text()
            .strip_prefix('/')
            .unwrap_or_default()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if let Some(picker) = self.modal_picker.as_mut() {
            picker.set_filter(filter);
        }
    }
    pub(super) fn start_file_mention_lookup(&mut self) -> AppAction {
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

    pub(super) fn start_session_lookup(&mut self) -> AppAction {
        self.modal_picker = Some(PickerState::new(
            PickerKind::Session,
            vec![PickerItem {
                id: String::new(),
                title: "Loading recent chats…".to_string(),
                description: "via /v1/trajectories".to_string(),
            }],
        ));
        self.composer_mode = ComposerMode::Chat;
        AppAction::LoadSessions
    }

    pub(super) fn accept_modal_picker(
        &mut self,
        kind: PickerKind,
        accept: PickerAccept,
    ) -> AppAction {
        match (kind, accept) {
            (PickerKind::Model, PickerAccept::Single(Some(item))) => {
                self.pending_model = Some(item.id.clone());
                self.model = Some(item.id.clone());
                self.add_notice(format!("Model selected for next message: {}", item.title));
                AppAction::None
            }
            (PickerKind::Mode, PickerAccept::Single(Some(item))) => {
                let previous = self.mode.clone().unwrap_or_else(|| "agent".to_string());
                self.pending_mode = Some(item.id.clone());
                self.mode = Some(item.id.clone());
                self.add_notice(format!(
                    "Mode selected for next message: {previous} → {}. The server will report the resolved transition details.",
                    item.title
                ));
                AppAction::None
            }
            (PickerKind::SlashCommand, PickerAccept::Single(Some(item))) => {
                let typed = self.composer.text().trim();
                let command = if typed.contains(char::is_whitespace) {
                    typed.to_string()
                } else {
                    item.id
                };
                self.execute_command_name(&command)
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
            (PickerKind::Session, PickerAccept::Single(Some(item))) => {
                if item.id.is_empty() {
                    self.add_notice("No session selected");
                    AppAction::None
                } else {
                    self.resume_chat(item.id, item.title, Some(item.description))
                }
            }
            (PickerKind::Permissions, PickerAccept::Multi(items)) => {
                let policy = session::permission_policy_from_items(&items);
                self.permission_policy = policy;
                self.add_notice(session::permission_policy_notice(policy));
                AppAction::SetParams {
                    patch: session::permission_policy_patch(policy),
                }
            }
            (PickerKind::Reasoning, PickerAccept::Single(Some(item))) => {
                match session::parse_reasoning_level(&item.id) {
                    Ok(Some(level)) => self.set_reasoning_level(level),
                    Ok(None) => AppAction::None,
                    Err(error) => {
                        self.add_notice(format!("/reasoning {error}"));
                        AppAction::None
                    }
                }
            }
            (PickerKind::Theme, PickerAccept::Single(Some(item))) => {
                self.theme_picker_snapshot = None;
                self.apply_theme_name(&item.id, true);
                AppAction::None
            }
            (PickerKind::ProviderLogout, PickerAccept::Single(Some(item))) => {
                self.composer.clear();
                self.modal_picker = None;
                self.add_notice(format!("Logging out provider {}", item.title));
                AppAction::ProviderOAuthLogout { provider: item.id }
            }
            (PickerKind::CompetitorImport, PickerAccept::Single(Some(item))) => {
                self.composer.clear();
                self.modal_picker = None;
                if item.id == "__list__" {
                    self.open_read_only_view(ReadOnlyView::Import)
                } else {
                    self.run_competitor_import(&item.id, "project")
                }
            }
            _ => AppAction::None,
        }
    }

    pub(super) fn start_provider_logout(&mut self, args: &str) -> AppAction {
        self.composer.clear();
        let provider = args.trim();
        if provider.is_empty() {
            self.add_notice("Loading providers for /logout…");
            AppAction::LoadProviderLogoutChoices
        } else {
            self.add_notice(format!("Logging out provider {provider}"));
            AppAction::ProviderOAuthLogout {
                provider: provider.to_string(),
            }
        }
    }

    pub(super) fn start_competitor_import(&mut self, args: &str) -> AppAction {
        self.composer.clear();
        let mut parts = args.split_whitespace();
        let source = parts.next();
        let scope = parts.next().unwrap_or("project");
        if parts.next().is_some() {
            self.add_notice("/import usage: /import [source|all] [project|global]");
            return AppAction::None;
        }
        match source {
            None => {
                self.open_view_overlay(ReadOnlyView::Import.loading_overlay());
                AppAction::LoadCompetitorImportSources
            }
            Some("list") | Some("sources") => self.open_read_only_view(ReadOnlyView::Import),
            Some(source) => self.run_competitor_import(source, scope),
        }
    }

    pub(super) fn run_competitor_import(&mut self, source: &str, scope: &str) -> AppAction {
        let source = source.trim();
        let scope = scope.trim();
        if !matches!(scope, "project" | "global") {
            self.add_notice("/import scope must be project or global");
            return AppAction::None;
        }
        let source = if source.eq_ignore_ascii_case("all") {
            None
        } else {
            Some(source.to_string())
        };
        self.add_notice(format!(
            "Running /import {} {scope}…",
            source.as_deref().unwrap_or("all")
        ));
        AppAction::RunCompetitorImport {
            source,
            scope: scope.to_string(),
        }
    }

    pub(super) fn open_read_only_view(&mut self, view: ReadOnlyView) -> AppAction {
        self.composer.clear();
        self.open_view_overlay(view.loading_overlay());
        AppAction::LoadReadOnlyView { view }
    }

    pub(super) fn open_task_board(&mut self) -> AppAction {
        if !board::task_board_enabled() {
            self.add_notice("Task board is disabled; set REFACT_TUI_SURFACES=1 to enable /board");
            return AppAction::None;
        }
        self.composer.clear();
        self.board_surface = Some(board::BoardSurface::loading());
        AppAction::LoadTaskBoard {
            task_id: self.task_id.clone(),
        }
    }

    pub(super) fn show_task_board(&mut self, data: crate::client::TaskBoardViewData) {
        self.task_id = Some(data.task.id.clone());
        self.board_surface = Some(board::BoardSurface::loaded(data));
    }

    pub(super) fn show_task_board_error(&mut self, error: String) {
        self.board_surface = Some(board::BoardSurface::failed(error));
    }

    pub(super) fn open_view_overlay(&mut self, overlay: ViewOverlay) {
        let raw_lines = if overlay.raw_lines.is_empty() {
            overlay.rendered_lines.clone()
        } else {
            overlay.raw_lines
        };
        self.transcript_overlay = Some(
            PagerOverlay::new(overlay.title, overlay.rendered_lines, raw_lines)
                .with_surface(overlay.surface),
        );
    }

    pub(super) fn handle_mcp_view_loaded(&mut self, result: Result<McpViewData, String>) {
        match result {
            Ok(data) => self.open_view_overlay(mcp_overlay(&data)),
            Err(error) => self.open_failed_view(ReadOnlyView::Mcp, error),
        }
    }

    pub(super) fn handle_skills_view_loaded(
        &mut self,
        result: Result<SlashCommandsListResponse, String>,
    ) {
        match result {
            Ok(data) => self.open_view_overlay(skills_overlay(&data)),
            Err(error) => self.open_failed_view(ReadOnlyView::Skills, error),
        }
    }

    pub(super) fn handle_memories_view_loaded(
        &mut self,
        result: Result<KnowledgeGraphResponse, String>,
    ) {
        match result {
            Ok(data) => self.open_view_overlay(memories_overlay(&data)),
            Err(error) => self.open_failed_view(ReadOnlyView::Memories, error),
        }
    }

    pub(super) fn handle_hooks_view_loaded(&mut self, result: Result<HooksResponse, String>) {
        match result {
            Ok(data) => self.open_view_overlay(hooks_overlay(&data)),
            Err(error) => self.open_failed_view(ReadOnlyView::Hooks, error),
        }
    }

    pub(super) fn handle_provider_logout_choices_loaded(
        &mut self,
        result: Result<ProviderListResponse, String>,
    ) {
        match result {
            Ok(data) => self.open_provider_logout_picker(data),
            Err(error) => {
                self.retry_hint = retry_hint_from_message(&error);
                self.add_notice(format!("Failed to load providers for /logout: {error}"));
            }
        }
    }

    pub(super) fn open_provider_logout_picker(&mut self, data: ProviderListResponse) {
        let mut items = data
            .providers
            .into_iter()
            .filter(|provider| {
                matches!(
                    provider.base_provider.as_str(),
                    "claude_code" | "openai_codex" | "github_copilot"
                ) && provider.has_credentials == Some(true)
            })
            .map(|provider| PickerItem {
                id: provider.name,
                title: provider.display_name,
                description: format!("{} · {}", provider.base_provider, provider.status),
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.title
                .cmp(&right.title)
                .then_with(|| left.id.cmp(&right.id))
        });
        if items.is_empty() {
            self.add_notice("No OAuth provider credentials available to log out");
            self.modal_picker = None;
        } else {
            self.modal_picker = Some(PickerState::new(PickerKind::ProviderLogout, items));
            self.composer_mode = ComposerMode::Chat;
        }
    }

    pub(super) fn handle_provider_logout_finished(
        &mut self,
        provider: String,
        result: Result<ProviderOAuthLogoutResponse, String>,
    ) {
        match result {
            Ok(response) if response.success == Some(true) => {
                let status = if response.auth_status.trim().is_empty() {
                    "logged out".to_string()
                } else {
                    response.auth_status
                };
                self.add_notice(format!("Logged out {provider}: {status}"));
            }
            Ok(response) => {
                let status = if response.auth_status.trim().is_empty() {
                    "backend returned success=false".to_string()
                } else {
                    response.auth_status
                };
                self.add_notice(format!("Failed to log out {provider}: {status}"));
            }
            Err(error) => {
                self.retry_hint = retry_hint_from_message(&error);
                self.add_notice(format!("Failed to log out {provider}: {error}"));
            }
        }
    }

    pub(super) fn handle_competitor_import_view_loaded(
        &mut self,
        result: Result<CompetitorImportInfoResponse, String>,
    ) {
        match result {
            Ok(data) => self.open_view_overlay(import_sources_overlay(&data)),
            Err(error) => self.open_failed_view(ReadOnlyView::Import, error),
        }
    }

    pub(super) fn handle_competitor_import_sources_loaded(
        &mut self,
        result: Result<CompetitorImportInfoResponse, String>,
    ) {
        match result {
            Ok(data) => self.open_competitor_import_picker(data),
            Err(error) => self.open_failed_view(ReadOnlyView::Import, error),
        }
    }

    pub(super) fn open_competitor_import_picker(&mut self, data: CompetitorImportInfoResponse) {
        let overlay = import_sources_overlay(&data);
        let mut items = data
            .sources
            .iter()
            .map(|source| PickerItem {
                id: source.id.clone(),
                title: if source.label.trim().is_empty() {
                    source.id.clone()
                } else {
                    source.label.clone()
                },
                description: format!("roots: {}", source.roots.join(", ")),
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.title
                .cmp(&right.title)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.open_view_overlay(overlay);
        if items.is_empty() {
            self.add_notice("No competitor import sources are available");
        } else {
            items.insert(
                0,
                PickerItem {
                    id: "__list__".to_string(),
                    title: "Show source list only".to_string(),
                    description: "do not run an import".to_string(),
                },
            );
            self.modal_picker = Some(PickerState::new(PickerKind::CompetitorImport, items));
            self.composer_mode = ComposerMode::Chat;
        }
    }

    pub(super) fn handle_competitor_import_finished(
        &mut self,
        result: Result<CompetitorImportRunResponse, String>,
    ) {
        match result {
            Ok(data) => {
                self.add_notice(import_run_notice(&data));
                self.open_view_overlay(import_run_overlay(&data));
            }
            Err(error) => self.open_failed_view(ReadOnlyView::Import, error),
        }
    }

    pub(super) fn open_failed_view(&mut self, view: ReadOnlyView, error: String) {
        self.retry_hint = retry_hint_from_message(&error);
        self.add_notice(format!("Failed to load /{}: {error}", view.command_name()));
        let lines = vec![
            view.title().to_string(),
            format!("Failed to load /{} backend data.", view.command_name()),
            error,
        ];
        self.open_view_overlay(ViewOverlay {
            title: view.title().to_string(),
            rendered_lines: lines.clone(),
            raw_lines: lines,
            surface: None,
        });
    }

    pub(super) fn execute_picker_command(&mut self, picker: CommandPicker) -> AppAction {
        self.composer.clear();
        match picker {
            CommandPicker::FileMention => self.start_file_mention_lookup(),
            CommandPicker::Theme => {
                self.open_theme_picker();
                AppAction::None
            }
        }
    }

    pub(super) fn apply_theme_name(&mut self, name: &str, persist: bool) {
        match TuiTheme::named_or_syntax(name, self.tui_theme_home().as_deref()) {
            Some(theme) => {
                let theme_name = theme.name().to_string();
                self.set_theme(theme);
                self.add_notice(format!("Theme set to {theme_name}"));
                if persist {
                    match self.persist_theme_name(&theme_name) {
                        Ok(path) => self.add_notice(format!("Theme saved to {}", path.display())),
                        Err(error) => self.add_notice(error),
                    }
                }
            }
            None => self.add_notice(format!(
                "Unknown theme `{}`; available: {}",
                name,
                TuiTheme::list_available(self.tui_theme_home().as_deref())
                    .into_iter()
                    .map(|entry| entry.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    pub(super) fn persist_theme_name(&mut self, name: &str) -> Result<PathBuf, String> {
        let Some(path) = self.tui_config_path.clone() else {
            return Err("Cannot persist TUI theme; config path is unavailable".to_string());
        };
        persist_theme_name_to_path(&path, name)?;
        Ok(path)
    }

    pub(super) fn open_transcript_overlay(&mut self) -> AppAction {
        self.transcript_overlay = Some(PagerOverlay::new(
            "Transcript",
            self.transcript_rendered_text_lines(100),
            self.transcript_raw_text_lines(),
        ));
        AppAction::None
    }

    pub(super) fn open_raw_transcript_overlay(&mut self) -> AppAction {
        self.transcript_overlay = Some(PagerOverlay::raw(
            "Transcript raw",
            self.transcript_rendered_text_lines(100),
            self.transcript_raw_text_lines(),
        ));
        AppAction::None
    }

    pub(super) fn open_activity_surface(&mut self) -> AppAction {
        if !activity::surfaces_enabled() {
            self.add_notice("/subagents activity surface is gated by REFACT_TUI_SURFACES=1");
            return AppAction::None;
        }
        self.composer.clear();
        self.activity_surface = Some(activity::ActivitySurfaceState::new(
            self.inbound_event_state.background_agents(),
        ));
        self.refresh_activity_surface();
        AppAction::RefreshWorkers
    }

    pub(super) fn refresh_activity_surface(&mut self) {
        let Some(surface) = self.activity_surface.as_ref() else {
            return;
        };
        let process_cards = self
            .transcript
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Tool(card) if activity::is_process_registry_card(card) => {
                    Some(card)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let overlay = activity::overlay(
            self.inbound_event_state.background_agents(),
            self.events_pane.workers(),
            self.current_worker(),
            self.inbound_event_state.process_completed(),
            &process_cards,
            surface.selected_agent_id(),
        );
        self.transcript_overlay = Some(PagerOverlay::new(
            "Activity",
            overlay.lines.clone(),
            overlay.lines,
        ));
    }

    pub(super) fn move_activity_selection(&mut self, offset: isize) {
        if let Some(surface) = self.activity_surface.as_mut() {
            surface.move_selection(self.inbound_event_state.background_agents(), offset);
        }
        self.refresh_activity_surface();
    }

    pub(super) fn open_selected_activity_agent(&mut self) -> AppAction {
        let child_chat_id = self
            .activity_surface
            .as_ref()
            .and_then(|surface| {
                surface.selected_agent(self.inbound_event_state.background_agents())
            })
            .and_then(|agent| agent.child_chat_id.as_deref())
            .filter(|chat_id| !chat_id.trim().is_empty())
            .map(str::to_string);
        let Some(chat_id) = child_chat_id else {
            self.add_notice("Selected background agent has no child chat to open");
            return AppAction::None;
        };
        self.activity_surface = None;
        self.resume_chat(chat_id.clone(), format!("Background agent {chat_id}"), None)
    }

    pub(super) fn copy_visible_overlay_text(&mut self, height: usize) -> AppAction {
        let Some(overlay) = self.transcript_overlay.as_ref() else {
            return AppAction::None;
        };
        let text = overlay.visible_raw_text(height);
        if text.is_empty() {
            self.add_notice("No overlay text to copy");
            return AppAction::None;
        }
        AppAction::CopyToClipboard {
            text,
            source: ClipboardCopySource::OverlayVisible,
        }
    }

    pub(super) fn transcript_rendered_text_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for item in self.overlay_transcript_items() {
            lines.extend(
                crate::history::render_transcript_item_lines_with_theme(
                    &item,
                    width,
                    false,
                    &self.theme,
                )
                .iter()
                .map(line_to_plain_string),
            );
        }
        lines
    }

    pub(super) fn transcript_raw_text_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for message in self.transcript_state.messages() {
            let id = message
                .message_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!(" {value}"))
                .unwrap_or_default();
            lines.push(format!("## {}{id}", message.role.as_str()));
            if !message.reasoning.is_empty() {
                lines.push("[reasoning]".to_string());
                lines.extend(message.reasoning.lines().map(str::to_string));
            }
            if !message.content.is_empty() {
                lines.extend(message.content.lines().map(str::to_string));
            }
            for tool in &message.tool_calls {
                lines.push(format!("[tool_call] {}", value_to_compact_string(tool)));
            }
            for citation in &message.citations {
                lines.push(format!("[citation] {}", value_to_compact_string(citation)));
            }
            for block in &message.server_content_blocks {
                lines.push(format!("[server] {}", value_to_compact_string(block)));
            }
            lines.push(String::new());
        }
        lines
    }

    pub(super) fn overlay_transcript_items(&self) -> Vec<TranscriptItem> {
        let mut items = Vec::new();
        if self.show_session_header || self.session_title.is_some() {
            items.push(self.session_header_item());
        }
        for message in self.transcript_state.messages() {
            match &message.role {
                TranscriptRole::User => {
                    if !message.content.is_empty() {
                        items.push(TranscriptItem::User(message.content.clone()));
                    }
                }
                TranscriptRole::Assistant => {
                    if !message.reasoning.is_empty() {
                        items.push(TranscriptItem::Reasoning(message.reasoning.clone(), false));
                    }
                    for mut item in thinking_block_items(message) {
                        if let TranscriptItem::ContentBlock { collapsed, .. } = &mut item {
                            *collapsed = false;
                        }
                        items.push(item);
                    }
                    if !message.content.is_empty() || message.tool_calls.is_empty() {
                        items.push(TranscriptItem::Assistant(message.content.clone()));
                    }
                    for tool in &message.tool_calls {
                        items.push(TranscriptItem::Tool(ToolCard::from_tool_call(tool)));
                    }
                    for citation in &message.citations {
                        let mut item = citation_item(citation);
                        if let TranscriptItem::ContentBlock { collapsed, .. } = &mut item {
                            *collapsed = false;
                        }
                        items.push(item);
                    }
                    for block in &message.server_content_blocks {
                        let mut item = server_content_block_item(block);
                        if let TranscriptItem::ContentBlock { collapsed, .. } = &mut item {
                            *collapsed = false;
                        }
                        items.push(item);
                    }
                }
                TranscriptRole::Tool | TranscriptRole::Diff => {
                    let mut card = ToolCard::from_tool_call(&json!({
                        "id": message.tool_call_id.clone().unwrap_or_default(),
                        "name": message.role.as_str()
                    }))
                    .with_result(
                        message.content.clone(),
                        if message.tool_failed {
                            ToolStatus::Failed
                        } else {
                            ToolStatus::Succeeded
                        },
                    );
                    card.apply_result_metadata(&message.extra);
                    items.push(TranscriptItem::Tool(card));
                }
                TranscriptRole::ClientLocalNotice => {
                    items.push(TranscriptItem::Notice(message.content.clone()))
                }
                TranscriptRole::System => {
                    items.push(TranscriptItem::SystemFact(vec![
                        "System".to_string(),
                        visible_message_content(message, "(empty)"),
                    ]));
                }
                TranscriptRole::ContextFile
                | TranscriptRole::PlainText
                | TranscriptRole::CdInstruction => {
                    items.push(info_message_item(message, message.role.as_str()));
                }
                TranscriptRole::CompressionReport => {
                    items.push(info_message_item(message, "Compression report"));
                }
                TranscriptRole::Error => items.push(TranscriptItem::Notice(format!(
                    "Error: {}",
                    visible_message_content(message, "Unknown error")
                ))),
                TranscriptRole::Unknown { role, raw } => items.push(TranscriptItem::Info(vec![
                    format!("Unknown role: {role}"),
                    visible_message_content(message, "(empty)"),
                    format!("Raw: {}", collapsed_unknown_payload(raw)),
                ])),
                TranscriptRole::Plan | TranscriptRole::Goal | TranscriptRole::Event => {}
            }
        }
        items
    }
}

fn settings_surface_enabled_from_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn info_message_item(message: &TranscriptMessage, label: &str) -> TranscriptItem {
    TranscriptItem::Info(vec![
        label.to_string(),
        visible_message_content(message, "(empty)"),
    ])
}

pub(super) fn persist_theme_name_to_path(path: &Path, name: &str) -> Result<(), String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "Failed to read TUI config from {}: {error}",
                path.display()
            ));
        }
    };
    let content = apply_theme_name_to_config(&content, name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create TUI config directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, content)
        .map_err(|error| format!("Failed to save TUI theme to {}: {error}", path.display()))
}

pub(super) fn apply_theme_name_to_config(content: &str, name: &str) -> String {
    let name_line = format!("name = {}", toml_string(name));
    if content.trim().is_empty() {
        return format!("[theme]\n{name_line}\n");
    }

    let mut out = Vec::new();
    let mut in_theme = false;
    let mut found_theme = false;
    let mut wrote_name = false;

    for line in content.lines() {
        let trimmed = line.trim();
        let section = toml_section_name(trimmed);
        if section.is_some() && in_theme && !wrote_name {
            out.push(name_line.clone());
            wrote_name = true;
        }
        if let Some(section) = section {
            in_theme = section == "theme";
            found_theme |= in_theme;
            out.push(line.to_string());
            continue;
        }
        if in_theme && toml_key_name(trimmed) == Some("name") {
            let indent = line
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .collect::<String>();
            out.push(format!("{indent}{name_line}"));
            wrote_name = true;
        } else {
            out.push(line.to_string());
        }
    }

    if found_theme {
        if in_theme && !wrote_name {
            out.push(name_line);
        }
    } else {
        out.push(String::new());
        out.push("[theme]".to_string());
        out.push(name_line);
    }

    let mut content = out.join("\n");
    content.push('\n');
    content
}

fn toml_section_name(line: &str) -> Option<&str> {
    Some(line.strip_prefix('[')?.strip_suffix(']')?.trim())
}

fn toml_key_name(line: &str) -> Option<&str> {
    let (key, _) = line.split_once('=')?;
    Some(key.trim())
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_surface_gate_requires_a_truthy_value() {
        assert!(settings_surface_enabled_from_value(Some("1")));
        assert!(settings_surface_enabled_from_value(Some("true")));
        assert!(!settings_surface_enabled_from_value(None));
        assert!(!settings_surface_enabled_from_value(Some("0")));
    }

    #[test]
    fn mode_picker_uses_the_surfaces_gate() {
        let mut app = App::notice_only("test");
        app.open_mode_picker(serde_json::json!({"modes": [{"id": "agent", "title": "Agent"}]}));

        assert!(app.modal_picker().is_none());
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Notice(text)) if text == "/mode requires REFACT_TUI_SURFACES=1"
        ));
    }

    #[test]
    fn permissions_picker_uses_the_surfaces_gate() {
        let mut app = App::notice_only("test");
        app.open_permissions_picker();

        assert!(app.modal_picker().is_none());
        assert!(matches!(
            app.visible_transcript().last(),
            Some(TranscriptItem::Notice(text)) if text == "/permissions requires REFACT_TUI_SURFACES=1"
        ));
    }

    #[test]
    fn mode_picker_marks_current_mode_and_resolved_auto_approval() {
        let mut app = App::notice_only("test");
        app.mode = Some("agent".to_string());
        app.test_open_mode_picker(serde_json::json!({"modes": [
            {
                "id": "ask", "title": "Ask", "tools_count": 1,
                "thread_defaults": {"auto_approve_editing_tools": false},
                "ui": {"order": 1, "tags": ["chat"]}
            },
            {
                "id": "agent", "title": "Agent", "tools_count": 8,
                "thread_defaults": {"auto_approve_editing_tools": true},
                "ui": {"order": 2, "tags": ["tools"]}
            }
        ]}));

        let modes = app.modal_picker().unwrap().filtered_mode_items();
        let agent = modes.iter().find(|item| item.item.id == "agent").unwrap();

        assert!(agent.is_current);
        assert_eq!(agent.auto_approval_badge(), Some("! edits auto-approved"));
    }

    #[test]
    fn view_overlay_uses_rendered_lines_as_raw_fallback() {
        let mut app = App::notice_only("test");
        app.open_view_overlay(ViewOverlay {
            title: "Surface".to_string(),
            rendered_lines: vec!["rendered".to_string()],
            raw_lines: Vec::new(),
            surface: None,
        });

        let overlay = app.transcript_overlay().expect("overlay opened");
        assert_eq!(overlay.title(), "Surface");
        assert_eq!(overlay.lines(), ["rendered"]);
    }

    #[test]
    fn theme_picker_cancel_restores_snapshot() {
        let mut app = App::notice_only("test");
        let original = app.theme().name().to_string();

        app.open_theme_picker();
        app.preview_theme_name("light");
        app.cancel_modal_picker();

        assert_eq!(app.theme().name(), original);
        assert!(app.modal_picker().is_none());
    }
}
