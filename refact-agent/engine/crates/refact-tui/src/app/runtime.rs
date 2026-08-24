use std::env;
use std::process::Command;

use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::client::{ChatSeqDecision, ChatSeqTracker, ClientError, DaemonClient};
use crate::terminal::{FrameRequester, TerminalTitleConfig, TARGET_FRAME_INTERVAL};

use super::*;

async fn show_startup_notice(message: String) -> Result<(), TuiError> {
    let mut app = App::notice_only(message);
    let config_content = load_tui_config_content();
    if let Some(config_content) = config_content.as_deref() {
        app.apply_tui_config_content(config_content);
    }
    let title_config = TerminalTitleConfig::from_env(config_content.as_deref());
    let mut terminal = TerminalSession::start_with_title_config(title_config)?;
    apply_terminal_mode(&mut app, &terminal);
    render_frame(&mut terminal, &mut app)?;
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
                render_frame(&mut terminal, &mut app)?;
            }
            Ok(Some(Ok(_))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => {}
        }
    }
    Ok(())
}

fn apply_caps_loaded(app: &mut App, project_id: &str, result: Result<Value, String>) {
    match result {
        _ if app.current_project_id() != Some(project_id) => {}
        Ok(caps) => app.apply_caps(&caps),
        Err(error) => {
            if worker_waking_message(&error) {
                app.subscription_status = SubscriptionStatus::Waking;
            }
            app.retry_hint = retry_hint_from_message(&error);
        }
    }
}

fn apply_models_loaded(app: &mut App, project_id: &str, result: Result<Value, String>) {
    match result {
        _ if app.current_project_id() != Some(project_id) => {}
        Ok(caps) => app.open_model_picker(caps),
        Err(error) => {
            app.retry_hint = retry_hint_from_message(&error);
            app.add_notice(format!("Failed to load models: {error}"));
        }
    }
}

fn apply_modes_loaded(app: &mut App, project_id: &str, result: Result<Value, String>) {
    match result {
        _ if app.current_project_id() != Some(project_id) => {}
        Ok(modes) => app.open_mode_picker(modes),
        Err(error) => {
            app.retry_hint = retry_hint_from_message(&error);
            app.add_notice(format!("Failed to load modes: {error}"));
        }
    }
}

#[derive(Debug)]
pub(super) enum RuntimeEvent {
    Input(Event),
    Frame,
    Chat {
        generation: u64,
        event: ChatEvent,
    },
    ChatResubscribe {
        generation: u64,
        message: String,
    },
    ChatDisconnected {
        generation: u64,
        message: String,
        unreachable: bool,
        auth_stale: bool,
    },
    InputError(String),
    DaemonEvent {
        generation: u64,
        event: DaemonEventRecord,
    },
    DaemonEventsDisconnected {
        generation: u64,
        message: String,
        retrying: bool,
    },
    ProjectsLoaded(Result<Vec<ProjectEntry>, String>),
    ProjectOpened(Result<OpenProjectResponse, String>),
    CapsLoaded {
        project_id: String,
        result: Result<Value, String>,
    },
    ModelsLoaded {
        project_id: String,
        result: Result<Value, String>,
    },
    ModesLoaded {
        project_id: String,
        result: Result<Value, String>,
    },
    FileMentionsLoaded(Result<Vec<String>, String>),
    McpViewLoaded(Result<McpViewData, String>),
    SkillsViewLoaded(Result<SlashCommandsListResponse, String>),
    MemoriesViewLoaded(Result<KnowledgeGraphResponse, String>),
    HooksViewLoaded(Result<HooksResponse, String>),
    ProviderLogoutChoicesLoaded(Result<ProviderListResponse, String>),
    ProviderLogoutFinished {
        provider: String,
        result: Result<ProviderOAuthLogoutResponse, String>,
    },
    CompetitorImportViewLoaded(Result<CompetitorImportInfoResponse, String>),
    CompetitorImportSourcesLoaded(Result<CompetitorImportInfoResponse, String>),
    CompetitorImportFinished(Result<CompetitorImportRunResponse, String>),
    SessionsLoaded {
        project_id: String,
        open_picker: bool,
        result: Result<Vec<TrajectoryMeta>, String>,
    },
    DaemonStatusLoaded(Result<(DaemonStatus, String), String>),
    WorkersLoaded(Result<Vec<WorkerInfo>, String>),
    HistorySaved(Result<(), String>),
    CommandFinished {
        generation: u64,
        context: CommandContextTag,
        result: Result<(), String>,
    },
    DiffLoaded(Result<String, String>),
}

#[derive(Debug, Default)]
pub(super) struct SubscriptionManager {
    pub(super) generation: u64,
    pub(super) current: Option<SubscriptionTask>,
}

#[derive(Debug)]
pub(super) struct SubscriptionTask {
    pub(super) generation: u64,
    pub(super) project_id: String,
    pub(super) chat_id: String,
    pub(super) reconnect_attempts: u32,
    pub(super) join: JoinHandle<()>,
}

