use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

type ServerKey = (String, Vec<(String, Vec<u8>)>);
type ServerState = Arc<Mutex<Option<Session>>>;

static SERVERS: OnceLock<Mutex<HashMap<ServerKey, ServerState>>> = OnceLock::new();
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct Session {
    id: Option<HeaderValue>,
    protocol: HeaderValue,
}

struct Failure {
    session: bool,
    message: &'static str,
}

impl Failure {
    fn transport(message: &'static str) -> Self {
        Self {
            session: false,
            message,
        }
    }
}

pub(super) async fn call(
    endpoint: &str,
    headers: HeaderMap,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    let result = request(
        endpoint,
        headers,
        "tools/call",
        json!({"name": tool, "arguments": args}),
    )
    .await?;
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err("MCP tool reported an error".to_owned());
    }
    if let Some(content) = result.get("structuredContent") {
        return Ok(content.clone());
    }
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn request(
    endpoint: &str,
    headers: HeaderMap,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let mut identity = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    identity.sort();
    let state = {
        let mut servers = SERVERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .await;
        servers
            .entry((endpoint.to_owned(), identity))
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    };
    for attempt in 0..2 {
        let session = {
            let mut cached = state.lock().await;
            if cached.is_none() {
                *cached = Some(
                    initialize(endpoint, &headers)
                        .await
                        .map_err(|err| err.message.to_owned())?,
                );
            }
            cached.clone()
        };
        let result = send(
            endpoint,
            &headers,
            session.as_ref(),
            method,
            params.clone(),
            false,
        )
        .await;
        match result {
            Ok((result, _)) => return Ok(result),
            Err(err) if err.session => {
                let mut cached = state.lock().await;
                if cached.as_ref().map(|s| &s.id) == session.as_ref().map(|s| &s.id) {
                    *cached = None;
                }
                if attempt == 1 {
                    return Err(err.message.to_owned());
                }
            }
            Err(err) => return Err(err.message.to_owned()),
        }
    }
    Err("MCP session recovery failed".to_owned())
}

async fn initialize(endpoint: &str, headers: &HeaderMap) -> Result<Session, Failure> {
    let (result, id) = send(
        endpoint,
        headers,
        None,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "refact", "version": env!("CARGO_PKG_VERSION")}
        }),
        false,
    )
    .await?;
    let protocol = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::transport("MCP initialization omitted protocol version"))?;
    if !matches!(protocol, "2024-11-05" | "2025-03-26" | "2025-06-18") {
        return Err(Failure::transport(
            "MCP server selected an unsupported protocol version",
        ));
    }
    let session = Session {
        id,
        protocol: HeaderValue::from_str(protocol)
            .map_err(|_| Failure::transport("MCP server returned an invalid protocol version"))?,
    };
    send(
        endpoint,
        headers,
        Some(&session),
        "notifications/initialized",
        json!({}),
        true,
    )
    .await?;
    Ok(session)
}

async fn send(
    endpoint: &str,
    headers: &HeaderMap,
    session: Option<&Session>,
    method: &str,
    params: Value,
    notification: bool,
) -> Result<(Value, Option<HeaderValue>), Failure> {
    let client =
        super::http::client().map_err(|_| Failure::transport("MCP HTTP client unavailable"))?;
    let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let mut body = json!({"jsonrpc": "2.0", "method": method, "params": params});
    if !notification {
        body["id"] = json!(id);
    }
    let mut request = client
        .post(endpoint)
        .headers(headers.clone())
        .header("Accept", "application/json, text/event-stream")
        .timeout(std::time::Duration::from_secs(60))
        .json(&body);
    if let Some(session) = session {
        request = request.header("MCP-Protocol-Version", session.protocol.clone());
        if let Some(id) = &session.id {
            request = request.header("Mcp-Session-Id", id.clone());
        }
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| Failure::transport("MCP HTTP request failed"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(Failure {
            session: session.is_some() && matches!(status.as_u16(), 404 | 410),
            message: "MCP server rejected the HTTP request",
        });
    }
    let session_id = response.headers().get("Mcp-Session-Id").cloned();
    if notification {
        return Ok((Value::Null, session_id));
    }
    let sse = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    let mut bytes = Vec::new();
    let mut data = Vec::new();
    let mut total = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Failure::transport("MCP response read failed"))?
    {
        total += chunk.len();
        if total > MAX_RESPONSE_BYTES {
            return Err(Failure::transport("MCP response exceeded size limit"));
        }
        bytes.extend_from_slice(&chunk);
        if sse {
            while let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
                let mut line = bytes.drain(..=end).collect::<Vec<_>>();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.is_empty() {
                    if !data.is_empty() {
                        if let Some(result) = decode(&data, id)? {
                            return Ok((result, session_id));
                        }
                        data.clear();
                    }
                } else if let Some(value) = line.strip_prefix(b"data:") {
                    if !data.is_empty() {
                        data.push(b'\n');
                    }
                    data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
                }
            }
        }
    }
    if sse {
        if let Some(value) = bytes.strip_prefix(b"data:") {
            if !data.is_empty() {
                data.push(b'\n');
            }
            data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
        }
        if !data.is_empty() {
            if let Some(result) = decode(&data, id)? {
                return Ok((result, session_id));
            }
        }
    } else if let Some(result) = decode(&bytes, id)? {
        return Ok((result, session_id));
    }
    Err(Failure::transport("MCP response omitted matching result"))
}

