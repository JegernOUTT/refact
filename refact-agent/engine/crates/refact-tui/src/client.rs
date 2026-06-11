use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eventsource_stream::Eventsource;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events_pane::{parse_daemon_event, DaemonEventRecord};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("request failed with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("SSE error: {0}")]
    Sse(String),
}

#[derive(Clone)]
pub struct DaemonClient {
    base_url: String,
    auth_token: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DaemonStatus {
    pub pid: u32,
    pub version: String,
    pub port: u16,
    pub started_at_ms: u64,
    pub uptime_secs: u64,
    pub workers: u64,
    #[serde(default)]
    pub cron_pending: HashMap<String, u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub slug: String,
    pub root: PathBuf,
    pub pinned: bool,
    pub last_active_ms: u64,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenProjectResponse {
    pub project_id: String,
    pub slug: String,
    pub root: PathBuf,
    pub pinned: bool,
    pub worker: Option<WorkerInfo>,
    pub cron_pending: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WorkerInfo {
    pub project_id: String,
    pub pid: Option<u32>,
    pub http_port: u16,
    pub lsp_port: u16,
    pub state: Value,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatEvent {
    pub chat_id: Option<String>,
    pub kind: String,
    pub raw: Value,
}

pub type ChatEventStream = BoxStream<'static, Result<ChatEvent, ClientError>>;
pub type DaemonEventStream = BoxStream<'static, Result<DaemonEventRecord, ClientError>>;

impl DaemonClient {
    pub fn new(
        base_url: impl Into<String>,
        auth_token: Option<String>,
    ) -> Result<Self, ClientError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| ClientError::Http(format!("failed to build HTTP client: {error}")))?;
        Ok(Self {
            base_url: trim_base_url(base_url.into()),
            auth_token,
            client,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn status(&self) -> Result<DaemonStatus, ClientError> {
        self.get_json("/daemon/v1/status").await
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectEntry>, ClientError> {
        self.get_json("/daemon/v1/projects").await
    }

    pub async fn open_project(&self, root: &Path) -> Result<OpenProjectResponse, ClientError> {
        let response = self
            .with_auth(self.client.post(self.url("/daemon/v1/projects/open")))
            .json(&json!({"root": root.to_string_lossy()}))
            .send()
            .await
            .map_err(|error| ClientError::Http(format!("failed to open project: {error}")))?;
        decode_response(response).await
    }

    pub async fn list_workers(&self) -> Result<Vec<WorkerInfo>, ClientError> {
        self.get_json("/daemon/v1/workers").await
    }

    pub async fn get_caps(&self, project_id: &str) -> Result<Value, ClientError> {
        let path = format!("/p/{}/v1/caps", encode_path_segment(project_id));
        self.get_json(&path).await
    }

    pub async fn get_chat_modes(&self, project_id: &str) -> Result<Value, ClientError> {
        let path = format!("/p/{}/v1/chat-modes", encode_path_segment(project_id));
        self.get_json(&path).await
    }

    pub async fn subscribe_daemon_events(&self) -> Result<DaemonEventStream, ClientError> {
        let response = self
            .with_auth(self.client.get(self.url("/daemon/v1/events?follow=true")))
            .send()
            .await
            .map_err(|error| {
                ClientError::Http(format!("failed to subscribe to daemon events: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(response
            .bytes_stream()
            .map_err(|error| ClientError::Sse(error.to_string()))
            .eventsource()
            .filter_map(|event| async move {
                match event {
                    Ok(event) if event.data.trim().is_empty() => None,
                    Ok(event) => Some(
                        parse_daemon_event(&event.data)
                            .map_err(|error| ClientError::Json(error.to_string())),
                    ),
                    Err(error) => Some(Err(ClientError::Sse(error.to_string()))),
                }
            })
            .boxed())
    }

    pub async fn subscribe_chat(
        &self,
        project_id: &str,
        chat_id: &str,
    ) -> Result<ChatEventStream, ClientError> {
        let path = format!(
            "/p/{}/v1/chats/subscribe?chat_id={}",
            encode_path_segment(project_id),
            encode_query_value(chat_id)
        );
        let response = self
            .with_auth(self.client.get(self.url(&path)))
            .send()
            .await
            .map_err(|error| ClientError::Http(format!("failed to subscribe to chat: {error}")))?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(response
            .bytes_stream()
            .map_err(|error| ClientError::Sse(error.to_string()))
            .eventsource()
            .filter_map(|event| async move {
                match event {
                    Ok(event) if event.data.trim().is_empty() => None,
                    Ok(event) => Some(parse_chat_event(&event.data)),
                    Err(error) => Some(Err(ClientError::Sse(error.to_string()))),
                }
            })
            .boxed())
    }

    pub async fn send_set_params(
        &self,
        project_id: &str,
        chat_id: &str,
        patch: Value,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": request_id("set-params"),
                "type": "set_params",
                "patch": patch,
            }),
        )
        .await
    }

    pub async fn send_user_message(
        &self,
        project_id: &str,
        chat_id: &str,
        content: &str,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": request_id("user-message"),
                "type": "user_message",
                "content": content,
            }),
        )
        .await
    }

    pub async fn send_abort(&self, project_id: &str, chat_id: &str) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": request_id("abort"),
                "type": "abort",
            }),
        )
        .await
    }

    pub async fn send_tool_decisions(
        &self,
        project_id: &str,
        chat_id: &str,
        decisions: Vec<ToolDecision>,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": request_id("tool-decisions"),
                "type": "tool_decisions",
                "decisions": decisions,
            }),
        )
        .await
    }

    async fn send_command(
        &self,
        project_id: &str,
        chat_id: &str,
        body: Value,
    ) -> Result<(), ClientError> {
        let path = format!(
            "/p/{}/v1/chats/{}/commands",
            encode_path_segment(project_id),
            encode_path_segment(chat_id)
        );
        let response = self
            .with_auth(self.client.post(self.url(&path)).json(&body))
            .send()
            .await
            .map_err(|error| ClientError::Http(format!("failed to send chat command: {error}")))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(status_error(response).await)
        }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ClientError> {
        let response = self
            .with_auth(self.client.get(self.url(path)))
            .send()
            .await
            .map_err(|error| ClientError::Http(error.to_string()))?;
        decode_response(response).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SseLineParser {
    buffer: String,
    data_lines: Vec<String>,
}

impl SseLineParser {
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.find('\n') {
            let mut line = self.buffer[..newline].to_string();
            self.buffer.drain(..=newline);
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                if !self.data_lines.is_empty() {
                    events.push(self.data_lines.join("\n"));
                    self.data_lines.clear();
                }
                continue;
            }
            if let Some(data) = line.strip_prefix("data:") {
                self.data_lines.push(data.trim_start().to_string());
            }
        }
        events
    }
}

