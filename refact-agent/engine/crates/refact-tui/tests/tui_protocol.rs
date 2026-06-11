use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use refact_tui::client::{DaemonClient, ToolDecision};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct State {
    commands: Arc<(Mutex<Vec<Value>>, Condvar)>,
}

impl State {
    fn record_command(&self, command: Value) {
        let (lock, cond) = &*self.commands;
        lock.lock().unwrap().push(command);
        cond.notify_all();
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
        write_chat_sse(&mut stream, &state);
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

fn write_sse_headers(stream: &mut TcpStream) {
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
    );
}

fn write_sse_event(stream: &mut TcpStream, value: &Value) {
    write_sse_headers(stream);
    write_sse_data(stream, value);
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