#[derive(Debug, Default)]
pub(super) struct DaemonEventSubscription {
    pub(super) generation: u64,
    pub(super) current: Option<JoinHandle<()>>,
}

impl DaemonEventSubscription {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn start(&mut self, client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>) {
        self.abort_current();
        self.generation = self.generation.wrapping_add(1).max(1);
        self.current = Some(spawn_daemon_events_task(client, tx, self.generation));
    }

    pub(super) fn restart(&mut self, client: DaemonClient, tx: mpsc::Sender<RuntimeEvent>) {
        self.start(client, tx);
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub(super) fn apply_event(
        &self,
        app: &mut App,
        generation: u64,
        event: DaemonEventRecord,
    ) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        app.push_daemon_event(event);
        true
    }

    pub(super) fn abort_current(&mut self) {
        if let Some(current) = self.current.take() {
            current.abort();
        }
    }
}

impl Drop for DaemonEventSubscription {
    fn drop(&mut self) {
        self.abort_current();
    }
}

impl SubscriptionManager {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn subscribe(
        &mut self,
        client: DaemonClient,
        project_id: String,
        chat_id: String,
        tx: mpsc::Sender<RuntimeEvent>,
    ) {
        self.start(client, project_id, chat_id, tx, Duration::ZERO, 0);
    }

    pub(super) fn reconnect_current(
        &mut self,
        client: DaemonClient,
        tx: mpsc::Sender<RuntimeEvent>,
    ) -> Result<(), String> {
        let Some(current) = &self.current else {
            return Ok(());
        };
        if current.reconnect_attempts >= CHAT_MAX_RECONNECT_ATTEMPTS {
            return Err("chat SSE reconnect limit reached".to_string());
        }
        let generation = current.generation;
        let project_id = current.project_id.clone();
        let chat_id = current.chat_id.clone();
        let reconnect_attempts = current.reconnect_attempts.saturating_add(1);
        let initial_delay = reconnect_backoff(
            CHAT_RECONNECT_INITIAL_BACKOFF,
            CHAT_RECONNECT_MAX_BACKOFF,
            reconnect_attempts,
            generation,
        );
        self.start(
            client,
            project_id,
            chat_id,
            tx,
            initial_delay,
            reconnect_attempts,
        );
        Ok(())
    }