fn parse_chat_event(data: &str) -> Result<ChatEvent, ClientError> {
    let raw: Value =
        serde_json::from_str(data).map_err(|error| ClientError::Json(error.to_string()))?;
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let chat_id = raw
        .get("chat_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ChatEvent { chat_id, kind, raw })
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, ClientError> {
    if !response.status().is_success() {
        return Err(status_error(response).await);
    }
    response
        .json::<T>()
        .await
        .map_err(|error| ClientError::Json(error.to_string()))
}

async fn status_error(response: reqwest::Response) -> ClientError {
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| error.to_string());
    ClientError::Status { status, body }
}

fn trim_base_url(base_url: String) -> String {
    base_url.trim_end_matches('/').to_string()
}

fn encode_query_value(value: &str) -> String {
    url_encode(value)
}

fn encode_path_segment(value: &str) -> String {
    url_encode(value)
}

fn url_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn request_id(prefix: &str) -> String {
    format!("tui-{prefix}-{}", uuid::Uuid::new_v4())
}

pub fn worker_state_label(worker: Option<&WorkerInfo>) -> String {
    match worker {
        Some(worker) => match &worker.state {
            Value::String(value) => value.clone(),
            Value::Object(map) => map
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "worker".to_string()),
            _ => "worker".to_string(),
        },
        None => "unknown".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolDecision {
    pub tool_call_id: String,
    pub accepted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_handles_partial_frames() {
        let mut parser = SseLineParser::default();
        assert!(parser.push("data: {\"a\"").is_empty());
        assert_eq!(parser.push(":1}\n\n"), vec!["{\"a\":1}".to_string()]);
        assert!(parser.push("data: one\n").is_empty());
        assert_eq!(parser.push("data: two\n\n"), vec!["one\ntwo".to_string()]);
    }

    #[test]
    fn chat_event_parses_type_and_chat_id() {
        let event = parse_chat_event(r#"{"chat_id":"c","type":"stream_started"}"#).unwrap();
        assert_eq!(event.chat_id.as_deref(), Some("c"));
        assert_eq!(event.kind, "stream_started");
    }
}
