use std::ffi::OsString;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Terminal;
use refact_tui::app::{App, SessionState, UsageSummary};
use refact_tui::client::{ChatEvent, OpenProjectResponse, WorkerInfo};
use refact_tui::commands::{command_by_name, workflow, CommandAction};
use refact_tui::commands::session::{PermissionPolicy, StatusSnapshot, StatusUsage};
use refact_tui::pickers::{PickerKind, PickerItem, PickerState};
use refact_tui::protocol::TranscriptMessage;
use refact_tui::theme::TuiTheme;
use refact_tui::ui::{footer, status_card, status_indicator};
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

fn render_widget_snapshot<F>(width: u16, height: u16, draw: F) -> String
where
    F: FnOnce(&mut ratatui::Frame<'_>),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(draw).unwrap();
    terminal_snapshot(&terminal, width, height)
}

fn terminal_snapshot(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
    normalize_dynamic_durations(terminal_snapshot_raw(terminal, width, height))
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

    fn apply(self) -> EnvironmentGuard {
        match self {
            Self::TrueColor => EnvironmentGuard::set(&[
                ("TERM", Some("xterm-truecolor")),
                ("COLORTERM", Some("truecolor")),
                ("NO_COLOR", None),
            ]),
            Self::Ansi16 => EnvironmentGuard::set(&[
                ("TERM", Some("xterm-16color")),
                ("COLORTERM", None),
                ("NO_COLOR", None),
            ]),
            Self::NoColor => EnvironmentGuard::set(&[
                ("TERM", Some("dumb")),
                ("COLORTERM", None),
                ("NO_COLOR", Some("1")),
            ]),
        }
    }
}

struct EnvironmentGuard {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl EnvironmentGuard {
    fn set(values: &[(&'static str, Option<&str>)]) -> Self {
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
        Self { previous }
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

type ScenarioSetup = fn(&mut App);

struct RenderScenario {
    name: &'static str,
    marker: Option<&'static str>,
    setup: ScenarioSetup,
    render_before_resize: bool,
}

const SNAPSHOT_SIZES: [(u16, u16); 4] = [(120, 40), (96, 30), (60, 20), (40, 15)];

fn render_matrix_snapshot(
    scenario: &RenderScenario,
    color: ColorMode,
    width: u16,
    height: u16,
) -> String {
    let _environment = color.apply();
    let mut app = App::new(project());
    (scenario.setup)(&mut app);
    if scenario.render_before_resize {
        let _ = render_app_snapshot(&mut app, 120, 40);
    }
    render_app_snapshot(&mut app, width, height)
}

fn assert_registered_scenarios_have_snapshots(scenarios: &[RenderScenario]) {
    for scenario in scenarios {
        assert!(
            scenario.marker.is_some(),
            "registered render scenario `{}` has no snapshot marker",
            scenario.name
        );
    }
}

fn assert_matrix_snapshot(
    scenario: &RenderScenario,
    color: ColorMode,
    width: u16,
    height: u16,
    snapshot: &str,
) {
    let marker = scenario.marker.expect("registered scenario marker checked");
    assert!(
        snapshot.contains(marker),
        "{} {} {}x{} lost `{marker}`:\n{snapshot}",
        scenario.name,
        color.label(),
        width,
        height,
    );
    if width < 60 {
        assert_no_box_drawing(snapshot, scenario.name, color, width, height);
    }
}

fn assert_no_box_drawing(
    snapshot: &str,
    scenario: &str,
    color: ColorMode,
    width: u16,
    height: u16,
) {
    assert!(
        !snapshot.chars().any(|character| {
            matches!(
                character,
                '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '─' | '│'
            )
        }),
        "{scenario} {} {width}x{height} retained box drawing:\n{snapshot}",
        color.label(),
    );
}

fn idle_scenario(_app: &mut App) {}

fn streaming_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "generating"},
            "messages": [{"role": "user", "content": "stream prompt"}]
        }),
    ));
    app.apply_chat_event(chat_event(
        app,
        "stream_started",
        json!({"type": "stream_started", "message_id": "assistant-stream"}),
    ));
    app.apply_chat_event(chat_event(
        app,
        "stream_delta",
        json!({
            "type": "stream_delta",
            "message_id": "assistant-stream",
            "ops": [{"op": "append_content", "text": "Streaming response"}]
        }),
    ));
}

fn tool_running_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "executing_tools"},
            "messages": [{
                "role": "assistant",
                "tool_calls": [{"id": "call-running", "function": {"name": "shell", "arguments": "{}"}}]
            }]
        }),
    ));
}

