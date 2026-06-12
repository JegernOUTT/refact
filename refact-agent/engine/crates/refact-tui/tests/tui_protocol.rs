use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use refact_tui::client::{
    discover_daemon_endpoint, discover_daemon_endpoint_from, resolve_daemon_endpoint, DaemonClient,
    ChatSeqDecision, ChatSeqTracker, ToolDecision,
};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct State {
    commands: Arc<(Mutex<Vec<Value>>, Condvar)>,
    authorizations: Arc<(Mutex<Vec<Option<String>>>, Condvar)>,
    chat_script: Arc<Mutex<Option<Vec<Value>>>>,
}

impl State {
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
    daemon_dir: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set_daemon_dir(path: &std::path::Path) -> Self {
        let guard = Self {
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