    pub(super) fn abort_current(&mut self) {
        if let Some(current) = self.current.take() {
            current.join.abort();
        }
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.generation == generation)
    }

    pub(super) fn current_generation(&self) -> Option<u64> {
        self.current.as_ref().map(|current| current.generation)
    }

    pub(super) fn command_generation(&self) -> u64 {
        self.current_generation().unwrap_or(self.generation)
    }

    pub(super) fn is_command_current(&self, generation: u64) -> bool {
        match &self.current {
            Some(current) => current.generation == generation,
            None => self.generation == generation,
        }
    }

    pub(super) fn mark_connected(&mut self, generation: u64) {
        if let Some(current) = self.current.as_mut() {
            if current.generation == generation {
                current.reconnect_attempts = 0;
            }
        }
    }

    pub(super) fn apply_chat_event(
        &mut self,
        app: &mut App,
        generation: u64,
        event: ChatEvent,
    ) -> Option<AppAction> {
        if !self.is_current(generation) {
            return None;
        }
        self.mark_connected(generation);
        Some(app.handle_chat_event(event))
    }

    pub(super) fn apply_command_finished(
        &self,
        app: &mut App,
        generation: u64,
        context: CommandContextTag,
        result: Result<(), String>,
    ) -> Option<AppAction> {
        if generation != 0 && !self.is_command_current(generation) {
            return None;
        }
        Some(app.handle_command_finished(context, result))
    }

    fn start(
        &mut self,
        client: DaemonClient,
        project_id: String,
        chat_id: String,
        tx: mpsc::Sender<RuntimeEvent>,
        initial_delay: Duration,
        reconnect_attempts: u32,
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
            reconnect_attempts,
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
    let endpoint = match crate::client::resolve_daemon_endpoint_with_auth(
        options
            .daemon_url
            .or_else(|| std::env::var("REFACT_DAEMON_URL").ok()),
        std::env::var("REFACT_DAEMON_TOKEN").ok(),
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
    let config_content = load_tui_config_content();
    if let Some(config_content) = config_content.as_deref() {
        app.apply_tui_config_content(config_content);
    }
    let title_config = TerminalTitleConfig::from_env(config_content.as_deref());
    let mut terminal = TerminalSession::start_with_title_config(title_config)?;
    apply_terminal_mode(&mut app, &terminal);
    let (tx, mut rx) = mpsc::channel::<RuntimeEvent>(256);
    let mut subscriptions = SubscriptionManager::new();
    let mut daemon_events = DaemonEventSubscription::new();
    let mut input_task = spawn_input_task(tx.clone());
    let (frame_requester, frame_rx) = FrameRequester::new();
    let frame_task = spawn_frame_task(frame_rx, tx.clone());
    daemon_events.start(client.clone(), tx.clone());
    spawn_worker_refresh_task(client.clone(), tx.clone());
    if let Some(project_id) = app.current_project_id().map(str::to_string) {
        app.begin_subscription_attempt();
        subscriptions.subscribe(
            client.clone(),
            project_id.clone(),
            app.chat_id().to_string(),
            tx.clone(),
        );
        load_caps(client.clone(), tx.clone(), project_id.clone());
        load_recent_sessions(client.clone(), tx.clone(), project_id, false);
    }

    loop {
        render_frame(&mut terminal, &mut app)?;
        if app.should_quit() {
            break;
        }
        schedule_next_frame(&app, &frame_requester);
        let Some(event) = rx.recv().await else {
            break;
        };
        match event {
            RuntimeEvent::Input(Event::Key(key)) => {
                let action = app.handle_key(key);
                match action {
                    AppAction::OpenExternalEditor { draft } => {
                        input_task.abort();
                        let result = edit_composer_in_external_editor(&mut terminal, draft).await;
                        match result {
                            Ok(content) => app.composer.set_text(content),
                            Err(message) => app.add_notice(message),
                        }
                        input_task = spawn_input_task(tx.clone());
                    }
                    AppAction::CopyToClipboard { text, source } => {
                        let result = terminal.write_clipboard(&text);
                        app.record_clipboard_result(source, result);
                    }
                    action => {
                        run_action(
                            &mut app,
                            action,
                            &client,
                            &tx,
                            &mut subscriptions,
                            &mut daemon_events,
                        )
                        .await
                    }
                }
            }
            RuntimeEvent::Input(Event::Paste(text)) => app.handle_paste(&text),
            RuntimeEvent::Input(Event::FocusGained) => app.set_terminal_focus(true),
            RuntimeEvent::Input(Event::FocusLost) => app.set_terminal_focus(false),
            RuntimeEvent::Input(Event::Resize(width, _)) => {
                if !app.note_terminal_resize_width(width) {
                    app.note_terminal_height_resize();
                }
            }
            RuntimeEvent::Frame => {
                app.run_stream_commit_tick();
                app.flush_pending_paste();
            }
            RuntimeEvent::Input(_) => {}
            RuntimeEvent::Chat { generation, event } => {
                if let Some(action) = subscriptions.apply_chat_event(&mut app, generation, event) {
                    run_action(
                        &mut app,
                        action,
                        &client,
                        &tx,
                        &mut subscriptions,
                        &mut daemon_events,
                    )
                    .await;
                }
            }
            RuntimeEvent::ChatResubscribe {
                generation,
                message,
            } => {
                if subscriptions.is_current(generation) {
                    app.record_chat_resubscribe(&message);
                    app.add_notice(format!("SSE resync: {message}"));
                    if let Err(error) = subscriptions.reconnect_current(client.clone(), tx.clone())
                    {
                        app.record_subscription_exhausted();
                        app.add_notice(error);
                    }
                }
            }
            RuntimeEvent::ChatDisconnected {
                generation,
                message,
                unreachable,
                auth_stale,
            } => {
                if !subscriptions.is_current(generation) {
                    continue;
                }
                app.record_chat_disconnected(&message, unreachable, auth_stale);
                if auth_stale {
                    app.add_notice(format!(
                        "SSE disconnected: {message}; daemon auth token is stale"
                    ));
                    continue;
                }
                match subscriptions.reconnect_current(client.clone(), tx.clone()) {
                    Ok(()) => app.add_notice(format!("SSE disconnected: {message}; reconnecting…")),
                    Err(error) => {
                        app.record_subscription_exhausted();
                        app.add_notice(format!("SSE disconnected: {message}; {error}"));
                    }
                }
            }
            RuntimeEvent::InputError(message) => app.add_notice(format!("Input error: {message}")),
            RuntimeEvent::DaemonEvent { generation, event } => {
                daemon_events.apply_event(&mut app, generation, event);
            }
            RuntimeEvent::DaemonEventsDisconnected {
                generation,
                message,
                retrying,
            } => {
                if !daemon_events.is_current(generation) {
                    continue;
                }
                app.handle_daemon_events_disconnected(message, retrying);
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
                    daemon_events.restart(client.clone(), tx.clone());
                    load_caps(client.clone(), tx.clone(), project_id.clone());
                    load_recent_sessions(client.clone(), tx.clone(), project_id, false);
                }
            }
            RuntimeEvent::ProjectOpened(Err(error)) => {
                app.add_notice(format!("Failed to open project: {error}"))
            }
            RuntimeEvent::CapsLoaded { project_id, result } => {
                apply_caps_loaded(&mut app, &project_id, result)
            }
            RuntimeEvent::ModelsLoaded { project_id, result } => {
                apply_models_loaded(&mut app, &project_id, result)
            }
            RuntimeEvent::ModesLoaded { project_id, result } => {
                apply_modes_loaded(&mut app, &project_id, result)
            }
            RuntimeEvent::FileMentionsLoaded(Ok(completions)) => {
                app.open_file_mention_picker(file_mention_items_from_completions(completions))
            }
            RuntimeEvent::FileMentionsLoaded(Err(error)) => {
                app.add_notice(format!("Failed to load file mentions: {error}"));
                app.open_file_mention_picker(Vec::new());
            }
            RuntimeEvent::McpViewLoaded(result) => app.handle_mcp_view_loaded(result),
            RuntimeEvent::SkillsViewLoaded(result) => app.handle_skills_view_loaded(result),
            RuntimeEvent::MemoriesViewLoaded(result) => app.handle_memories_view_loaded(result),
            RuntimeEvent::HooksViewLoaded(result) => app.handle_hooks_view_loaded(result),
            RuntimeEvent::ProviderLogoutChoicesLoaded(result) => {
                app.handle_provider_logout_choices_loaded(result)
            }
            RuntimeEvent::ProviderLogoutFinished { provider, result } => {
                app.handle_provider_logout_finished(provider, result)
            }
            RuntimeEvent::CompetitorImportSourcesLoaded(result) => {
                app.handle_competitor_import_sources_loaded(result)
            }
            RuntimeEvent::CompetitorImportViewLoaded(result) => {
                app.handle_competitor_import_view_loaded(result)
            }
            RuntimeEvent::CompetitorImportFinished(result) => {
                app.handle_competitor_import_finished(result)
            }
            RuntimeEvent::SessionsLoaded {
                project_id,
                open_picker,
                result,
            } => match result {
                _ if app.current_project_id() != Some(project_id.as_str()) => {}
                Ok(trajectories) if open_picker => {
                    app.open_session_picker_from_trajectories(trajectories)
                }
                Ok(trajectories) => app.refresh_recent_sessions_from_trajectories(trajectories),
                Err(error) if open_picker => {
                    app.retry_hint = retry_hint_from_message(&error);
                    app.add_notice(format!("Failed to load recent chats: {error}"));
                    app.open_session_picker(Vec::new());
                }
                Err(error) => app.retry_hint = retry_hint_from_message(&error),
            },
            RuntimeEvent::DaemonStatusLoaded(Ok((status, base_url))) => {
                app.apply_daemon_status(status, base_url)
            }
            RuntimeEvent::DaemonStatusLoaded(Err(error)) => app.record_daemon_status_error(&error),
            RuntimeEvent::WorkersLoaded(Ok(workers)) => app.set_workers(workers),
            RuntimeEvent::WorkersLoaded(Err(error)) => {
                if app.events_pane.open {
                    app.add_notice(format!("Failed to refresh workers: {error}"));
                }
            }
            RuntimeEvent::HistorySaved(result) => app.handle_history_save_result(result),
            RuntimeEvent::CommandFinished {
                generation,
                context,
                result,
            } => {
                if let Some(action) =
                    subscriptions.apply_command_finished(&mut app, generation, context, result)
                {
                    run_action(
                        &mut app,
                        action,
                        &client,
                        &tx,
                        &mut subscriptions,
                        &mut daemon_events,
                    )
                    .await;
                }
            }
            RuntimeEvent::DiffLoaded(Ok(diff)) => app.show_diff_result(diff),
            RuntimeEvent::DiffLoaded(Err(error)) => {
                app.add_notice(format!("Failed to load git diff: {error}"))
            }
        }
        if let Some(request) = app.take_pending_history_save() {
            spawn_history_save_task(request, tx.clone());
        }
        for bytes in app.take_pending_notifications() {
            terminal.write_notification(&bytes)?;
        }
    }
    frame_task.abort();
    Ok(())
}

