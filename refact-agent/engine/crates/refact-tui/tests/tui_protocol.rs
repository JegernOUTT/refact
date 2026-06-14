use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use refact_tui::app::{App, SessionState, TranscriptItem};
use refact_tui::client::{
    discover_daemon_endpoint, discover_daemon_endpoint_from, resolve_daemon_endpoint, ChatEvent,
    ChatSeqDecision, ChatSeqTracker, DaemonClient, OpenProjectResponse, ToolDecision,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::{json, Value};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
struct State {
    commands: Arc<(Mutex<Vec<Value>>, Condvar)>,
    authorizations: Arc<(Mutex<Vec<Option<String>>>, Condvar)>,
    chat_script: Arc<Mutex<Option<Vec<Value>>>>,
}

impl State {
    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "fixture".to_string(),
            root: std::path::PathBuf::from("/tmp/fixture"),
            pinned: false,
            worker: None,
            cron_pending: None,
        }
    }

    fn record_command(&self, command: Value) {
        let (lock, cond) = &*self.commands;
        lock.lock().unwrap().push(command);
        cond.notify_all();
    }

    fn record_authorization(&self, authorization: Option<String>) {
        let (lock, cond) = &*self.authorizations;
        lock.lock().unwrap().push(authorization);
        cond.notify_all();
    }

    fn with_chat_script(events: Vec<Value>) -> Self {
        let state = Self::default();
        *state.chat_script.lock().unwrap() = Some(events);
        state
    }

    fn chat_script(&self) -> Option<Vec<Value>> {
        self.chat_script.lock().unwrap().clone()
    }

    fn wait_for_tool_decisions(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (lock, cond) = &*self.commands;
        let mut commands = lock.lock().unwrap();
        loop {
            if commands.iter().any(|command| {
                command.get("type").and_then(Value::as_str) == Some("tool_decisions")
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

    fn wait_for_authorization(&self, expected: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (lock, cond) = &*self.authorizations;
        let mut authorizations = lock.lock().unwrap();
        loop {
            if authorizations
                .iter()
                .any(|value| value.as_deref() == Some(expected))
            {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let wait = deadline.saturating_duration_since(now);
            let (next_authorizations, timeout) = cond.wait_timeout(authorizations, wait).unwrap();
            authorizations = next_authorizations;
            if timeout.timed_out() {
                return false;
            }
        }
    }
}

struct FixtureRun {
    app: App,
    recovery: Option<String>,
}

fn fixture_events(name: &str) -> Vec<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()));
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(idx, line)| {
            serde_json::from_str::<Value>(line).unwrap_or_else(|error| {
                panic!(
                    "invalid JSONL in {} line {}: {error}",
                    path.display(),
                    idx + 1
                )
            })
        })
        .collect()
}

fn chat_event_from_fixture(raw: Value, chat_id: &str) -> ChatEvent {
    let seq = match raw.get("seq") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(value)) => Some(value.parse::<u64>().unwrap()),
        _ => None,
    };
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    ChatEvent {
        chat_id: Some(chat_id.to_string()),
        seq,
        kind,
        raw,
    }
}

fn run_fixture(name: &str) -> FixtureRun {
    let mut app = App::new(State::project());
    let chat_id = app.chat_id().to_string();
    let mut tracker = ChatSeqTracker::new();
    for raw in fixture_events(name) {
        let event = chat_event_from_fixture(raw, &chat_id);
        match tracker.observe(&event) {
            ChatSeqDecision::Apply => {
                app.apply_chat_event(event);
            }
            ChatSeqDecision::Resubscribe(message) => {
                return FixtureRun {
                    app,
                    recovery: Some(message),
                };
            }
        }
    }
    FixtureRun {
        app,
        recovery: None,
    }
}

fn run_fixture_with_recovery_snapshots(name: &str, native_scrollback: bool) -> FixtureRun {
    let mut app = App::new(State::project());
    app.set_native_scrollback(native_scrollback);
    let chat_id = app.chat_id().to_string();
    let mut tracker = ChatSeqTracker::new();
    let mut recovery = None;
    for raw in fixture_events(name) {
        let event = chat_event_from_fixture(raw, &chat_id);
        match tracker.observe(&event) {
            ChatSeqDecision::Apply => {
                app.apply_chat_event(event);
            }
            ChatSeqDecision::Resubscribe(message) => {
                recovery.get_or_insert(message);
                tracker.reset();
            }
        }
    }
    FixtureRun { app, recovery }
}

