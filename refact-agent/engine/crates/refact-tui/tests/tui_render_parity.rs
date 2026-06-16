use std::path::PathBuf;

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Terminal;
use refact_tui::app::{App, SessionState, UsageSummary};
use refact_tui::client::{ChatEvent, OpenProjectResponse, WorkerInfo};
use refact_tui::commands::session::{PermissionPolicy, StatusSnapshot, StatusUsage};
use refact_tui::pickers::{PickerKind, PickerItem, PickerState};
use refact_tui::theme::TuiTheme;
use refact_tui::ui::{footer, status_card, status_indicator};
use serde_json::{json, Value};

fn project() -> OpenProjectResponse {
    OpenProjectResponse {
        project_id: "p1".to_string(),
        slug: "fixture".to_string(),
        root: PathBuf::from("/tmp/fixture"),
        pinned: false,
        worker: Some(WorkerInfo {
            project_id: "p1".to_string(),
            pid: Some(42),
            http_port: 32000,
            lsp_port: 32001,
            state: json!("ready"),
            last_error: None,
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
    app.set_native_scrollback(false);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| refact_tui::ui::render(frame, app))
        .unwrap();
    terminal_snapshot(&terminal, width, height)
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
    let cells = terminal.backend().buffer().content();
    let snapshot = (0..height as usize)
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
        .join("\n");
    normalize_dynamic_durations(snapshot)
}
fn normalize_dynamic_durations(snapshot: String) -> String {
    let duration_re = regex_lite::Regex::new(r" · [0-9]+ms").unwrap();
    let chat_id_re = regex_lite::Regex::new(r" · [0-9a-f]{8} ─").unwrap();
    let version_re = regex_lite::Regex::new(r"refact \(v[^)]+\)").unwrap();
    let snapshot = duration_re.replace_all(&snapshot, " · <ms>");
    let snapshot = chat_id_re.replace_all(&snapshot, " · <chat> ─");
    version_re
        .replace_all(snapshot.trim_end_matches('\n'), "refact (v<version>)")
        .to_string()
}

fn assert_snapshot(actual: String, expected: &str) {
    assert_eq!(actual, expected);
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
  ─ Parity sweep · fixture · gpt-demo · agent · <chat> ───────────────────────────────────────────

  • Proposed Plan


  plan · agent · v1 · 1 update


    ## Plan


    - inspect rendering
    - keep refact data


    ———


    ## Plan updates


    Add golden parity coverage.




  › Inspect @src/lib.rs and summarize the TUI parity risks.

  • collapsed

  • ## Findings


    - Parser ok
    - Visual style kept


    fn main() {}


     kind    value
    ━━━━━━  ━━━━━━━
     ok      yes

  exec selected
  ▸ ✅  $ cargo test -p refact-tui · exit 0 · <ms>
    └ ok

  diff
  ▸ ✅  1 file · +1 -1 · <ms>
  • Edited src/lib.rs (+1 -1)
  Δ src/lib.rs +1 -1

  • Daemon event captured separately.








› Ask Refact…
  Enter send   Ctrl-J newline
 98% context left (1.54K used) · fixture · gpt-demo · agent · reason:off · ● idle · daemon online ·…"#,
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



     Approval required · approval 1 of 1
     › apply_patch  apply patch
       rule: ask










     y approve · a approve for chat · n reject · v details · Esc


› Ask Refact…
  approval pending   Enter send   Ctrl-J newline
 fixture · default · agent · reason:off · ◆ generating · Esc to interrupt · daemon online…"#,
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

    let actual = render_app_snapshot(&mut app, 90, 24);
    assert_snapshot(
        actual,
        r#"refact fixture | Ctrl-N new · Ctrl-P projects · Alt-M model · Ctrl-O mode · ? help
  • Questions
  ▸ ✅  ask_questions({}) · <ms>











  Question 1/2
  Which file should get the parity snapshot?

  › ◉ tests/tui_render_parity.rs
    ○ src/ui/mod.rs

  Press Enter to confirm or Esc to go back
  ↑/↓ choose · ←/→ question

 fixture · gpt-demo · agent · reason:off · ● idle · daemon online · worker ready"#,
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
            prompt_tokens: 1200,
            completion_tokens: 340,
            total_tokens: 1540,
        }),
        context_window_tokens: Some(100000),
        retry_hint: Some("retry available".to_string()),
        interrupt_key: "Esc".to_string(),
    };
    let footer_actual = render_widget_snapshot(100, 1, |frame| {
        Paragraph::new(footer::footer_line(&footer_data)).render(frame.area(), frame.buffer_mut());
    });
    assert_snapshot(
        footer_actual,
        r#" 98% context left (1.54K used) · fixture · gpt-demo · agent · reason:high · ◆ generating · Esc to in"#,
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
            prompt_tokens: 1200,
            completion_tokens: 340,
            total_tokens: 1540,
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
│  Daemon:           v1.2.3 on port 32000                                              │
│  Worker:           ready · pid 42 · http 32000 · lsp 32001                           │
│  Model:            gpt-demo                                                          │
│  Mode:             agent                                                             │
│  Reasoning:        high                                                              │
│  Directory:        /tmp/fixture                                                      │
│  Permissions:      auto_approve_editing_tools=true · auto_approve_dangerous_commands │
│  Token usage:      1.54K total (1.2K input + 340 output)                             │
│  Context window:   98% left (1.54K/100K)                                             │
│  Retry hint:       retry available                                                   │
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








      models:
      › GPT Demo     default · tools
        Claude Demo  reasoning · long context
      Press Enter to confirm or Esc to go back"#,
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










      /status               show daemon status
      /theme                choose TUI theme"#,
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
        r#"
  daemon events                                    workers
  p1 chat.process_completed                        No workers
  {"source":"exec.registry","content":"Process
  cargo test exited with code
  0","payload":{"process_…"#,
    );
}
