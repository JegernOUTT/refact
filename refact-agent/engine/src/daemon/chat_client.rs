//! Proxy-only chat protocol client used by daemon frontends.
//!
//! The engine chat API is driven through `/p/{project_id}/v1/*` on the daemon proxy.
//! A caller opens `GET /v1/chats/subscribe?chat_id=<uuid>` first; the worker replies with SSE
//! `data:` JSON envelopes shaped as `{chat_id, seq: "N", type: "snapshot" | ...}`. Commands are
//! flattened JSON posts to `POST /v1/chats/{chat_id}/commands` with `client_request_id`, optional
//! `priority`, and a command `type`. The shapes used here are:
//!
//! - `{"type":"set_params","patch":{"mode":"agent","tool_use":"agent","model":"..."}}`
//! - `{"type":"user_message","content":"..."}`
//! - `{"type":"tool_decisions","decisions":[{"tool_call_id":"...","accepted":true}]}`
//! - `{"type":"abort"}`
//!
//! Streaming text arrives as `stream_delta` operations with
//! `ops: [{"op":"append_content","text":"..."}]`. Tool calls are surfaced by
//! `set_tool_calls`; confirmation pauses arrive as `pause_required` with `reasons` containing
//! `tool_call_id`, `tool_name`, `command`, and `rule`. Completion is indicated by
//! `stream_finished` plus an idle snapshot/runtime state.

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::json;

use crate::daemon::state::DaemonInfo;

pub(crate) const CHAT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const CHAT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum ChatClientError {
    Http(String),
    Status { status: u16, body: String },
    Event(String),
    Json(String),
}

impl std::fmt::Display for ChatClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatClientError::Http(message) => write!(f, "{message}"),
            ChatClientError::Status { status, body } => {
                write!(f, "request failed with status {status}: {body}")
            }
            ChatClientError::Event(message) => write!(f, "{message}"),
            ChatClientError::Json(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ChatClientError {}

impl ChatClientError {
    pub fn is_unreachable(&self) -> bool {
        match self {
            ChatClientError::Http(_) => true,
            ChatClientError::Status { status, .. } => matches!(*status, 502 | 503 | 504),
            ChatClientError::Event(_) | ChatClientError::Json(_) => false,
        }
    }
}

pub type ChatEventStream = BoxStream<'static, Result<serde_json::Value, ChatClientError>>;

#[derive(Clone)]
pub struct ProxyChatClient {
    base_url: String,
    project_id: String,
    auth_token: Option<String>,
    client: reqwest::Client,
}

impl ProxyChatClient {
    pub fn new(
        base_url: String,
        project_id: String,
        auth_token: Option<String>,
    ) -> Result<Self, ChatClientError> {
        let client = reqwest::Client::builder()
            .connect_timeout(CHAT_CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                ChatClientError::Http(format!("failed to build HTTP client: {error}"))
            })?;
        Ok(Self {
            base_url: trim_base_url(base_url),
            project_id,
            auth_token,
            client,
        })
    }

    pub fn from_daemon_info(
        info: &DaemonInfo,
        project_id: String,
    ) -> Result<Self, ChatClientError> {
        Self::new(daemon_base_url(info), project_id, info.auth_token.clone())
    }

    pub async fn subscribe(&self, chat_id: &str) -> Result<ChatEventStream, ChatClientError> {
        let url = format!(
            "{}/p/{}/v1/chats/subscribe?chat_id={}",
            self.base_url,
            self.project_id,
            encode_query_value(chat_id)
        );
        let response = send_with_timeout(
            self.with_auth(self.client.get(url)),
            "failed to subscribe to chat",
        )
        .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let stream = response
            .bytes_stream()
            .eventsource()
            .filter_map(|event| async move {
                match event {
                    Ok(event) if event.data.trim().is_empty() => None,
                    Ok(event) => Some(
                        serde_json::from_str::<serde_json::Value>(&event.data).map_err(|error| {
                            ChatClientError::Json(format!("invalid SSE JSON: {error}"))
                        }),
                    ),
                    Err(error) => Some(Err(ChatClientError::Event(format!(
                        "chat SSE error: {error}"
                    )))),
                }
            })
            .boxed();
        Ok(stream)
    }

    pub async fn send_set_params(
        &self,
        chat_id: &str,
        client_request_id: String,
        patch: serde_json::Value,
    ) -> Result<(), ChatClientError> {
        self.send_command(
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "set_params",
                "patch": patch,
            }),
        )
        .await
    }

    pub async fn send_user_message(
        &self,
        chat_id: &str,
        client_request_id: String,
        prompt: &str,
    ) -> Result<(), ChatClientError> {
        self.send_command(
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "user_message",
                "content": prompt,
            }),
        )
        .await
    }

    pub async fn send_tool_decisions(
        &self,
        chat_id: &str,
        client_request_id: String,
        decisions: Vec<ToolDecision>,
    ) -> Result<(), ChatClientError> {
        self.send_command(
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "tool_decisions",
                "decisions": decisions,
            }),
        )
        .await
    }

    pub async fn send_abort(
        &self,
        chat_id: &str,
        client_request_id: String,
    ) -> Result<(), ChatClientError> {
        self.send_command(
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "abort",
            }),
        )
        .await
    }

    async fn send_command(
        &self,
        chat_id: &str,
        body: serde_json::Value,
    ) -> Result<(), ChatClientError> {
        let url = format!(
            "{}/p/{}/v1/chats/{}/commands",
            self.base_url,
            self.project_id,
            encode_path_segment(chat_id)
        );
        let response = send_with_timeout(
            self.with_auth(self.client.post(url).json(&body)),
            "failed to send chat command",
        )
        .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(status_error(response).await)
        }
    }

    fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolDecision {
    pub tool_call_id: String,
    pub accepted: bool,
}