fn apply_terminal_mode(app: &mut App, terminal: &TerminalSession) {
    app.set_native_scrollback(terminal.mode() == crate::terminal::TerminalMode::Inline);
}

fn spawn_history_save_task(request: HistorySaveRequest, tx: mpsc::Sender<RuntimeEvent>) {
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || execute_history_save_request(request))
            .await
            .unwrap_or_else(|error| Err(format!("Failed to save composer history: {error}")));
        let _ = tx.send(RuntimeEvent::HistorySaved(result)).await;
    });
}

pub(super) fn execute_history_save_request(request: HistorySaveRequest) -> Result<(), String> {
    save_history(&request.path, &request.entries).map_err(|error| {
        format!(
            "Failed to save composer history to {}: {error}",
            request.path.display()
        )
    })
}

pub(super) async fn run_action(
    app: &mut App,
    action: AppAction,
    client: &DaemonClient,
    tx: &mpsc::Sender<RuntimeEvent>,
    subscriptions: &mut SubscriptionManager,
    daemon_events: &mut DaemonEventSubscription,
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
                    let _ = tx
                        .send(RuntimeEvent::ModelsLoaded { project_id, result })
                        .await;
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
                    let _ = tx
                        .send(RuntimeEvent::ModesLoaded { project_id, result })
                        .await;
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
        AppAction::LoadReadOnlyView { view } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    match view {
                        ReadOnlyView::Mcp => {
                            let result = client
                                .mcp_view_data(&project_id)
                                .await
                                .map_err(|error| error.to_string());
                            let _ = tx.send(RuntimeEvent::McpViewLoaded(result)).await;
                        }
                        ReadOnlyView::Skills => {
                            let result = client
                                .slash_commands(&project_id)
                                .await
                                .map_err(|error| error.to_string());
                            let _ = tx.send(RuntimeEvent::SkillsViewLoaded(result)).await;
                        }
                        ReadOnlyView::Memories => {
                            let result = client
                                .knowledge_graph(&project_id)
                                .await
                                .map_err(|error| error.to_string());
                            let _ = tx.send(RuntimeEvent::MemoriesViewLoaded(result)).await;
                        }
                        ReadOnlyView::Hooks => {
                            let result = client
                                .hooks(&project_id)
                                .await
                                .map_err(|error| error.to_string());
                            let _ = tx.send(RuntimeEvent::HooksViewLoaded(result)).await;
                        }
                        ReadOnlyView::Import => {
                            let result = client
                                .competitor_import_info(&project_id)
                                .await
                                .map_err(|error| error.to_string());
                            let _ = tx
                                .send(RuntimeEvent::CompetitorImportViewLoaded(result))
                                .await;
                        }
                    }
                });
            } else {
                app.add_notice(format!("No active project for /{}", view.command_name()));
            }
        }
        AppAction::LoadProviderLogoutChoices => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .providers(&project_id)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::ProviderLogoutChoicesLoaded(result))
                        .await;
                });
            } else {
                app.add_notice("No active project for /logout");
            }
        }
        AppAction::ProviderOAuthLogout { provider } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .provider_oauth_logout(&project_id, &provider)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::ProviderLogoutFinished { provider, result })
                        .await;
                });
            } else {
                app.add_notice("No active project for /logout");
            }
        }
        AppAction::LoadCompetitorImportSources => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .competitor_import_info(&project_id)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CompetitorImportSourcesLoaded(result))
                        .await;
                });
            } else {
                app.add_notice("No active project for /import");
            }
        }
        AppAction::RunCompetitorImport { source, scope } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .competitor_import_run(&project_id, source.as_deref(), &scope)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CompetitorImportFinished(result))
                        .await;
                });
            } else {
                app.add_notice("No active project for /import");
            }
        }
        AppAction::LoadSessions => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                load_recent_sessions(client.clone(), tx.clone(), project_id, true);
            }
        }
        AppAction::RefreshRecentSessions => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                load_recent_sessions(client.clone(), tx.clone(), project_id, false);
            }
        }
        AppAction::RefreshWorkers => refresh_workers(client.clone(), tx.clone()),
        AppAction::LoadDaemonStatus => {
            let client = client.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let base_url = client.base_url().to_string();
                let result = client
                    .status()
                    .await
                    .map(|status| (status, base_url))
                    .map_err(|error| error.to_string());
                let _ = tx.send(RuntimeEvent::DaemonStatusLoaded(result)).await;
            });
        }
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
                daemon_events.restart(client.clone(), tx.clone());
            }
        }
        AppAction::SendMessage {
            prompt,
            params,
            correlation,
        } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let generation = subscriptions.command_generation();
                let client = client.clone();
                let tx = tx.clone();
                let context = CommandContextTag::SendMessage {
                    prompt: prompt.clone(),
                    params: params.clone(),
                    correlation: correlation.clone(),
                };
                tokio::spawn(async move {
                    let result = async {
                        if params.as_object().is_some_and(|object| !object.is_empty()) {
                            client
                                .send_set_params(&project_id, &chat_id, params)
                                .await?;
                        }
                        client
                            .send_user_message_with_ids(
                                &project_id,
                                &chat_id,
                                &correlation.client_request_id,
                                &correlation.client_message_id,
                                &prompt,
                            )
                            .await
                    }
                    .await
                    .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
                });
            }
        }
        AppAction::RetryFromIndex { index, content } => {
            let context = CommandContextTag::RetryFromIndex {
                rollback: app.pending_backtrack_rollback.clone(),
            };
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let generation = subscriptions.command_generation();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .send_retry_from_index(&project_id, &chat_id, index, content)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
                });
            } else {
                let _ = app.handle_command_finished(
                    context,
                    Err("no active project for retry".to_string()),
                );
            }
        }
        AppAction::SetParams { patch } => {
            let context = app.set_params_context(&patch);
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let generation = subscriptions.command_generation();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .send_set_params(&project_id, &chat_id, patch)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
                });
            }
        }
        AppAction::RenameChat { title } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let generation = subscriptions.command_generation();
                let patch = json!({"title": title, "is_title_generated": false});
                let client = client.clone();
                let tx = tx.clone();
                let context = CommandContextTag::Rename { title };
                tokio::spawn(async move {
                    let result = client
                        .send_set_params(&project_id, &chat_id, patch)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
                });
            }
        }
        AppAction::ForkChat {
            target_chat_id,
            source_chat_id,
            up_to_message_id,
            title,
        } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let generation = subscriptions.command_generation();
                let client = client.clone();
                let tx = tx.clone();
                let context = CommandContextTag::Fork {
                    target_chat_id: target_chat_id.clone(),
                    title,
                };
                tokio::spawn(async move {
                    let result = client
                        .send_branch_from_chat(
                            &project_id,
                            &target_chat_id,
                            &source_chat_id,
                            &up_to_message_id,
                        )
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
                });
            }
        }
        AppAction::ArchiveChat {
            chat_id,
            new_chat_id,
        } => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let generation = subscriptions.command_generation();
                let client = client.clone();
                let tx = tx.clone();
                let context = CommandContextTag::Archive {
                    chat_id: new_chat_id,
                };
                tokio::spawn(async move {
                    let result = client
                        .delete_trajectory(&project_id, &chat_id)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
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
        AppAction::CopyToClipboard { .. } => {}
        AppAction::OpenExternalEditor { .. } => {}
        AppAction::SendToolDecisions { decisions, patch } => {
            let context = CommandContextTag::ToolDecisions {
                rollback: app.pending_tool_decision_rollback.clone(),
            };
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let generation = subscriptions.command_generation();
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
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context,
                            result,
                        })
                        .await;
                });
            } else {
                let _ = app.handle_command_finished(
                    context,
                    Err("no active project for tool decision".to_string()),
                );
            }
        }
        AppAction::Abort => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let generation = subscriptions.command_generation();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = match tokio::time::timeout(
                        ABORT_BEFORE_QUIT_TIMEOUT,
                        client.send_abort(&project_id, &chat_id),
                    )
                    .await
                    {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(error)) => Err(error.to_string()),
                        Err(_) => Err("abort request timed out".to_string()),
                    };
                    let _ = tx
                        .send(RuntimeEvent::CommandFinished {
                            generation,
                            context: CommandContextTag::Abort,
                            result,
                        })
                        .await;
                });
            } else {
                let _ = app.handle_command_finished(
                    CommandContextTag::Abort,
                    Err("no active project for abort".to_string()),
                );
            }
        }
        AppAction::Quit { abort_active } => {
            if abort_active {
                match send_abort_for_current_chat(app, client).await {
                    Ok(()) => app.handle_quit_abort_success(),
                    Err(error) => app.handle_quit_abort_failure(error),
                }
            }
        }
    }
}