fn tool_failed_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle"},
            "messages": [
                {"role": "assistant", "tool_calls": [{"id": "call-failed", "function": {"name": "shell", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "call-failed", "content": "failed command", "tool_failed": true}
            ]
        }),
    ));
}

fn approval_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "pause_required",
        json!({
            "type": "pause_required",
            "pause_id": "matrix-approval",
            "reasons": [{
                "type": "confirmation",
                "tool_name": "apply_patch",
                "command": "apply patch",
                "rule": "ask",
                "tool_call_id": "call-approval"
            }]
        }),
    ));
}

fn ask_form_scenario(app: &mut App) {
    let content = json!({
        "type": "ask_questions",
        "tool_call_id": "call-ask",
        "questions": [{"id": "confirm", "type": "yes_no", "text": "Continue matrix?"}]
    })
    .to_string();
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "waiting_user_input"},
            "messages": [
                {"role": "assistant", "tool_calls": [{"id": "call-ask", "function": {"name": "ask_questions", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "call-ask", "content": content}
            ]
        }),
    ));
}

fn error_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "error"},
            "messages": [{"role": "error", "content": "Provider unavailable", "_ui_only": true}]
        }),
    ));
}

fn goal_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle", "goal": {"active": true, "status": "pursuing", "turn_count": 1}},
            "messages": [{
                "role": "goal",
                "content": "Ship the snapshot matrix",
                "extra": {"goal": {"version": 1}}
            }]
        }),
    ));
}

fn mode_transition_scenario(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty()));
    app.apply_chat_event(chat_event(
        app,
        "message_added",
        json!({
            "type": "message_added",
            "message": {
                "role": "event",
                "content": "Mode transition",
                "extra": {"event": {"subkind": "mode_switch", "source": "chat.session", "payload": {"from": "ask", "to": "agent"}}}
            }
        }),
    ));
}

fn history_events_scenario(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty()));
    app.apply_chat_event(chat_event(
        app,
        "message_added",
        json!({
            "type": "message_added",
            "message": {
                "role": "event",
                "content": "History event retained",
                "extra": {"event": {"subkind": "process_completed", "source": "exec.registry", "payload": {"exit_code": 0}}}
            }
        }),
    ));
}

fn trajectory_500_turn_scenario(app: &mut App) {
    let messages = (0..500)
        .flat_map(|turn| {
            [
                json!({"role": "user", "content": format!("turn {turn} request")}),
                json!({"role": "assistant", "content": format!("turn {turn} response"), "stream_finished": true}),
            ]
        })
        .collect::<Vec<_>>();
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle"},
            "messages": messages
        }),
    ));
}

fn mid_resize_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "generating"},
            "messages": [{"role": "assistant", "content": "Resize keeps this visible", "stream_finished": true}]
        }),
    ));
}

fn post_reconnect_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "message_added",
        json!({
            "type": "message_added",
            "message": {"role": "notice", "content": "SSE resync restored the transcript"}
        }),
    ));
}

fn image_fallback_scenario(app: &mut App) {
    app.apply_chat_event(chat_event(
        app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle"},
            "messages": [{
                "role": "assistant",
                "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJDRA=="}}],
                "stream_finished": true
            }]
        }),
    ));
}

fn render_scenarios() -> Vec<RenderScenario> {
    vec![
        RenderScenario {
            name: "idle",
            marker: Some("Opened project"),
            setup: idle_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "streaming",
            marker: Some("Streaming response"),
            setup: streaming_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "tool running",
            marker: Some("running"),
            setup: tool_running_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "tool failed",
            marker: Some("failed"),
            setup: tool_failed_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "approval",
            marker: Some("Approval"),
            setup: approval_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "ask form",
            marker: Some("Question"),
            setup: ask_form_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "error turn",
            marker: Some("Error"),
            setup: error_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "goal dock",
            marker: Some("Current Goal"),
            setup: goal_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "mode transition",
            marker: Some("Mode sw"),
            setup: mode_transition_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "history events",
            marker: Some("Proces"),
            setup: history_events_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "500 turn trajectory",
            marker: Some("turn 499"),
            setup: trajectory_500_turn_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "mid resize",
            marker: Some("Resize keeps"),
            setup: mid_resize_scenario,
            render_before_resize: true,
        },
        RenderScenario {
            name: "post reconnect",
            marker: Some("SSE resync"),
            setup: post_reconnect_scenario,
            render_before_resize: false,
        },
        RenderScenario {
            name: "image fallback",
            marker: Some("[image:"),
            setup: image_fallback_scenario,
            render_before_resize: false,
        },
    ]
}