fn decode(bytes: &[u8], id: u64) -> Result<Option<Value>, Failure> {
    let envelope: Value = serde_json::from_slice(bytes)
        .map_err(|_| Failure::transport("MCP server returned invalid JSON"))?;
    if envelope.get("id") != Some(&json!(id)) {
        return Ok(None);
    }
    if let Some(error) = envelope.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        return Err(Failure {
            session: message.contains("session")
                || error.get("code").and_then(Value::as_i64) == Some(-32001),
            message: "MCP server returned a protocol error",
        });
    }
    envelope
        .get("result")
        .cloned()
        .map(Some)
        .ok_or_else(|| Failure::transport("MCP response omitted result"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    struct Step {
        method: &'static str,
        session: Option<&'static str>,
        issue_session: Option<&'static str>,
        result: Value,
        sse: bool,
        status: u16,
    }

    fn init(session: Option<&'static str>) -> Step {
        Step {
            method: "initialize",
            session: None,
            issue_session: session,
            result: json!({"protocolVersion": "2025-03-26", "capabilities": {}}),
            sse: false,
            status: 200,
        }
    }

    fn initialized(session: Option<&'static str>) -> Step {
        Step {
            method: "notifications/initialized",
            session,
            issue_session: None,
            result: Value::Null,
            sse: false,
            status: 202,
        }
    }

    fn tool(session: Option<&'static str>, result: Value) -> Step {
        Step {
            method: "tools/call",
            session,
            issue_session: None,
            result,
            sse: false,
            status: 200,
        }
    }

    async fn server(steps: Vec<Step>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let endpoint = format!(
            "http://{}/mcp/{}?key=never-echo-this",
            listener.local_addr().expect("server address"),
            REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        let task = tokio::spawn(async move {
            for step in steps {
                let (mut socket, _) = listener.accept().await.expect("accept request");
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.expect("read headers");
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec())
                    .expect("UTF-8 headers")
                    .to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                    .expect("content length")
                    .parse::<usize>()
                    .expect("numeric content length");
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.expect("read body");
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let body: Value = serde_json::from_slice(&bytes[header_end..header_end + length])
                    .expect("JSON request");
                assert_eq!(body["method"], step.method);
                let session = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("mcp-session-id:").map(str::trim));
                assert_eq!(session, step.session);
                if step.method == "notifications/initialized" {
                    assert!(body.get("id").is_none());
                }
                let envelope = if step.result.get("error").is_some() {
                    json!({"jsonrpc": "2.0", "id": body["id"], "error": step.result["error"]})
                } else {
                    json!({"jsonrpc": "2.0", "id": body["id"], "result": step.result})
                };
                let response_body = if step.status == 202 {
                    String::new()
                } else if step.sse {
                    format!("event: message\r\ndata: {}\r\n\r\nevent: message\r\ndata: {envelope}\r\n\r\n", json!({"jsonrpc":"2.0", "method":"notifications/progress", "params":{}}))
                } else {
                    envelope.to_string()
                };
                let session_header = step
                    .issue_session
                    .map(|id| format!("Mcp-Session-Id: {id}\r\n"))
                    .unwrap_or_default();
                let content_type = if step.sse {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!("HTTP/1.1 {} OK\r\nContent-Type: {content_type}\r\n{session_header}Content-Length: {}\r\nConnection: close\r\n\r\n{response_body}", step.status, response_body.len());
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
        });
        (endpoint, task)
    }

    async fn finish(task: JoinHandle<()>) {
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("server completes")
            .expect("server assertions pass");
    }

    #[tokio::test]
    async fn json_session_cached_and_structured_content_preferred() {
        let (endpoint, task) = server(vec![
            init(Some("session-a")),
            initialized(Some("session-a")),
            tool(
                Some("session-a"),
                json!({"structuredContent":{"hits":[1]}, "content":[{"text":"ignored"}]}),
            ),
            tool(
                Some("session-a"),
                json!({"content":[{"text":"one"},{"type":"image"},{"text":"two"}]}),
            ),
        ])
        .await;
        assert_eq!(
            call(&endpoint, HeaderMap::new(), "search", json!({}))
                .await
                .expect("first call"),
            json!({"hits":[1]})
        );
        assert_eq!(
            call(&endpoint, HeaderMap::new(), "search", json!({}))
                .await
                .expect("cached call"),
            json!("one\ntwo")
        );
        finish(task).await;
    }

    #[tokio::test]
    async fn stateless_sse_parses_json_text() {
        let mut initialization = init(None);
        initialization.sse = true;
        let mut result = tool(None, json!({"content":[{"text":"{\"hits\":[]}"}]}));
        result.sse = true;
        let (endpoint, task) = server(vec![initialization, initialized(None), result]).await;
        assert_eq!(
            call(&endpoint, HeaderMap::new(), "search", json!({}))
                .await
                .expect("SSE call"),
            json!({"hits":[]})
        );
        finish(task).await;
    }

    #[tokio::test]
    async fn expired_session_reinitializes_once() {
        let mut expired = tool(Some("old"), Value::Null);
        expired.status = 404;
        let (endpoint, task) = server(vec![
            init(Some("old")),
            initialized(Some("old")),
            expired,
            init(Some("new")),
            initialized(Some("new")),
            tool(Some("new"), json!({"content":[{"text":"recovered"}]})),
        ])
        .await;
        assert_eq!(
            call(&endpoint, HeaderMap::new(), "search", json!({}))
                .await
                .expect("recovery"),
            json!("recovered")
        );
        finish(task).await;
    }

    #[tokio::test]
    async fn protocol_session_error_retries_only_once() {
        let error = json!({"error":{"code":-32001,"message":"Invalid session never-echo-this"}});
        let (endpoint, task) = server(vec![
            init(Some("old")),
            initialized(Some("old")),
            tool(Some("old"), error.clone()),
            init(Some("new")),
            initialized(Some("new")),
            tool(Some("new"), error),
        ])
        .await;
        let error = call(&endpoint, HeaderMap::new(), "search", json!({}))
            .await
            .expect_err("second error stops retry");
        assert_eq!(error, "MCP server returned a protocol error");
        assert!(!error.contains("never-echo-this"));
        finish(task).await;
    }

    #[tokio::test]
    async fn different_authentication_uses_separate_initialization() {
        let (endpoint, task) = server(vec![
            init(Some("auth-a")),
            initialized(Some("auth-a")),
            tool(Some("auth-a"), json!({"content":[{"text":"first"}]})),
            init(Some("auth-b")),
            initialized(Some("auth-b")),
            tool(Some("auth-b"), json!({"content":[{"text":"second"}]})),
        ])
        .await;
        for token in ["Bearer first", "Bearer second"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                HeaderValue::from_str(token).expect("valid test header"),
            );
            call(&endpoint, headers, "search", json!({}))
                .await
                .expect("authenticated call");
        }
        finish(task).await;
    }

    #[tokio::test]
    async fn concurrent_calls_initialize_only_once() {
        let (endpoint, task) = server(vec![
            init(Some("shared")),
            initialized(Some("shared")),
            tool(Some("shared"), json!({"content":[{"text":"ok"}]})),
            tool(Some("shared"), json!({"content":[{"text":"ok"}]})),
        ])
        .await;
        let (first, second) = tokio::join!(
            call(&endpoint, HeaderMap::new(), "search", json!({})),
            call(&endpoint, HeaderMap::new(), "search", json!({})),
        );
        assert_eq!(first.expect("first concurrent call"), json!("ok"));
        assert_eq!(second.expect("second concurrent call"), json!("ok"));
        finish(task).await;
    }

    #[tokio::test]
    async fn tool_errors_are_not_returned_or_retried() {
        let (endpoint, task) = server(vec![init(None), initialized(None), tool(None, json!({"isError":true, "structuredContent":{"secret":"never-echo-this"}, "content":[{"text":"never-echo-this"}]}))]).await;
        assert_eq!(
            call(&endpoint, HeaderMap::new(), "search", json!({}))
                .await
                .expect_err("tool error"),
            "MCP tool reported an error"
        );
        finish(task).await;
    }
}