impl App {
    pub(super) fn handle_send_message_failure(
        &mut self,
        prompt: String,
        params: Value,
        correlation: ClientMessageCorrelation,
        error: String,
    ) -> AppAction {
        let Some(in_flight) = self.in_flight_send.as_ref() else {
            return AppAction::None;
        };
        if in_flight.correlation.client_request_id != correlation.client_request_id {
            return AppAction::None;
        }
        let accepted = in_flight.accepted;
        self.in_flight_send = None;
        if accepted {
            return AppAction::None;
        }
        self.retry_hint = retry_hint_from_message(&error);
        self.rollback_failed_send_message(prompt, params, correlation, &error)
    }

    fn rollback_failed_send_message(
        &mut self,
        prompt: String,
        params: Value,
        correlation: ClientMessageCorrelation,
        error: &str,
    ) -> AppAction {
        self.set_session_state(SessionState::Idle);
        self.clear_stream_controllers();
        self.rollback_failed_send_transcript(&correlation.client_message_id);
        self.restore_failed_prompt(prompt, params, correlation);
        self.add_notice(format!("Command failed: {error}"));
        AppAction::None
    }

    fn rollback_failed_send_transcript(&mut self, client_message_id: &str) {
        let messages = self.transcript_state.messages();
        let truncate_from = if messages.len() >= 2
            && messages[messages.len() - 2].role == TranscriptRole::User
            && messages[messages.len() - 2].message_id.is_none()
            && messages[messages.len() - 2].client_message_id() == Some(client_message_id)
            && is_empty_live_assistant(&messages[messages.len() - 1])
        {
            Some(messages.len() - 2)
        } else if messages.last().is_some_and(is_empty_live_assistant) {
            Some(messages.len() - 1)
        } else {
            None
        };
        if let Some(index) = truncate_from {
            self.transcript_state.truncate_messages(index);
            self.rebuild_render_transcript_from_state();
        }
    }

