//! Minimal stdio MCP server used by the e2e test suite (`tests/mcp_stdio_e2e.rs`).
//!
//! Speaks newline-delimited JSON-RPC over stdin/stdout and implements just
//! enough of the 2024-11-05 MCP revision to exercise the client stack:
//! `initialize`, `tools/list`, `tools/call` (echo), `prompts/list`,
//! `resources/list`, `resources/read`, and `ping`. Not shipped to users.
use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn handle_request(method: &str, params: &Value, id: Value) -> Value {
    match method {
        "initialize" => response(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "prompts": {},
                    "resources": {}
                },
                "serverInfo": { "name": "refact-mcp-echo-server", "version": "1.0.0" }
            }),
        ),
        "ping" => response(id, json!({})),
        "tools/list" => response(
            id,
            json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echoes the input text back",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "text": { "type": "string" } },
                        "required": ["text"]
                    }
                }]
            }),
        ),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if tool_name != "echo" {
                return error_response(id, -32602, "unknown tool");
            }
            let text = params
                .get("arguments")
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            response(
                id,
                json!({
                    "content": [{ "type": "text", "text": format!("echo: {}", text) }],
                    "isError": false
                }),
            )
        }
        "prompts/list" => response(
            id,
            json!({
                "prompts": [{
                    "name": "greet",
                    "description": "A test prompt",
                    "arguments": [{ "name": "name", "required": true }]
                }]
            }),
        ),
        "resources/list" => response(
            id,
            json!({
                "resources": [{
                    "uri": "echo://readme",
                    "name": "readme",
                    "mimeType": "text/plain"
                }]
            }),
        ),
        "resources/read" => response(
            id,
            json!({
                "contents": [{
                    "uri": "echo://readme",
                    "mimeType": "text/plain",
                    "text": "echo server readme"
                }]
            }),
        ),
        _ => error_response(id, -32601, "method not found"),
    }
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(_) => continue,
        };
        let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
        match message.get("id") {
            Some(id) if !id.is_null() => {
                let params = message.get("params").cloned().unwrap_or(json!({}));
                let reply = handle_request(method, &params, id.clone());
                let mut out = stdout.lock();
                if serde_json::to_writer(&mut out, &reply).is_ok() {
                    let _ = out.write_all(b"\n");
                    let _ = out.flush();
                }
            }
            _ => {
                // Notifications (e.g. notifications/initialized) need no reply.
            }
        }
    }
}
