use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use refact_tui::app::{App, AppAction};
use refact_tui::client::{ChatEvent, OpenProjectResponse, WorkerInfo};
use serde_json::{json, Value};

fn project() -> OpenProjectResponse {
    OpenProjectResponse {
        project_id: "p1".to_string(),
        slug: "fixture".to_string(),
        root: PathBuf::from("/tmp/fixture"),
        pinned: Some(false),
        worker: Some(WorkerInfo {
            project_id: "p1".to_string(),
            pid: Some(42),
            http_port: Some(32000),
            lsp_port: Some(32001),
            state: json!("ready"),
            last_error: None,
            ..WorkerInfo::default()
        }),
        cron_pending: Some(2),
    }
}

fn chat_event(app: &App, kind: &str, raw: Value) -> ChatEvent {
    ChatEvent {
        chat_id: Some(app.chat_id().to_string()),
        seq: None,
        kind: kind.to_string(),
        raw,
    }
}

fn render_app_snapshot(app: &mut App, width: u16, height: u16) -> String {
    normalize_dynamic_durations(render_app_snapshot_raw(app, width, height))
}

fn render_app_snapshot_raw(app: &mut App, width: u16, height: u16) -> String {
    app.set_native_scrollback(false);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| refact_tui::ui::render(frame, app))
        .unwrap();
    terminal_snapshot_raw(&terminal, width, height)
}