fn transcript_text(app: &App) -> String {
    app.visible_transcript()
        .iter()
        .map(|item| match item {
            TranscriptItem::User(text) => format!("user:{text}"),
            TranscriptItem::Assistant(text) => format!("assistant:{text}"),
            TranscriptItem::Reasoning(text, _) => format!("reasoning:{text}"),
            TranscriptItem::Tool(card) => format!("tool:{}:{}:{}", card.id, card.name, card.result),
            TranscriptItem::Plan(plan) => {
                format!("plan:{}:{}:{}", plan.mode, plan.version, plan.content)
            }
            TranscriptItem::Citation(text) => format!("citation:{text}"),
            TranscriptItem::ServerContentBlock(text) => format!("server:{text}"),
            TranscriptItem::Diff(text) => format!("diff:{text}"),
            TranscriptItem::Notice(text) => format!("notice:{text}"),
            TranscriptItem::Info(lines) => format!("info:{}", lines.join("|")),
            TranscriptItem::Approval(_, outcome) => format!("approval:{outcome:?}"),
            TranscriptItem::Session { title, subtitle } => {
                format!(
                    "session:{title}:{}",
                    subtitle.as_deref().unwrap_or_default()
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rendered_snapshot(app: &App, width: u16, height: u16) -> String {
    let mut app = app.clone();
    app.set_native_scrollback(false);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| refact_tui::ui::render(frame, &mut app))
        .unwrap();
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

fn drain_stream(app: &mut App) {
    while app.stream_has_committable_lines() {
        app.apply_stream_commit_tick();
    }
}

#[test]
fn fixture_directory_covers_required_protocol_cases() {
    let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let mut names = std::fs::read_dir(&fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        vec![
            "approvals.jsonl",
            "assistant_message_added_dedup.jsonl",
            "assistant_streaming.jsonl",
            "citations.jsonl",
            "extra_updates.jsonl",
            "protocol_mutations.jsonl",
            "reasoning.jsonl",
            "seq_gap.jsonl",
            "server_content_blocks.jsonl",
            "snapshot_recovery_content.jsonl",
            "snapshot_resume.jsonl",
            "subchat_turn_cleanup.jsonl",
            "thinking_blocks.jsonl",
            "tool_calls.jsonl",
            "unknown_delta_ops.jsonl",
            "usage_updates.jsonl",
        ]
    );
}

#[test]
fn golden_fixtures_drive_app_state_machine_offline() {
    let streaming = run_fixture("assistant_streaming.jsonl");
    assert!(streaming.recovery.is_none());
    let text = transcript_text(&streaming.app);
    assert!(text.contains("user:render a table"));
    assert!(text.contains("| one | two |"));
    assert_eq!(streaming.app.usage().unwrap().total_tokens, 30);
    assert_eq!(streaming.app.session_state(), SessionState::Idle);

    let reasoning = run_fixture("reasoning.jsonl");
    assert!(transcript_text(&reasoning.app).contains("reasoning:Plan: inspect files. Then edit."));

    let tools = run_fixture("tool_calls.jsonl");
    let tool_text = transcript_text(&tools.app);
    assert!(tool_text.contains("tool:call-1:shell:+hi"));
    assert!(tool_text.contains("-no"));

    let approvals = run_fixture("approvals.jsonl");
    assert_eq!(approvals.app.session_state(), SessionState::Paused);
    assert_eq!(
        approvals.app.approval_modal().unwrap().reasons()[0].tool_call_id,
        "call-approve"
    );

    let usage = run_fixture("usage_updates.jsonl");
    assert_eq!(usage.app.usage().unwrap().total_tokens, 27);
    assert_eq!(
        usage.app.transcript_state().usage().unwrap()["total_tokens"],
        27
    );

    let citations = run_fixture("citations.jsonl");
    let citations_state = citations.app.transcript_state();
    assert!(transcript_text(&citations.app).contains("citation:"));
    assert_eq!(
        citations_state.messages()[0].citations[0]["title"],
        "README"
    );

    let thinking = run_fixture("thinking_blocks.jsonl");
    let thinking_blocks = &thinking.app.transcript_state().messages()[0].thinking_blocks;
    assert_eq!(thinking_blocks[0]["signature"], "sig-demo");

    let server = run_fixture("server_content_blocks.jsonl");
    assert!(transcript_text(&server.app).contains("server:"));
    assert_eq!(
        server.app.transcript_state().messages()[0].server_content_blocks[0]["type"],
        "web_search_call"
    );

    let extra = run_fixture("extra_updates.jsonl");
    let extra_message = &extra.app.transcript_state().messages()[0];
    assert_eq!(extra_message.extra["metering_a"], 2);
    assert_eq!(extra_message.extra["metering_b"], "kept");
    assert_eq!(extra_message.extra["nested"]["ok"], true);

    let unknown = run_fixture("unknown_delta_ops.jsonl");
    assert!(unknown.recovery.is_none());
    assert!(transcript_text(&unknown.app).contains("before unknown"));
    assert!(transcript_text(&unknown.app).contains("after unknown"));
    assert_eq!(unknown.app.transcript_state().unknown_delta_ops().len(), 2);
    assert_eq!(
        unknown.app.transcript_state().unknown_delta_ops()[0]
            .op
            .as_deref(),
        Some("future_delta")
    );

    let resumed = run_fixture("snapshot_resume.jsonl");
    let resumed_text = transcript_text(&resumed.app);
    assert!(resumed_text.contains("session:Saved chat:"));
    assert!(resumed_text.contains("user:resume"));
    assert!(resumed_text.contains("assistant:old answer"));
    assert!(resumed_text.contains("tool:resume-call:cat:file body"));
    assert_eq!(resumed.app.session_state(), SessionState::Idle);
}

#[test]
fn mutation_fixtures_update_thread_and_transcript_state() {
    let run = run_fixture("protocol_mutations.jsonl");
    assert!(run.recovery.is_none());

    assert_eq!(run.app.session_title(), Some("Renamed"));
    assert_eq!(run.app.model(), Some("gpt-new"));
    assert_eq!(run.app.mode(), Some("ask"));
    assert_eq!(run.app.reasoning_effort_label(), "high");
    assert!(run.app.permission_policy().auto_approve_editing_tools);

    let text = transcript_text(&run.app);
    assert!(text.contains("session:Renamed:"));
    assert!(text.contains("user:first"));
    assert!(text.contains("assistant:edited answer"));
    assert!(!text.contains("old answer"));
    assert!(!text.contains("remove me"));
    assert!(!text.contains("truncate me"));
    assert_eq!(run.app.transcript_state().messages().len(), 2);
}

#[test]
fn persisted_assistant_message_added_dedups_streamed_turn() {
    let run = run_fixture("assistant_message_added_dedup.jsonl");
    assert!(run.recovery.is_none());

    let text = transcript_text(&run.app);
    assert_eq!(text.matches("assistant:hello once").count(), 1);
    assert_eq!(run.app.transcript_state().messages().len(), 1);
}

#[test]
fn subchat_running_card_is_cleaned_at_turn_end() {
    let run = run_fixture("subchat_turn_cleanup.jsonl");
    assert!(run.recovery.is_none());

    let cards = run
        .app
        .visible_transcript()
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::Tool(card) => Some(card),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].id, "call-sub");
    assert_eq!(cards[0].status, refact_tui::tools::ToolStatus::Success);
    assert!(!cards[0].subchat_active);
    assert_eq!(cards[0].attached_files, vec!["src/lib.rs".to_string()]);
}

#[test]
fn seq_gap_fixture_requests_resubscribe_without_applying_gap_delta() {
    let run = run_fixture("seq_gap.jsonl");
    assert!(run.recovery.unwrap().contains("expected 2, got 3"));
    assert!(!transcript_text(&run.app).contains("must not apply"));
}

#[test]
fn snapshot_recovery_updates_stable_message_id_without_duplicates() {
    let mut run = run_fixture_with_recovery_snapshots("snapshot_recovery_content.jsonl", true);
    assert!(run.recovery.unwrap().contains("expected 4, got 5"));
    let insertions = run.app.pending_history_insertions(80);
    let inserted_text = insertions
        .iter()
        .flat_map(|insertion| insertion.lines.iter())
        .map(|line| {
            line.line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(inserted_text.contains("snapshot corrected"));
    assert!(!inserted_text.contains("partial stale"));
    assert!(!inserted_text.contains("must not apply"));
    assert_eq!(
        insertions
            .iter()
            .map(|insertion| insertion.cell_ids.len())
            .sum::<usize>(),
        1
    );
    assert!(run.app.pending_history_insertions(80).is_empty());
}

#[test]
fn streaming_table_rows_hold_until_block_completes() {
    let mut app = App::new(State::project());
    let chat_id = app.chat_id().to_string();
    app.apply_chat_event(chat_event_from_fixture(
        json!({"chat_id": chat_id, "seq": "1", "type": "stream_started", "message_id": "assistant-1"}),
        &chat_id,
    ));
    app.apply_chat_event(chat_event_from_fixture(
        json!({"chat_id": chat_id, "seq": "2", "type": "stream_delta", "message_id": "assistant-1", "ops": [{"op": "append_content", "text": "| A | B |\n"}]}),
        &chat_id,
    ));
    drain_stream(&mut app);
    assert_eq!(app.active_stream_committed(), "");
    assert_eq!(app.active_stream_live(), "| A | B |\n");
    assert!(transcript_text(&app).contains("assistant:| A | B |"));
    assert!(!transcript_text(&app).contains("| one | two |"));

    app.apply_chat_event(chat_event_from_fixture(
        json!({"chat_id": chat_id, "seq": "3", "type": "stream_delta", "message_id": "assistant-1", "ops": [{"op": "append_content", "text": "| --- | --- |\n| one | two |\n"}]}),
        &chat_id,
    ));
    drain_stream(&mut app);
    let text = transcript_text(&app);
    assert_eq!(app.active_stream_committed(), "");
    assert!(app.active_stream_live().contains("| one | two |"));
    assert!(text.contains("| one | two |"));
    assert!(!text.contains("assistant:| A | B |\nassistant:| one | two |"));

    app.apply_chat_event(chat_event_from_fixture(
        json!({"chat_id": chat_id, "seq": "4", "type": "stream_delta", "message_id": "assistant-1", "ops": [{"op": "append_content", "text": "\nAfter table.\n"}]}),
        &chat_id,
    ));
    drain_stream(&mut app);
    assert!(app.active_stream_committed().contains("| one | two |"));
    assert!(app.active_stream_committed().contains("After table."));
    assert!(transcript_text(&app).contains("After table."));
}

#[test]
fn streaming_code_fence_splits_and_unicode_boundaries_finalize_correctly() {
    let mut app = App::new(State::project());
    let chat_id = app.chat_id().to_string();
    let deltas = ["```rust\nfn", " main() { println!(\"🦀\"); }\n`", "``\n"];
    app.apply_chat_event(chat_event_from_fixture(
        json!({"chat_id": chat_id, "seq": "1", "type": "stream_started", "message_id": "assistant-1"}),
        &chat_id,
    ));
    for (idx, delta) in deltas.iter().enumerate() {
        app.apply_chat_event(chat_event_from_fixture(
            json!({"chat_id": chat_id, "seq": (idx + 2).to_string(), "type": "stream_delta", "message_id": "assistant-1", "ops": [{"op": "append_content", "text": delta}]}),
            &chat_id,
        ));
        app.apply_stream_commit_tick();
    }
    app.apply_chat_event(chat_event_from_fixture(
        json!({"chat_id": chat_id, "seq": "5", "type": "stream_finished", "message_id": "assistant-1"}),
        &chat_id,
    ));
    assert!(transcript_text(&app).contains("```rust\nfn main() { println!(\"🦀\"); }\n```\n"));
}

#[test]
fn streaming_final_output_matches_fixture_source_after_ticks() {
    let source = "| A | B |\n|---|---|\n| one | two |\n\n```rust\nfn main() {}\n```\n";
    let mut app = run_fixture("assistant_streaming.jsonl").app;
    drain_stream(&mut app);
    let text = transcript_text(&app);
    assert!(text.contains(source));
    assert_eq!(app.transcript_state().messages()[1].content, source);
}

#[test]
fn render_snapshot_for_assistant_streaming_fixture() {
    let run = run_fixture("assistant_streaming.jsonl");
    let snapshot = rendered_snapshot(&run.app, 72, 16);
    let expected = r#"refact fixture  Ctrl-N new · Ctrl-P projects · Ctrl-M model · Ctrl-O mod
you
render a table

assistant
A   │ B
━━━━━━━━━
one │ two
code · rust
  fn main() {}

────────────────────────────────────────────────────────────────────────
┌ message ─────────────────────────────────────────────────────────────┐
│Ask Refact…                                                           │
└──────────────────────────────────────────────────────────────────────┘
 30 used · fixture · gpt-demo · agent · reason:off · ● idle · daemon on…"#;
    assert_eq!(snapshot, expected);
}

#[tokio::test]
async fn scripted_fake_worker_pause_approve_resumes_stream() {
    let state = State::default();
    let base_url = spawn_server(state).await;
    let client = DaemonClient::new(base_url, None).unwrap();
    let mut stream = client.subscribe_chat("p1", "chat-1").await.unwrap();

    client
        .send_user_message("p1", "chat-1", "hello")
        .await
        .unwrap();

    let mut saw_pause = false;
    while let Some(event) = futures::StreamExt::next(&mut stream).await {
        let event = event.unwrap();
        if event.kind == "pause_required" {
            saw_pause = true;
            break;
        }
    }
    assert!(saw_pause);

    client
        .send_tool_decisions(
            "p1",
            "chat-1",
            vec![ToolDecision {
                tool_call_id: "fake-call-1".to_string(),
                accepted: true,
            }],
        )
        .await
        .unwrap();

    let mut content = String::new();
    while let Some(event) = futures::StreamExt::next(&mut stream).await {
        let event = event.unwrap();
        if event.kind == "stream_delta" {
            for op in event.raw["ops"].as_array().unwrap() {
                if op["op"] == "append_content" {
                    content.push_str(op["text"].as_str().unwrap());
                }
            }
        }
        if event.kind == "stream_finished" {
            break;
        }
    }
    assert_eq!(content, "approved path");
}

#[tokio::test]
async fn model_modes_and_events_protocol_paths_work() {
    let state = State::default();
    let base_url = spawn_server(state).await;
    let client = DaemonClient::new(base_url, None).unwrap();

    let caps = client.get_caps("p1").await.unwrap();
    assert_eq!(caps["chat_models"]["m1"]["name"], "Model One");

    let modes = client.get_chat_modes("p1").await.unwrap();
    assert_eq!(modes["modes"][0]["id"], "agent");

    let workers = client.list_workers().await.unwrap();
    assert_eq!(workers[0].project_id, "p1");

    let mut events = client.subscribe_daemon_events().await.unwrap();
    let event = futures::StreamExt::next(&mut events)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.kind, "worker_ready");
}

#[tokio::test]
async fn daemon_info_env_override_selects_custom_port_and_token() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("daemon.json"),
        r#"{"pid":7,"port":43123,"bind":"0.0.0.0","version":"9.9.9","auth_token":"secret-token"}"#,
    )
    .unwrap();
    let _guard = EnvGuard::set_daemon_dir(dir.path());

    let endpoint = discover_daemon_endpoint().unwrap().unwrap();

    assert_eq!(endpoint.base_url, "http://127.0.0.1:43123");
    assert_eq!(endpoint.auth_token.as_deref(), Some("secret-token"));
}

#[tokio::test]
async fn explicit_url_override_preserves_discovered_token() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("daemon.json"),
        r#"{"pid":7,"port":43123,"bind":"127.0.0.1","version":"9.9.9","auth_token":"secret-token"}"#,
    )
    .unwrap();
    let _guard = EnvGuard::set_daemon_dir(dir.path());

    let endpoint = resolve_daemon_endpoint(Some("http://127.0.0.1:45454".to_string())).unwrap();

    assert_eq!(endpoint.base_url, "http://127.0.0.1:45454");
    assert_eq!(endpoint.auth_token.as_deref(), Some("secret-token"));
}