    fn restore_failed_prompt(
        &mut self,
        prompt: String,
        params: Value,
        correlation: ClientMessageCorrelation,
    ) {
        let draft = self.composer.text().to_string();
        if !draft.trim().is_empty() && draft != prompt {
            let draft_params = self.take_pending_params();
            self.enqueue_input(draft, draft_params);
        }
        self.pending_send_retry = Some(PendingSendRetry {
            prompt: prompt.clone(),
            params,
            correlation,
        });
        self.composer.set_text(prompt);
    }

    pub(super) fn handle_send_ack(&mut self, client_request_id: &str, accepted: bool) {
        if !accepted {
            return;
        }
        if let Some(in_flight) = self
            .in_flight_send
            .as_mut()
            .filter(|in_flight| in_flight.correlation.client_request_id == client_request_id)
        {
            in_flight.accepted = true;
        }
    }

    pub(super) fn clear_in_flight_send(&mut self, client_request_id: &str) -> bool {
        if self
            .in_flight_send
            .as_ref()
            .is_some_and(|in_flight| in_flight.correlation.client_request_id == client_request_id)
        {
            self.in_flight_send = None;
            true
        } else {
            false
        }
    }

    fn handle_quit_abort_success(&mut self) {
        self.abort_in_flight = false;
        self.set_session_state(SessionState::Idle);
        self.clear_approvals();
        self.clear_active_ask_questions();
        self.should_quit = true;
    }