fn terminal_snapshot_raw(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
    let cells = terminal.backend().buffer().content();
    (0..height as usize)
        .map(|row| {
            let start = row * width as usize;
            let end = start + width as usize;
            cells[start..end]
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn normalize_dynamic_durations(snapshot: String) -> String {
    let duration_re = regex_lite::Regex::new(r" · [0-9]+ms").unwrap();
    let chat_id_re = regex_lite::Regex::new(r" · [0-9a-f]{8} ─").unwrap();
    let event_timestamp_re = regex_lite::Regex::new(r"│[0-9]{2}:[0-9]{2}:[0-9]{2} ·").unwrap();
    let version_re = regex_lite::Regex::new(r"refact \(v[^)]+\)").unwrap();
    let snapshot = duration_re.replace_all(&snapshot, " · <ms>");
    let snapshot = chat_id_re.replace_all(&snapshot, " · <chat> ─");
    let snapshot = event_timestamp_re.replace_all(&snapshot, "│<time> ·");
    version_re
        .replace_all(snapshot.trim_end_matches('\n'), "refact (v<version>)")
        .to_string()
}

fn assert_snapshot(actual: String, expected: &str) {
    assert_eq!(actual, expected);
}

#[derive(Clone, Copy)]
enum ColorMode {
    TrueColor,
    Ansi16,
    NoColor,
}

impl ColorMode {
    const ALL: [Self; 3] = [Self::TrueColor, Self::Ansi16, Self::NoColor];

    fn label(self) -> &'static str {
        match self {
            Self::TrueColor => "truecolor",
            Self::Ansi16 => "ansi16",
            Self::NoColor => "no-color",
        }
    }

    fn apply(self, surfaces: Option<&str>) -> EnvironmentGuard {
        match self {
            Self::TrueColor => EnvironmentGuard::set(&[
                ("TERM", Some("xterm-truecolor")),
                ("COLORTERM", Some("truecolor")),
                ("NO_COLOR", None),
                ("REFACT_TUI_SURFACES", surfaces),
            ]),
            Self::Ansi16 => EnvironmentGuard::set(&[
                ("TERM", Some("xterm-16color")),
                ("COLORTERM", None),
                ("NO_COLOR", None),
                ("REFACT_TUI_SURFACES", surfaces),
            ]),
            Self::NoColor => EnvironmentGuard::set(&[
                ("TERM", Some("dumb")),
                ("COLORTERM", None),
                ("NO_COLOR", Some("1")),
                ("REFACT_TUI_SURFACES", surfaces),
            ]),
        }
    }
}

static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

struct EnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl EnvironmentGuard {
    fn set(values: &[(&'static str, Option<&str>)]) -> Self {
        let lock = ENVIRONMENT_LOCK.lock().unwrap();
        let previous = values
            .iter()
            .map(|(key, value)| {
                let previous = std::env::var_os(key);
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
                (*key, previous)
            })
            .collect();
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[test]
fn approval_overlay_golden_snapshot() {
    let mut app = App::new(project());
    app.apply_chat_event(chat_event(
        &app,
        "pause_required",
        json!({
            "type": "pause_required",
            "pause_id": "approval-1",
            "reasons": [
                {
                    "type": "confirmation",
                    "tool_name": "apply_patch",
                    "command": "apply patch",
                    "rule": "ask",
                    "tool_call_id": "call-patch",
                    "args": {"patch": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new"},
                    "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new"
                }
            ]
        }),
    ));

    let actual = render_app_snapshot(&mut app, 90, 24);
    assert_snapshot(
        actual,
        r#"refact fixture | Ctrl-N new · Ctrl-P projects · Alt-M model · Ctrl-O mode · ? help
  • Opened project fixture at /tmp/fixture

   ┌──────────────────────────────────────────────────────────────────────────────────┐
   │Approval required · approval 1 of 1                                               │
   │› apply_patch  apply patch                                                        │
   │  rule: ask                                                                       │
   │                                                                                  │
   │                                                                                  │
   │                                                                                  │
   │                                                                                  │
   │                                                                                  │
   │                                                                                  │
   │                                                                                  │
   │y approve · a approve for chat · n reject · v details · Esc                       │
   └──────────────────────────────────────────────────────────────────────────────────┘



╭ approval pending · Enter queues · Esc cancels ─────────────────────────────────────────╮
│ › Ask Refact…                                                                          │
╰────────────────────────────────────────────────────────────────────────────────────────╯
  Enter queue  ·  \+Enter newline  ·  Esc interrupt  ·  ? shortcuts
 ◆ approval pending · Esc to interrupt · daemon online · fixture · default · agent · reas…"#,
    );
}

#[test]
fn ask_form_bottom_pane_golden_snapshot() {
    let mut app = App::new(project());
    let ask_content = json!({
        "type": "ask_questions",
        "tool_call_id": "call-ask",
        "questions": [
            {"id": "path", "type": "single_select", "text": "Which file should get the parity snapshot?", "options": ["tests/tui_render_parity.rs", "src/ui/mod.rs"]},
            {"id": "notes", "type": "free_text", "text": "Any visual notes?"}
        ]
    })
    .to_string();
    app.apply_chat_event(chat_event(
        &app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "waiting_user_input"},
            "messages": [
                {
                    "message_id": "a1",
                    "role": "assistant",
                    "tool_calls": [{"id": "call-ask", "function": {"name": "ask_questions", "arguments": "{}"}}],
                    "stream_finished": true
                },
                {
                    "message_id": "tool-ask",
                    "role": "tool",
                    "tool_call_id": "call-ask",
                    "content": ask_content,
                    "tool_failed": false
                }
            ]
        }),
    ));

    let first = render_app_snapshot_raw(&mut app, 90, 24);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = render_app_snapshot_raw(&mut app, 90, 24);
    assert_eq!(first, second);

    let actual = normalize_dynamic_durations(first);
    assert_snapshot(
        actual,
        r#"refact fixture | Ctrl-N new · Ctrl-P projects · Alt-M model · Ctrl-O mode · ? help
  • Questions
  ▸ ✅  succeeded ask_questions({})










┌────────────────────────────────────────────────────────────────────────────────────────┐
│Question 1/2                                                                            │
│Which file should get the parity snapshot? (answer required)                            │
│                                                                                        │
│› ○ tests/tui_render_parity.rs                                                          │
│  ○ src/ui/mod.rs                                                                       │
│                                                                                        │
│Press Enter to confirm or Esc to go back                                                │
│↑/↓ choose · ←/→ question                                                               │
└────────────────────────────────────────────────────────────────────────────────────────┘
 ◆ waiting for input · Esc to interrupt · daemon online · fixture · gpt-demo · agent · re…"#,
    );
}

#[test]
fn compact_layout_reserves_transcript_and_marks_truncation() {
    let _environment = ColorMode::NoColor.apply(None);
    let mut app = App::new(project());
    app.apply_chat_event(chat_event(
        &app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "generating"},
            "messages": [{"role": "assistant", "content": "Transcript survives the compact layout", "stream_finished": true}]
        }),
    ));

    let snapshot = render_app_snapshot(&mut app, 30, 10);

    assert!(snapshot.contains("Transcript"), "{snapshot}");
    assert!(snapshot.contains("… content truncated"), "{snapshot}");
}