fn fixture_snapshot_messages() -> Vec<Value> {
    vec![
        json!({
            "message_id": "plan-1",
            "role": "plan",
            "content": "## Plan\n- inspect rendering\n- keep refact data",
            "extra": {"plan": {"mode": "agent", "version": 1}}
        }),
        json!({
            "message_id": "plan-delta-1",
            "role": "event",
            "content": "Add golden parity coverage.",
            "extra": {"event": {"subkind": "plan_delta", "source": "tool.update_plan", "payload": {"seq": 1}}}
        }),
        json!({
            "message_id": "u1",
            "role": "user",
            "content": "Inspect @src/lib.rs and summarize the TUI parity risks."
        }),
        json!({
            "message_id": "a1",
            "role": "assistant",
            "reasoning_content": "Need check parser. Then render.",
            "content": "## Findings\n- Parser ok\n- Visual style kept\n\n```rust\nfn main() {}\n```\n\n| kind | value |\n| --- | --- |\n| ok | yes |\n",
            "tool_calls": [
                {"id": "call-shell", "function": {"name": "shell", "arguments": "{\"command\":\"cargo test -p refact-tui\"}"}},
                {"id": "call-patch", "function": {"name": "apply_patch", "arguments": {"patch": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new"}}}
            ],
            "stream_finished": true
        }),
        json!({
            "message_id": "tool-shell",
            "role": "tool",
            "tool_call_id": "call-shell",
            "content": "ok\n\nThe command was running 0.120s, finished with exit code 0",
            "tool_failed": false
        }),
        json!({
            "message_id": "tool-patch",
            "role": "tool",
            "tool_call_id": "call-patch",
            "content": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new",
            "tool_failed": false
        }),
        json!({
            "message_id": "notice-1",
            "role": "notice",
            "content": "Daemon event captured separately."
        }),
    ]
}

#[test]
fn keymap_help_golden_snapshot() {
    let mut app = App::new(project());
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()));

    let actual = render_app_snapshot(&mut app, 100, 30);
    assert_snapshot(
        actual,
        r#"refact fixture | Ctrl-N new · Ctrl-P projects · Alt-M model · Ctrl-O mode · ? help
  • Opened project fixture at /tmp/fixture


    ┌──────────────────────────────────────────────────────────────────────────────────────────┐
    │Help                                                                                      │
    │Theme dark · vim off                                                                      │
    │                                                                                          │
    │        main ?                     show generated keymap help                             │
    │    projects Backspace             delete left or remove queued item                      │
    │     pickers Backspace             delete left or remove queued item                      │
    │   approvals a                     approve matching tools for chat                        │
    │     overlay Esc, q                cancel, close, or abort active work                    │
    │overlay search Backspace             delete left or remove queued item                    │
    │  vim normal a                     append after cursor and insert                         │
    │  vim insert Esc                   return to vim normal mode                              │
    │     history —                   not yet bound                                            │
    │    activity —                   not yet bound                                            │
    │       board —                   not yet bound                                            │
    │        goal —                   not yet bound                                            │
    │   worktrees —                   not yet bound                                            │
    │    settings —                   not yet bound                                            │
    │    ask form Backspace             delete left or remove queued item                      │
    │transcript cell t                     expand selected tool card                           │
    └──────────────────────────────────────────────────────────────────────────────────────────┘


│› Ask Refact…
│  Enter send   Ctrl-J newline
 ● idle · daemon online · fixture · default · agent · reason:off · worker ready"#,
    );
}

#[test]
fn transcript_cells_golden_snapshot() {
    let mut app = App::new(project());
    app.apply_chat_event(chat_event(
        &app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "title": "Parity sweep", "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle", "usage": {"prompt_tokens": 1200, "completion_tokens": 340, "total_tokens": 1540}},
            "messages": fixture_snapshot_messages()
        }),
    ));
    app.apply_caps(&json!({"chat_models": {"gpt-demo": {"n_ctx": 100000}}}));

    let actual = render_app_snapshot(&mut app, 100, 66);
    assert_snapshot(
        actual,
        r#"refact fixture | Ctrl-N new · Ctrl-P projects · Alt-M model · Ctrl-O mode · ? help
  >_ refact (v<version>)

  model: gpt-demo · /model to change
  directory: /tmp/fixture
  Tips: type /help for shortcuts; Ctrl-C twice exits

  • Proposed Plan


  plan · agent · v1 · 1 update


    ## Plan


    - inspect rendering
    - keep refact data


    ————————————————————————————————————————————————————————————————————————————————————————————————


    ## Plan updates


    Add golden parity coverage.




  › Inspect @src/lib.rs and summarize the TUI parity risks.

  • … 1 line hidden (expand)

  • ## Findings


    - Parser ok
    - Visual style kept


    fn main() {}


     kind    value
    ━━━━━━  ━━━━━━━
     ok      yes

  exec selected
  ▸ ✅  succeeded $ cargo test -p refact-tui · exit 0
    └ ok

  diff
  ▸ ✅  succeeded 1 file · +1 -1
  • Edited src/lib.rs (+1 -1)
  Δ src/lib.rs +1 -1

  • Daemon event captured separately.




│› Ask Refact…
│  Enter send   Ctrl-J newline
 98% context left (1.54K used) · ● idle · daemon online · fixture · gpt-demo · agent · reason:off ·…"#,
    );
}