    fn handle_quit_abort_failure(&mut self, error: String) {
        self.abort_in_flight = false;
        self.should_quit = false;
        self.add_notice(format!("Abort before quit failed: {error}"));
    }
}

pub(super) async fn load_git_diff(root: PathBuf) -> Result<String, String> {
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

async fn edit_composer_in_external_editor(
    terminal: &mut TerminalSession,
    draft: String,
) -> Result<String, String> {
    let editor = external_editor_command()?;
    terminal.suspend();
    let edit_result = edit_text_with_editor_command(&editor, draft);
    let resume_result = terminal.resume();
    resume_result.map_err(|error| format!("Failed to restore terminal after editor: {error}"))?;
    edit_result
}

pub(super) fn edit_text_with_editor_command(editor: &str, draft: String) -> Result<String, String> {
    let path = temp_editor_path();
    fs::write(&path, draft)
        .map_err(|error| format!("Failed to write editor temp file: {error}"))?;
    let edit_result = run_editor_command(&editor, &path);
    let read_result = edit_result.and_then(|_| {
        fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read editor temp file: {error}"))
    });
    let _ = fs::remove_file(&path);
    read_result
}

fn external_editor_command() -> Result<String, String> {
    let editor = env::var("EDITOR").ok();
    let visual = env::var("VISUAL").ok();
    external_editor_command_from_values(editor.as_deref(), visual.as_deref(), command_in_path)
}

pub(super) fn external_editor_command_from_values(
    editor: Option<&str>,
    visual: Option<&str>,
    command_exists: impl Fn(&str) -> bool,
) -> Result<String, String> {
    for value in [editor, visual].into_iter().flatten() {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_string());
        }
    }
    if command_exists("vi") {
        Ok("vi".to_string())
    } else {
        Err("No $EDITOR/$VISUAL set and fallback vi was not found".to_string())
    }
}

pub(super) fn parse_editor_command(editor: &str) -> Result<EditorCommand, String> {
    let parts = shell_words::split(editor)
        .map_err(|error| format!("Failed to parse editor command `{editor}`: {error}"))?;
    let Some((program, args)) = parts.split_first() else {
        return Err("Editor command is empty".to_string());
    };
    Ok(EditorCommand {
        program: program.clone(),
        args: args.to_vec(),
    })
}

fn temp_editor_path() -> PathBuf {
    env::temp_dir().join(format!(
        "refact-tui-{}-{}.md",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

fn run_editor_command(editor: &str, path: &Path) -> Result<(), String> {
    let command = parse_editor_command(editor)?;
    let status = Command::new(&command.program)
        .args(&command.args)
        .arg(path)
        .status()
        .map_err(|error| format!("Failed to launch editor `{editor}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Editor `{editor}` exited with {status}"))
    }
}

fn command_in_path(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(command).is_file();
    }
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(command).is_file()))
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

fn spawn_input_task(tx: mpsc::Sender<RuntimeEvent>) -> JoinHandle<()> {
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
    })
}

fn spawn_frame_task(
    mut frame_rx: mpsc::Receiver<()>,
    tx: mpsc::Sender<RuntimeEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while frame_rx.recv().await.is_some() {
            if tx.send(RuntimeEvent::Frame).await.is_err() {
                break;
            }
        }
    })
}

fn schedule_next_frame(app: &App, frame_requester: &FrameRequester) {
    let mut delay = None;
    if app.stream_has_committable_lines() {
        delay = Some(TARGET_FRAME_INTERVAL);
    }
    if app.session_state().shows_working_indicator() {
        delay = Some(min_frame_delay(delay, WORKING_ANIMATION_INTERVAL));
    }
    if let Some(paste_delay) = app.pending_paste_delay() {
        delay = Some(min_frame_delay(delay, paste_delay));
    }
    if let Some(resize_delay) = app.resize_reflow_delay() {
        delay = Some(min_frame_delay(delay, resize_delay));
    }
    if let Some(delay) = delay {
        if delay.is_zero() {
            frame_requester.schedule_frame();
        } else {
            frame_requester.schedule_frame_in(delay);
        }
    }
}