#[tokio::test]
async fn daemon_client_sends_bearer_header() {
    let state = State::default();
    let base_url = spawn_server(state.clone()).await;
    let client = DaemonClient::new(base_url, Some("secret-token".to_string())).unwrap();

    let caps = client.get_caps("p1").await.unwrap();

    assert_eq!(caps["chat_models"]["m1"]["name"], "Model One");
    assert!(state.wait_for_authorization("Bearer secret-token"));
}

#[tokio::test]
async fn corrupt_daemon_info_surfaces_visible_notice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.json");
    std::fs::write(&path, "not json").unwrap();

    let warning = discover_daemon_endpoint_from(&path).unwrap_err();

    let notice = warning.notice();
    assert!(notice.contains("Failed to read daemon info"));
    assert!(notice.contains("daemon.json"));
    assert!(notice.contains("expected ident"));
}

#[tokio::test]
async fn fake_stream_seq_gap_triggers_recovery_without_applying_gap_delta() {
    let state = State::with_chat_script(vec![
        json!({"chat_id": "chat-1", "seq": "0", "type": "snapshot", "thread": {"id": "chat-1", "model": "", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        json!({"chat_id": "chat-1", "seq": "1", "type": "stream_started"}),
        json!({"chat_id": "chat-1", "seq": "2", "type": "stream_delta", "ops": [{"op": "append_content", "text": "kept"}]}),
        json!({"chat_id": "chat-1", "seq": "4", "type": "stream_delta", "ops": [{"op": "append_content", "text": "dropped"}]}),
    ]);
    let base_url = spawn_server(state).await;
    let client = DaemonClient::new(base_url, None).unwrap();
    let mut stream = client.subscribe_chat("p1", "chat-1").await.unwrap();
    let mut tracker = ChatSeqTracker::new();
    let mut content = String::new();
    let mut recovery = None;

    while let Some(event) = futures::StreamExt::next(&mut stream).await {
        let event = event.unwrap();
        match tracker.observe(&event) {
            ChatSeqDecision::Apply => {
                if event.kind == "stream_delta" {
                    for op in event.raw["ops"].as_array().unwrap() {
                        if op["op"] == "append_content" {
                            content.push_str(op["text"].as_str().unwrap());
                        }
                    }
                }
            }
            ChatSeqDecision::Resubscribe(message) => {
                recovery = Some(message);
                break;
            }
        }
    }

    assert_eq!(content, "kept");
    assert!(recovery.unwrap().contains("expected 3, got 4"));
}

#[tokio::test]
async fn duplicate_seq_triggers_recovery_without_duplicate_content() {
    let state = State::with_chat_script(vec![
        json!({"chat_id": "chat-1", "seq": "0", "type": "snapshot", "thread": {"id": "chat-1", "model": "", "mode": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
        json!({"chat_id": "chat-1", "seq": "1", "type": "stream_started"}),
        json!({"chat_id": "chat-1", "seq": "2", "type": "stream_delta", "ops": [{"op": "append_content", "text": "once"}]}),
        json!({"chat_id": "chat-1", "seq": "2", "type": "stream_delta", "ops": [{"op": "append_content", "text": "twice"}]}),
    ]);
    let base_url = spawn_server(state).await;
    let client = DaemonClient::new(base_url, None).unwrap();
    let mut stream = client.subscribe_chat("p1", "chat-1").await.unwrap();
    let mut tracker = ChatSeqTracker::new();
    let mut content = String::new();
    let mut recovery = None;

    while let Some(event) = futures::StreamExt::next(&mut stream).await {
        let event = event.unwrap();
        match tracker.observe(&event) {
            ChatSeqDecision::Apply => {
                if event.kind == "stream_delta" {
                    for op in event.raw["ops"].as_array().unwrap() {
                        if op["op"] == "append_content" {
                            content.push_str(op["text"].as_str().unwrap());
                        }
                    }
                }
            }
            ChatSeqDecision::Resubscribe(message) => {
                recovery = Some(message);
                break;
            }
        }
    }

    assert_eq!(content, "once");
    assert!(recovery.unwrap().contains("expected 3, got 2"));
}

async fn spawn_server(state: State) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            thread::spawn(move || handle_connection(stream, state));
        }
    });
    format!("http://{addr}")
}

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    daemon_dir: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set_daemon_dir(path: &std::path::Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap();
        let guard = Self {
            _lock: lock,
            daemon_dir: std::env::var_os("REFACT_DAEMON_DIR"),
        };
        std::env::set_var("REFACT_DAEMON_DIR", path);
        guard
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.daemon_dir {
            std::env::set_var("REFACT_DAEMON_DIR", value);
        } else {
            std::env::remove_var("REFACT_DAEMON_DIR");
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: State) {
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
    let header_end = data
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
        .unwrap();
    let headers = String::from_utf8_lossy(&data[..header_end]).to_string();
    state.record_authorization(header_value(&headers, "authorization"));
    let mut first = headers
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = first.next().unwrap_or_default().to_string();
    let path = first.next().unwrap_or_default().to_string();
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
    let body = &data[header_end..header_end + content_length];

    if method == "POST" && path.contains("/commands") {
        let command = serde_json::from_slice(body).unwrap_or(Value::Null);
        state.record_command(command);
        write_json(&mut stream, json!({"status": "accepted"}));
    } else if method == "GET" && path.contains("/chats/subscribe") {
        if let Some(events) = state.chat_script() {
            write_chat_script_sse(&mut stream, &events);
        } else {
            write_chat_sse(&mut stream, &state);
        }
    } else if method == "GET" && path.ends_with("/v1/caps") {
        write_json(
            &mut stream,
            json!({"chat_models": {"m1": {"name": "Model One", "supports_tools": true}}}),
        );
    } else if method == "GET" && path.ends_with("/v1/chat-modes") {
        write_json(
            &mut stream,
            json!({"modes": [{"id": "agent", "title": "Agent", "description": "Act", "tools_count": 1, "thread_defaults": {}, "ui": {"order": 1, "tags": []}}], "errors": []}),
        );
    } else if method == "GET" && path.starts_with("/daemon/v1/events") {
        write_sse_event(
            &mut stream,
            &json!({"ts_ms": 1, "kind": "worker_ready", "project_id": "p1", "payload": {"pid": 7}}),
        );
    } else if method == "GET" && path == "/daemon/v1/workers" {
        write_json(
            &mut stream,
            json!([{"project_id": "p1", "pid": 7, "http_port": 31000, "lsp_port": 31001, "state": "ready", "last_error": null}]),
        );
    } else {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
    }
}

fn write_json(stream: &mut TcpStream, value: Value) {
    let body = value.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(), body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn write_sse_headers(stream: &mut TcpStream) {
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
    );
}

fn write_sse_event(stream: &mut TcpStream, value: &Value) {
    write_sse_headers(stream);
    write_sse_data(stream, value);
}

fn write_chat_script_sse(stream: &mut TcpStream, events: &[Value]) {
    write_sse_headers(stream);
    for event in events {
        write_sse_data(stream, event);
    }
}

fn write_chat_sse(stream: &mut TcpStream, state: &State) {
    write_sse_headers(stream);
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "0", "type": "snapshot", "thread": {"id": "chat-1", "model": "", "mode": "agent", "tool_use": "agent"}, "runtime": {"state": "idle"}, "messages": []}),
    );
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "1", "type": "pause_required", "reasons": [{"type": "confirmation", "tool_name": "fake_tool", "command": "fake_tool({\"x\":1})", "rule": "fake", "tool_call_id": "fake-call-1"}]}),
    );
    assert!(state.wait_for_tool_decisions());
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "2", "type": "pause_cleared"}),
    );
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "3", "type": "stream_started", "message_id": "assistant-1"}),
    );
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "4", "type": "stream_delta", "message_id": "assistant-1", "ops": [{"op": "append_content", "text": "approved "}]}),
    );
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "5", "type": "stream_delta", "message_id": "assistant-1", "ops": [{"op": "append_content", "text": "path"}]}),
    );
    write_sse_data(
        stream,
        &json!({"chat_id": "chat-1", "seq": "6", "type": "stream_finished", "message_id": "assistant-1"}),
    );
}

fn write_sse_data(stream: &mut TcpStream, value: &Value) {
    let _ = stream.write_all(format!("data: {}\n\n", value).as_bytes());
    let _ = stream.flush();
}