#[test]
fn goal_role_renders_as_goal_banner() {
    let mut app = App::new(project());
    app.apply_chat_event(chat_event(
        &app,
        "snapshot",
        json!({
            "type": "snapshot",
            "thread": {"id": app.chat_id(), "title": "Goal sweep", "model": "gpt-demo", "mode": "agent"},
            "runtime": {"state": "idle"},
            "messages": [
                {
                    "message_id": "goal-1",
                    "role": "goal",
                    "content": "## Goal\n- ship hidden role rendering",
                    "extra": {"goal": {"version": 2}}
                },
                {
                    "message_id": "goal-delta-1",
                    "role": "event",
                    "content": "Add TUI parity coverage.",
                    "extra": {"event": {"subkind": "goal_delta", "source": "tool.update_goal", "payload": {"seq": 1}}}
                }
            ]
        }),
    ));

    let actual = render_app_snapshot(&mut app, 90, 28);

    assert!(actual.contains("• Current Goal"));
    assert!(actual.contains("goal · v2 · 1 update"));
    assert!(actual.contains("## Goal updates"));
    assert!(actual.contains("Add TUI parity coverage."));
}

#[test]
fn goal_command_is_state_display_and_synthesizes_goal_updates() {
    let command = command_by_name("goal").unwrap();
    assert_eq!(
        command.action,
        CommandAction::Workflow {
            command: workflow::WorkflowCommand::ShowGoal,
        }
    );

    let messages = vec![
        TranscriptMessage::from_wire(&json!({
            "role": "goal",
            "content": "base goal",
            "extra": {"goal": {"version": 1}}
        })),
        TranscriptMessage::from_wire(&json!({
            "role": "event",
            "content": "delta one",
            "extra": {"event": {"subkind": "goal_delta", "payload": {"seq": 1}}}
        })),
    ];

    assert_eq!(
        workflow::synthesize_current_goal(&messages).unwrap(),
        "base goal\n\n---\n\n## Goal updates\n\ndelta one"
    );
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
   │                                                                                  │
   │                                                                                  │
   │                                                                                  │
   │y approve · a approve for chat · n reject · v details · Esc                       │
   └──────────────────────────────────────────────────────────────────────────────────┘

│› Ask Refact…
│  approval pending · Enter queues · Esc cancels   Enter send   Ctrl-J newline   Enter qu…
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
fn status_footer_and_working_indicator_golden_snapshots() {
    let status_data = status_indicator::StatusIndicatorData {
        state: SessionState::Generating,
        elapsed_ms: 65_000,
        tick: 4,
        detail: Some(
            "apply_patch({\"file\":\"src/ui/mod.rs\"}) completed while polishing parity"
                .to_string(),
        ),
        reduced_motion: true,
        interrupt_key: "Esc".to_string(),
    };
    let status_lines = status_indicator::status_indicator_lines(&status_data, 72).unwrap();
    let status_actual = render_widget_snapshot(72, 4, |frame| {
        Paragraph::new(status_lines).render(frame.area(), frame.buffer_mut());
    });
    assert_snapshot(
        status_actual,
        r#" • Working (1m 05s • Esc to interrupt)
  └ apply_patch({"file":"src/ui/mod.rs"}) completed while polishing
    parity"#,
    );

    let footer_data = footer::FooterData {
        project: "fixture".to_string(),
        model: "gpt-demo".to_string(),
        mode: "agent".to_string(),
        reasoning: "high".to_string(),
        runtime_state: footer::FooterRuntimeState::Generating,
        worker: "ready".to_string(),
        usage: Some(UsageSummary {
            prompt_tokens: Some(1200),
            completion_tokens: Some(340),
            total_tokens: Some(1540),
        }),
        context_window_tokens: Some(100000),
        retry_hint: Some("retry available".to_string()),
        interrupt_key: "Esc".to_string(),
        retry_key: "Ctrl-Shift-R".to_string(),
    };
    let footer_actual = render_widget_snapshot(100, 1, |frame| {
        Paragraph::new(footer::footer_line(&footer_data)).render(frame.area(), frame.buffer_mut());
    });
    assert_snapshot(
        footer_actual,
        r#" 98% context left (1.54K used) · ◆ generating · Esc to interrupt · daemon online · fixture · gpt-dem"#,
    );
}

#[test]
fn status_command_card_golden_snapshot() {
    let snapshot = StatusSnapshot {
        daemon_online: true,
        daemon_version: Some("1.2.3".to_string()),
        daemon_port: Some(32000),
        daemon_base_url: Some("http://127.0.0.1:32000".to_string()),
        worker: "ready · pid 42 · http 32000 · lsp 32001".to_string(),
        project: "fixture".to_string(),
        project_root: Some("/tmp/fixture".to_string()),
        model: "gpt-demo".to_string(),
        mode: "agent".to_string(),
        reasoning: "high".to_string(),
        permission_policy: PermissionPolicy {
            auto_approve_editing_tools: true,
            auto_approve_dangerous_commands: false,
        },
        session_id: "chat-fixture".to_string(),
        usage: Some(StatusUsage {
            prompt_tokens: Some(1200),
            completion_tokens: Some(340),
            total_tokens: Some(1540),
            context_window_tokens: Some(100000),
        }),
        retry_hint: Some("retry available".to_string()),
    };

    let actual = render_widget_snapshot(88, 16, |frame| {
        let paragraph = status_card::render(frame.area().width, &snapshot, &TuiTheme::dark());
        frame.render_widget(paragraph, frame.area());
    });
    assert_snapshot(
        actual,
        r#"╭──────────────────────────────────────────────────────────────────────────────────────╮
│  refact (v<version>)                                                                     │
│                                                                                      │
│  Daemon:                v1.2.3 on port 32000                                         │
│  Worker:                ready · pid 42 · http 32000 · lsp 32001                      │
│  Model:                 gpt-demo                                                     │
│  Mode:                  agent                                                        │
│  Reasoning:             high                                                         │
│  Terminal background:   unavailable (OSC 10/11 probe timed out or is unsupported)    │
│  Directory:             /tmp/fixture                                                 │
│  Permissions:           auto_approve_editing_tools=true · auto_approve_dangerous_com │
│  Token usage:           1.54K total (1.2K input + 340 output)                        │
│  Context window:        98% left (1.54K/100K)                                        │
│  Retry hint:            retry available                                              │
╰──────────────────────────────────────────────────────────────────────────────────────╯"#,
    );
}

#[test]
fn picker_golden_snapshots() {
    let modal_picker = PickerState::new(
        PickerKind::Model,
        vec![
            PickerItem {
                id: "gpt-demo".to_string(),
                title: "GPT Demo".to_string(),
                description: "default · tools".to_string(),
            },
            PickerItem {
                id: "claude-demo".to_string(),
                title: "Claude Demo".to_string(),
                description: "reasoning · long context".to_string(),
            },
        ],
    );
    let modal_actual = render_widget_snapshot(88, 18, |frame| {
        refact_tui::ui::picker::render_modal_picker(
            frame,
            &modal_picker,
            frame.area(),
            Rect::new(0, 14, 88, 3),
        );
    });
    assert_snapshot(
        modal_actual,
        r#"







    ┌──────────────────────────────────────────────────────────────────────────────┐
    │models:                                                                       │
    │› GPT Demo     default · tools                                                │
    │  Claude Demo  reasoning · long context                                       │
    │Press Enter to confirm or Esc to go back                                      │
    └──────────────────────────────────────────────────────────────────────────────┘"#,
    );

    let mut slash_picker = PickerState::new(
        PickerKind::SlashCommand,
        vec![
            PickerItem {
                id: "status".to_string(),
                title: "/status".to_string(),
                description: "show daemon status".to_string(),
            },
            PickerItem {
                id: "theme".to_string(),
                title: "/theme".to_string(),
                description: "choose TUI theme".to_string(),
            },
        ],
    );
    slash_picker.push_filter('s');
    let slash_actual = render_widget_snapshot(88, 18, |frame| {
        refact_tui::ui::picker::render_modal_picker(
            frame,
            &slash_picker,
            frame.area(),
            Rect::new(0, 14, 88, 3),
        );
    });
    assert_snapshot(
        slash_actual,
        r#"









    ┌──────────────────────────────────────────────────────────────────────────────┐
    │/status                show daemon status                                     │
    │/theme                 choose TUI theme                                       │
    └──────────────────────────────────────────────────────────────────────────────┘"#,
    );
}

#[test]
fn events_pane_golden_snapshot() {
    let mut app = App::new(project());
    app.apply_chat_event(chat_event(
        &app,
        "message_added",
        json!({
            "type": "message_added",
            "message": {
                "message_id": "event-1",
                "role": "event",
                "content": "Process cargo test exited with code 0",
                "extra": {"event": {"subkind": "process_completed", "source": "exec.registry", "payload": {"process_id": "exec_1", "exit_code": 0}}}
            }
        }),
    ));

    let actual = render_widget_snapshot(88, 12, |frame| {
        refact_tui::ui::events::render_events_pane(frame, &app, frame.area());
    });
    assert_snapshot(
        actual,
        r#"┌──────────────────────────────────────────────────────────────────────────────────────┐
│daemon events                                    │workers                             │
│<time> · ■ · exec.registry · Process completed…│No workers                          │
│                                                 │                                    │
│                                                 │                                    │
│                                                 │                                    │
│                                                 │                                    │
│                                                 │                                    │
│                                                 │                                    │
│                                                 │                                    │
│                                                 │                                    │
└──────────────────────────────────────────────────────────────────────────────────────┘"#,
    );
}

#[test]
fn degradation_matrix_covers_registered_scenarios() {
    let scenarios = render_scenarios();
    assert_registered_scenarios_have_snapshots(&scenarios);

    for scenario in &scenarios {
        for color in ColorMode::ALL {
            for (width, height) in SNAPSHOT_SIZES {
                let snapshot = render_matrix_snapshot(scenario, color, width, height);
                assert_matrix_snapshot(scenario, color, width, height, &snapshot);
            }
        }
    }
}

#[test]
#[should_panic(expected = "registered render scenario `missing` has no snapshot marker")]
fn registered_scenario_without_snapshot_marker_fails_loudly() {
    assert_registered_scenarios_have_snapshots(&[RenderScenario {
        name: "missing",
        marker: None,
        setup: idle_scenario,
        render_before_resize: false,
    }]);
}

#[test]
fn compact_layout_reserves_transcript_and_marks_truncation() {
    let _environment = ColorMode::NoColor.apply();
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

#[test]
fn compact_layout_prioritizes_transcript_over_secondary_docks() {
    let _environment = ColorMode::NoColor.apply();
    let mut app = App::new(project());
    history_events_scenario(&mut app);

    let snapshot = render_app_snapshot(&mut app, 30, 10);

    assert!(!snapshot.contains("daemon events"), "{snapshot}");
    assert!(snapshot.contains("… content truncated"), "{snapshot}");
}

#[test]
fn narrow_modals_drop_borders_and_events_stack() {
    let _environment = ColorMode::NoColor.apply();
    let approval = RenderScenario {
        name: "approval",
        marker: Some("Approval"),
        setup: approval_scenario,
        render_before_resize: false,
    };
    let modal_snapshot = render_matrix_snapshot(&approval, ColorMode::NoColor, 39, 20);
    assert!(!modal_snapshot.contains("+---"), "{modal_snapshot}");
    assert!(!modal_snapshot.contains("|Approval"), "{modal_snapshot}");

    let events = RenderScenario {
        name: "history events",
        marker: Some("Proces"),
        setup: history_events_scenario,
        render_before_resize: false,
    };
    let events_snapshot = render_matrix_snapshot(&events, ColorMode::NoColor, 59, 20);
    let event_row = events_snapshot
        .lines()
        .position(|line| line.contains("daemon events"))
        .unwrap();
    let worker_row = events_snapshot
        .lines()
        .position(|line| line.contains("workers"))
        .unwrap();
    assert!(event_row < worker_row, "{events_snapshot}");
    assert_no_box_drawing(
        &events_snapshot,
        "history events",
        ColorMode::NoColor,
        59,
        20,
    );
}
