use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::client::{ChatEvent, DaemonClient, OpenProjectResponse, ProjectEntry, WorkerInfo};
use crate::terminal::TerminalSession;
use crate::vendored::markdown_stream::MarkdownStreamCollector;

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
    Tool(String),
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
    current_project: Option<OpenProjectResponse>,
    chat_id: String,
    model: Option<String>,
    session_state: SessionState,
    daemon_online: bool,
    scroll_offset: usize,
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
            current_project: Some(project),
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            session_state: SessionState::Idle,
            daemon_online: true,
            scroll_offset: 0,
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

    fn open_picker(&mut self, projects: Vec<ProjectEntry>) {
        self.picker = ProjectPickerState::new(projects);
        self.composer_mode = ComposerMode::ProjectPicker;
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
    }

    fn new_chat(&mut self) {
        self.chat_id = uuid::Uuid::new_v4().to_string();
        self.transcript.clear();
        self.transcript
            .push(TranscriptItem::Notice("New chat started".to_string()));
        self.session_state = SessionState::Idle;
        self.stream_collector.clear();
    }

    fn submit_composer(&mut self) -> Option<String> {
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
        Some(prompt)
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
            "pause_required" => {
                self.session_state = SessionState::Paused;
                self.add_notice(
                    "Approval required; use the web UI for rich approvals in this version.",
                );
            }
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
        }
        if let Some(runtime) = raw.get("runtime") {
            self.apply_runtime_state(runtime);
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
                        self.transcript
                            .push(TranscriptItem::Tool(tool_summary(tool)));
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind != KeyEventKind::Press {
            return AppAction::None;
        }
        if self.composer_mode == ComposerMode::ProjectPicker {
            return self.handle_picker_key(key);
        }
        match key {
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
                if matches!(
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
            } => self
                .submit_composer()
                .map(AppAction::SendMessage)
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

    fn ctrl_c_action(&mut self) -> AppAction {
        if matches!(
            self.session_state,
            SessionState::Generating | SessionState::ExecutingTools | SessionState::Paused
        ) {
            self.session_state = SessionState::Idle;
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

    fn handle_picker_key(&mut self, key: KeyEvent) -> AppAction {
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    None,
    LoadProjects,
    OpenProject(PathBuf),
    SubscribeCurrent,
    SendMessage(String),
    Abort,
}

#[derive(Debug)]
enum RuntimeEvent {
    Input(Event),
    Tick,
    Chat(ChatEvent),
    ChatDisconnected(String),
    ProjectsLoaded(Result<Vec<ProjectEntry>, String>),
    ProjectOpened(Result<OpenProjectResponse, String>),
    CommandFinished(Result<(), String>),
}

pub async fn run(options: TuiOptions) -> Result<(), TuiError> {
    let base_url = options
        .daemon_url
        .or_else(|| std::env::var("REFACT_DAEMON_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8488".to_string());
    let client = DaemonClient::new(base_url, None)?;
    let root = match options.project_hint {
        Some(path) => path,
        None => std::env::current_dir().map_err(|error| TuiError::Message(error.to_string()))?,
    };
    let project = client.open_project(&root).await?;
    let mut app = App::new(project);
    let mut terminal = TerminalSession::start()?;
    let (tx, mut rx) = mpsc::channel::<RuntimeEvent>(256);
    spawn_input_task(tx.clone());
    spawn_tick_task(tx.clone());
    if let Some(project_id) = app.current_project_id() {
        spawn_subscription_task(
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
                run_action(&mut app, action, &client, &tx);
            }
            RuntimeEvent::Input(Event::Resize(_, _)) | RuntimeEvent::Tick => {}
            RuntimeEvent::Input(_) => {}
            RuntimeEvent::Chat(event) => app.handle_chat_event(event),
            RuntimeEvent::ChatDisconnected(message) => {
                app.daemon_online = false;
                app.add_notice(format!("SSE disconnected: {message}; reconnecting…"));
            }
            RuntimeEvent::ProjectsLoaded(Ok(projects)) => app.open_picker(projects),
            RuntimeEvent::ProjectsLoaded(Err(error)) => {
                app.add_notice(format!("Failed to list projects: {error}"))
            }
            RuntimeEvent::ProjectOpened(Ok(project)) => {
                app.set_project(project);
                if let Some(project_id) = app.current_project_id() {
                    spawn_subscription_task(
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
                spawn_subscription_task(
                    client.clone(),
                    project_id.to_string(),
                    app.chat_id().to_string(),
                    tx.clone(),
                );
            }
        }
        AppAction::SendMessage(prompt) => {
            if let Some(project_id) = app.current_project_id().map(str::to_string) {
                let chat_id = app.chat_id().to_string();
                let client = client.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = async {
                        client
                            .send_set_params(
                                &project_id,
                                &chat_id,
                                json!({"mode": "agent", "tool_use": "agent"}),
                            )
                            .await?;
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
                        .send(RuntimeEvent::ChatDisconnected(format!(
                            "input error: {error}"
                        )))
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

fn spawn_subscription_task(
    client: DaemonClient,
    project_id: String,
    chat_id: String,
    tx: mpsc::Sender<RuntimeEvent>,
) {
    tokio::spawn(async move {
        loop {
            match client.subscribe_chat(&project_id, &chat_id).await {
                Ok(mut stream) => {
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(event) => {
                                if tx.send(RuntimeEvent::Chat(event)).await.is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                if tx
                                    .send(RuntimeEvent::ChatDisconnected(error.to_string()))
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
                        .send(RuntimeEvent::ChatDisconnected(error.to_string()))
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

fn tool_summary(tool: &Value) -> String {
    let name = tool
        .get("function")
        .and_then(|function| function.get("name"))
        .or_else(|| tool.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let args = tool
        .get("function")
        .and_then(|function| function.get("arguments"))
        .or_else(|| tool.get("arguments"))
        .or_else(|| tool.get("args"))
        .or_else(|| tool.get("input"))
        .map(|value| match value {
            Value::String(value) => value.clone(),
            value => value.to_string(),
        })
        .unwrap_or_default();
    let args = args.replace('\n', " ");
    let mut preview = String::new();
    for (idx, ch) in args.chars().enumerate() {
        if idx >= 96 {
            preview.push('…');
            break;
        }
        preview.push(ch);
    }
    if preview.is_empty() {
        name.to_string()
    } else {
        format!("{name}({preview})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn app_submits_message_and_streams_answer() {
        let mut app = App::new(project());
        app.composer = "hello".to_string();
        let action = app.handle_key(key(KeyCode::Enter));
        assert_eq!(action, AppAction::SendMessage("hello".to_string()));
        assert_eq!(app.session_state(), SessionState::Generating);
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            kind: "stream_delta".to_string(),
            raw: json!({"ops": [{"op": "append_content", "text": "hi"}]}),
        });
        assert!(
            matches!(app.visible_transcript().last(), Some(TranscriptItem::Assistant(text)) if text == "hi")
        );
        app.handle_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
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
}