pub fn daemon_base_url(info: &DaemonInfo) -> String {
    format!("http://{}:{}", connect_host(&info.bind), info.port)
}

fn connect_host(bind: &str) -> String {
    match bind {
        "0.0.0.0" | "::" => "127.0.0.1".to_string(),
        other => other.to_string(),
    }
}

fn trim_base_url(base_url: String) -> String {
    base_url.trim_end_matches('/').to_string()
}

fn encode_query_value(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn encode_path_segment(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

async fn status_error(response: reqwest::Response) -> ChatClientError {
    let status = response.status().as_u16();
    let body = match tokio::time::timeout(CHAT_REQUEST_TIMEOUT, response.text()).await {
        Ok(Ok(body)) => body,
        Ok(Err(error)) => error.to_string(),
        Err(_) => format!(
            "response body timed out after {} seconds",
            CHAT_REQUEST_TIMEOUT.as_secs()
        ),
    };
    ChatClientError::Status { status, body }
}

async fn send_with_timeout(
    request: reqwest::RequestBuilder,
    context: &str,
) -> Result<reqwest::Response, ChatClientError> {
    match tokio::time::timeout(CHAT_REQUEST_TIMEOUT, request.send()).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(ChatClientError::Http(format!("{context}: {error}"))),
        Err(_) => Err(ChatClientError::Http(format!(
            "{context}: request timed out after {} seconds",
            CHAT_REQUEST_TIMEOUT.as_secs()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn chat_client_builds_with_bounded_setup_timeouts() {
        assert_eq!(CHAT_CONNECT_TIMEOUT, Duration::from_secs(10));
        assert_eq!(CHAT_REQUEST_TIMEOUT, Duration::from_secs(30));
        let _ = ProxyChatClient::new("http://127.0.0.1:8488".to_string(), "p".to_string(), None)
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn send_command_times_out_on_unresponsive_endpoint() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
            drop(socket);
        });
        let client =
            ProxyChatClient::new(format!("http://127.0.0.1:{port}"), "p".to_string(), None)
                .unwrap();
        let request = tokio::spawn(async move {
            client
                .send_user_message("chat", "request".to_string(), "hello")
                .await
        });
        accepted_rx.await.unwrap();
        tokio::time::advance(CHAT_REQUEST_TIMEOUT + Duration::from_millis(1)).await;
        let error = request.await.unwrap().unwrap_err();
        let error = error.to_string();
        assert!(error.contains("failed to send chat command"));
        assert!(error.contains("timed out"));
        server.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_does_not_timeout_sse_body_stream() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(CHAT_REQUEST_TIMEOUT + Duration::from_secs(5)).await;
            socket
                .write_all(b"data: {\"type\":\"snapshot\",\"messages\":[]}\n\n")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let client =
            ProxyChatClient::new(format!("http://127.0.0.1:{port}"), "p".to_string(), None)
                .unwrap();
        let mut stream = client.subscribe("chat").await.unwrap();
        tokio::time::advance(CHAT_REQUEST_TIMEOUT + Duration::from_secs(5)).await;
        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(
            event.get("type").and_then(serde_json::Value::as_str),
            Some("snapshot")
        );
        server.abort();
    }

    #[test]
    fn daemon_base_url_uses_loopback_for_wildcard_binds() {
        let mut info = DaemonInfo {
            pid: 1,
            port: 8488,
            bind: "0.0.0.0".to_string(),
            version: "1".to_string(),
            auth_token: None,
            started_at_ms: 0,
            hostname_local: "host.local".to_string(),
            urls: crate::daemon::state::DaemonUrls {
                loopback: "".to_string(),
                mdns: "".to_string(),
            },
        };
        assert_eq!(daemon_base_url(&info), "http://127.0.0.1:8488");
        info.bind = "127.0.0.1".to_string();
        assert_eq!(daemon_base_url(&info), "http://127.0.0.1:8488");
    }

    #[test]
    fn chat_client_status_502_is_unreachable() {
        assert!(ChatClientError::Status {
            status: 502,
            body: "bad gateway".to_string(),
        }
        .is_unreachable());
        assert!(!ChatClientError::Status {
            status: 400,
            body: "bad".to_string(),
        }
        .is_unreachable());
    }
}