fn min_frame_delay(current: Option<Duration>, candidate: Duration) -> Duration {
    current.map_or(candidate, |current| current.min(candidate))
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
        let _ = tx
            .send(RuntimeEvent::CapsLoaded { project_id, result })
            .await;
    });
}

fn load_recent_sessions(
    client: DaemonClient,
    tx: mpsc::Sender<RuntimeEvent>,
    project_id: String,
    open_picker: bool,
) {
    tokio::spawn(async move {
        let result = client
            .list_all_trajectories(&project_id)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(RuntimeEvent::SessionsLoaded {
                project_id,
                open_picker,
                result,
            })
            .await;
    });
}

fn reconnect_backoff(initial: Duration, max: Duration, attempt: u32, generation: u64) -> Duration {
    let shift = attempt.saturating_sub(1).min(8);
    let multiplier = 1u32 << shift;
    let base = initial.saturating_mul(multiplier).min(max);
    let jitter_seed = generation.wrapping_add(u64::from(attempt).wrapping_mul(17));
    let jitter_ms = jitter_seed % 97;
    base.saturating_add(Duration::from_millis(jitter_ms))
        .min(max)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn project(project_id: &str) -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: project_id.to_string(),
            slug: project_id.to_string(),
            root: PathBuf::from(format!("/tmp/{project_id}")),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    #[test]
    fn stale_caps_and_model_picker_responses_are_discarded_after_project_switch() {
        let mut app = App::new(project("p1"));
        app.set_project(project("p2"));

        apply_caps_loaded(
            &mut app,
            "p1",
            Ok(json!({"defaults": {"chat_default_model": "stale-model"}})),
        );
        apply_models_loaded(
            &mut app,
            "p1",
            Ok(json!({"chat_models": {"stale-model": {"name": "Stale Model"}}})),
        );

        assert_eq!(app.model(), None);
        assert!(app.modal_picker().is_none());
    }
}

fn spawn_daemon_events_task(
    client: DaemonClient,
    tx: mpsc::Sender<RuntimeEvent>,
    generation: u64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut reconnect_attempts = 0u32;
        loop {
            match client.subscribe_daemon_events().await {
                Ok(mut stream) => {
                    let mut disconnected = false;
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(event) => {
                                reconnect_attempts = 0;
                                if tx
                                    .send(RuntimeEvent::DaemonEvent { generation, event })
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(error) => {
                                disconnected = true;
                                reconnect_attempts = reconnect_attempts.saturating_add(1);
                                let retrying =
                                    reconnect_attempts <= DAEMON_EVENTS_MAX_RECONNECT_ATTEMPTS;
                                if tx
                                    .send(RuntimeEvent::DaemonEventsDisconnected {
                                        generation,
                                        message: error.to_string(),
                                        retrying,
                                    })
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                if !retrying {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                    if disconnected {
                        continue;
                    }
                    reconnect_attempts = reconnect_attempts.saturating_add(1);
                    let retrying = reconnect_attempts <= DAEMON_EVENTS_MAX_RECONNECT_ATTEMPTS;
                    if tx
                        .send(RuntimeEvent::DaemonEventsDisconnected {
                            generation,
                            message: "stream ended".to_string(),
                            retrying,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if !retrying {
                        return;
                    }
                }
                Err(error) => {
                    reconnect_attempts = reconnect_attempts.saturating_add(1);
                    let retrying = reconnect_attempts <= DAEMON_EVENTS_MAX_RECONNECT_ATTEMPTS;
                    if tx
                        .send(RuntimeEvent::DaemonEventsDisconnected {
                            generation,
                            message: error.to_string(),
                            retrying,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if !retrying {
                        return;
                    }
                }
            }
            if reconnect_attempts == 0 {
                reconnect_attempts = 1;
            }
            let delay = reconnect_backoff(
                DAEMON_EVENTS_RECONNECT_INITIAL_BACKOFF,
                DAEMON_EVENTS_RECONNECT_MAX_BACKOFF,
                reconnect_attempts,
                generation,
            );
            tokio::time::sleep(delay).await;
        }
    })
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
                            ChatSeqDecision::Suppress => {}
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
                            let unreachable = error.is_unreachable();
                            let auth_stale = error.is_auth_stale();
                            let _ = tx
                                .send(RuntimeEvent::ChatDisconnected {
                                    generation,
                                    message: error.to_string(),
                                    unreachable,
                                    auth_stale,
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
                        unreachable: ClientError::SseDisconnect("stream ended".to_string())
                            .is_unreachable(),
                        auth_stale: false,
                    })
                    .await;
            }
            Err(error) => {
                let unreachable = error.is_unreachable();
                let auth_stale = error.is_auth_stale();
                let _ = tx
                    .send(RuntimeEvent::ChatDisconnected {
                        generation,
                        message: error.to_string(),
                        unreachable,
                        auth_stale,
                    })
                    .await;
            }
        }
    })
}
