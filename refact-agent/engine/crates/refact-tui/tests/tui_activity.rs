use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use refact_tui::app::App;
use refact_tui::client::{ChatEvent, OpenProjectResponse, WorkerInfo};
use serde_json::json;

static SURFACES_ENV_LOCK: Mutex<()> = Mutex::new(());

fn project() -> OpenProjectResponse {
    OpenProjectResponse {
        project_id: "p1".to_string(),
        slug: "fixture".to_string(),
        root: PathBuf::from("/tmp/fixture"),
        pinned: Some(false),
        worker: Some(WorkerInfo {
            project_id: "p1".to_string(),
            slug: "fixture".to_string(),
            root: PathBuf::from("/tmp/fixture"),
            root_exists: Some(true),
            pinned: Some(false),
            last_active_ms: Some(1),
            state: json!("ready"),
            pid: Some(2),
            rss_bytes: Some(3),
            cpu_percent: Some(4.5),
            uptime_secs: Some(5),
            http_port: Some(6),
            lsp_port: Some(7),
            lsp_clients: Some(8),
            busy_chats: Some(9),
            exec_running: Some(10),
            live_proxy_streams: Some(11),
            cron_next_fire_ms: Some(12),
            idle_deadline_ms: Some(13),
            last_status_report_ms: Some(14),
            last_error: None,
            log_path: "/tmp/fixture.log".to_string(),
        }),
        cron_pending: Some(1),
    }
}

fn event(app: &App, kind: &str, raw: serde_json::Value) -> ChatEvent {
    ChatEvent {
        chat_id: Some(app.chat_id().to_string()),
        seq: None,
        kind: kind.to_string(),
        raw,
    }
}

fn snapshot_with_agents(app: &App) -> ChatEvent {
    event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle"},
            "messages": [{
                "role": "assistant",
                "content": "Starting service",
                "tool_calls": [{
                    "id": "proc-1",
                    "function": {"name": "process_start", "arguments": "{\"service_name\":\"demo\"}"},
                    "status": "running"
                }],
                "stream_finished": true
            }],
            "background_agents": [
                {
                    "agentId": "agent-1",
                    "childChatId": "child-1",
                    "kind": "task",
                    "status": "running",
                    "title": "Implement parser",
                    "progress": "Writing tests",
                    "stepCount": 3,
                    "editedFiles": ["src/parser.rs"],
                    "diffSummary": "one file changed",
                    "conflictSummary": "none",
                    "resultSummary": "partial"
                },
                {
                    "agentId": "agent-2",
                    "childChatId": "child-2",
                    "kind": "review",
                    "status": "queued",
                    "title": "Review parser",
                    "progress": "Waiting"
                }
            ]
        }),
    )
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    app.set_native_scrollback(false);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| refact_tui::ui::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

struct SurfaceEnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl SurfaceEnvGuard {
    fn enable() -> Self {
        let lock = SURFACES_ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("REFACT_TUI_SURFACES");
        std::env::set_var("REFACT_TUI_SURFACES", "1");
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for SurfaceEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("REFACT_TUI_SURFACES", previous);
        } else {
            std::env::remove_var("REFACT_TUI_SURFACES");
        }
    }
}

#[test]
fn subagents_surface_renders_background_agents_worker_telemetry_and_processes() {
    let _surfaces = SurfaceEnvGuard::enable();
    let mut app = App::new(project());
    app.apply_chat_event(snapshot_with_agents(&app));
    app.apply_chat_event(event(
        &app,
        "process_completed",
        json!({
            "type": "process_completed",
            "process_id": "exec-1",
            "status": "exited",
            "exit_code": 0,
            "short_description": "cargo test",
            "mode": "service"
        }),
    ));

    app.execute_command_name("subagents");
    let text = app.transcript_overlay().unwrap().lines().join("\n");

    assert!(text.contains("Background agents / delegates"));
    assert!(text.contains("[running] Implement parser"));
    assert!(text.contains("progress: Writing tests"));
    assert!(text.contains("edited files: src/parser.rs"));
    assert!(text.contains("diff summary: one file changed"));
    assert!(text.contains("conflict summary: none"));
    assert!(text.contains("result summary: partial"));
    assert!(text.contains("Worker telemetry"));
    for field in [
        "project ID",
        "slug",
        "root",
        "root exists",
        "pinned",
        "last active ms",
        "state",
        "PID",
        "RSS bytes",
        "CPU percent",
        "uptime secs",
        "HTTP port",
        "LSP port",
        "LSP clients",
        "busy chats",
        "exec running",
        "live proxy streams",
        "cron next fire ms",
        "idle deadline ms",
        "last status report ms",
        "last error",
        "log path",
    ] {
        assert!(text.contains(field), "missing {field}");
    }
    assert!(text.contains("Processes and services"));
    assert!(text.contains("cargo test · exited · service"));
    assert!(text.contains("process_start · running"));
}

#[test]
fn background_agent_update_refreshes_an_open_activity_surface() {
    let _surfaces = SurfaceEnvGuard::enable();
    let mut app = App::new(project());
    app.apply_chat_event(snapshot_with_agents(&app));
    app.execute_command_name("subagents");
    assert!(app
        .transcript_overlay()
        .unwrap()
        .lines()
        .join("\n")
        .contains("[running] Implement parser"));

    app.apply_chat_event(event(
        &app,
        "background_agent_updated",
        json!({
            "type": "background_agent_updated",
            "agent": {
                "agentId": "agent-1",
                "childChatId": "child-1",
                "kind": "task",
                "status": "completed",
                "title": "Implement parser",
                "progress": "Done",
                "editedFiles": ["src/parser.rs"],
                "resultSummary": "passed"
            }
        }),
    ));

    let text = app.transcript_overlay().unwrap().lines().join("\n");
    assert!(text.contains("[completed] Implement parser"));
    assert!(text.contains("progress: Done"));
    assert!(text.contains("result summary: passed"));
}

#[test]
fn opening_selected_agent_navigates_to_child_chat() {
    let _surfaces = SurfaceEnvGuard::enable();
    let mut app = App::new(project());
    app.apply_chat_event(snapshot_with_agents(&app));
    app.execute_command_name("subagents");

    let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));

    assert!(matches!(
        action,
        refact_tui::app::AppAction::SubscribeCurrent
    ));
    assert_eq!(app.chat_id(), "child-1");
}

#[test]
fn activity_surface_honors_narrow_degradation_rules_at_40_by_15() {
    let _surfaces = SurfaceEnvGuard::enable();
    let mut app = App::new(project());
    app.apply_chat_event(snapshot_with_agents(&app));
    app.execute_command_name("subagents");

    let text = render(&mut app, 40, 15);

    assert!(text.contains("Activity"));
    assert!(text.contains("Background"));
    assert!(!text.chars().any(|character| {
        matches!(
            character,
            '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '─' | '│'
        )
    }));
}
