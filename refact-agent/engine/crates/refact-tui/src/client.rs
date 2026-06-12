use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events_pane::{parse_daemon_event, DaemonEventRecord};

const DEFAULT_DAEMON_PORT: u16 = 8488;
const PLAIN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PLAIN_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SSE_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_DIR_ENV: &str = "REFACT_DAEMON_DIR";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonDiscoveryWarning {
    InvalidInfo { path: PathBuf, message: String },
}

impl DaemonDiscoveryWarning {
    pub fn notice(&self) -> String {
        match self {
            DaemonDiscoveryWarning::InvalidInfo { path, message } => {
                format!(
                    "Failed to read daemon info from {}: {message}",
                    path.display()
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub base_url: String,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DaemonInfoFile {
    pub pid: u32,
    pub port: u16,
    #[serde(default)]
    pub bind: String,
    pub version: String,
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Clone)]
pub struct DaemonClient {
    base_url: String,
    auth_token: Option<String>,
    client: reqwest::Client,
    sse_client: reqwest::Client,
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
    pub seq: Option<u64>,
    pub kind: String,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatSeqDecision {
    Apply,
    Resubscribe(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatSeqTracker {
    last_seq: Option<u64>,
}

impl ChatSeqTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.last_seq = None;
    }

    pub fn observe(&mut self, event: &ChatEvent) -> ChatSeqDecision {
        let Some(seq) = event.seq else {
            return ChatSeqDecision::Resubscribe(format!(
                "missing SSE seq for {} event",
                event.kind
            ));
        };
        if event.kind == "snapshot" {
            self.last_seq = Some(seq);
            return ChatSeqDecision::Apply;
        }
        match self.last_seq.and_then(|last| last.checked_add(1)) {
            Some(expected) if seq == expected => {
                self.last_seq = Some(seq);
                ChatSeqDecision::Apply
            }
            Some(expected) => ChatSeqDecision::Resubscribe(format!(
                "SSE seq mismatch: expected {expected}, got {seq} for {} event",
                event.kind
            )),
            None => ChatSeqDecision::Resubscribe(format!(
                "SSE stream started with {} event before snapshot",
                event.kind
            )),
        }
    }
}

pub type ChatEventStream = BoxStream<'static, Result<ChatEvent, ClientError>>;
pub type DaemonEventStream = BoxStream<'static, Result<DaemonEventRecord, ClientError>>;

impl DaemonEndpoint {
    pub fn fallback() -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{DEFAULT_DAEMON_PORT}"),
            auth_token: None,
        }
    }

    fn from_info(info: DaemonInfoFile) -> Self {
        Self {
            base_url: format!("http://{}:{}", connect_host(&info.bind), info.port),
            auth_token: info.auth_token.filter(|token| !token.is_empty()),
        }
    }
}

pub fn discover_daemon_endpoint() -> Result<Option<DaemonEndpoint>, DaemonDiscoveryWarning> {
    discover_daemon_endpoint_from(&daemon_json_path())
}

pub fn resolve_daemon_endpoint(
    explicit_base_url: Option<String>,
) -> Result<DaemonEndpoint, DaemonDiscoveryWarning> {
    let endpoint = discover_daemon_endpoint()?.unwrap_or_else(DaemonEndpoint::fallback);
    Ok(
        match explicit_base_url.filter(|value| !value.trim().is_empty()) {
            Some(base_url) => DaemonEndpoint {
                base_url,
                ..endpoint
            },
            None => endpoint,
        },
    )
}

pub fn discover_daemon_endpoint_from(
    path: &Path,
) -> Result<Option<DaemonEndpoint>, DaemonDiscoveryWarning> {
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<DaemonInfoFile>(&content) {
            Ok(info) => Ok(Some(DaemonEndpoint::from_info(info))),
            Err(error) => Err(invalid_info_warning(path, error.to_string())),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(invalid_info_warning(path, error.to_string())),
    }
}

impl DaemonClient {
    pub fn new(
        base_url: impl Into<String>,
        auth_token: Option<String>,
    ) -> Result<Self, ClientError> {
        let client = build_plain_http_client()?;
        let sse_client = build_sse_http_client()?;
        Ok(Self {
            base_url: trim_base_url(base_url.into()),
            auth_token,
            client,
            sse_client,
        })
    }

    pub fn from_endpoint(endpoint: DaemonEndpoint) -> Result<Self, ClientError> {
        Self::new(endpoint.base_url, endpoint.auth_token)
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
            .with_auth(
                self.sse_client
                    .get(self.url("/daemon/v1/events?follow=true")),
            )
            .send()
            .await
            .map_err(|error| {
                ClientError::Http(format!("failed to subscribe to daemon events: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(sse_data_stream(response)
            .map(|data| {
                data.and_then(|data| {
                    parse_daemon_event(&data).map_err(|error| ClientError::Json(error.to_string()))
                })
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
            .with_auth(self.sse_client.get(self.url(&path)))
            .send()
            .await
            .map_err(|error| ClientError::Http(format!("failed to subscribe to chat: {error}")))?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(sse_data_stream(response)
            .map(|data| data.and_then(|data| parse_chat_event(&data)))
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

fn sse_data_stream(response: reqwest::Response) -> BoxStream<'static, Result<String, ClientError>> {
    response
        .bytes_stream()
        .map({
            let mut parser = SseLineParser::default();
            let mut pending_utf8 = Vec::new();
            move |chunk| {
                let chunk = chunk.map_err(|error| ClientError::Sse(error.to_string()))?;
                let text = drain_utf8_chunk(&mut pending_utf8, &chunk)?;
                let events = parser
                    .push(&text)
                    .into_iter()
                    .filter(|data| !data.trim().is_empty())
                    .map(Ok::<_, ClientError>)
                    .collect::<Vec<_>>();
                Ok(futures::stream::iter(events))
            }
        })
        .try_flatten()
        .boxed()
}

fn parse_chat_event(data: &str) -> Result<ChatEvent, ClientError> {
    let raw: Value =
        serde_json::from_str(data).map_err(|error| ClientError::Json(error.to_string()))?;
    let seq = parse_seq(raw.get("seq"))?;
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let chat_id = raw
        .get("chat_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ChatEvent {
        chat_id,
        seq,
        kind,
        raw,
    })
}

fn parse_seq(value: Option<&Value>) -> Result<Option<u64>, ClientError> {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| ClientError::Json("invalid SSE seq number".to_string())),
        Some(Value::String(value)) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|error| ClientError::Json(format!("invalid SSE seq: {error}"))),
        Some(_) => Err(ClientError::Json("invalid SSE seq type".to_string())),
        None => Ok(None),
    }
}

fn drain_utf8_chunk(pending: &mut Vec<u8>, chunk: &[u8]) -> Result<String, ClientError> {
    pending.extend_from_slice(chunk);
    match std::str::from_utf8(pending) {
        Ok(text) => {
            let text = text.to_string();
            pending.clear();
            Ok(text)
        }
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            let text = std::str::from_utf8(&pending[..valid])
                .map_err(|error| ClientError::Sse(error.to_string()))?
                .to_string();
            let rest = pending.split_off(valid);
            *pending = rest;
            Ok(text)
        }
        Err(error) => Err(ClientError::Sse(format!(
            "invalid UTF-8 in SSE stream: {error}"
        ))),
    }
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

fn build_plain_http_client() -> Result<reqwest::Client, ClientError> {
    reqwest::Client::builder()
        .connect_timeout(PLAIN_HTTP_CONNECT_TIMEOUT)
        .timeout(PLAIN_HTTP_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| ClientError::Http(format!("failed to build HTTP client: {error}")))
}

fn build_sse_http_client() -> Result<reqwest::Client, ClientError> {
    reqwest::Client::builder()
        .connect_timeout(SSE_HTTP_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| ClientError::Http(format!("failed to build SSE client: {error}")))
}

fn invalid_info_warning(path: &Path, message: String) -> DaemonDiscoveryWarning {
    DaemonDiscoveryWarning::InvalidInfo {
        path: path.to_path_buf(),
        message,
    }
}

fn daemon_json_path() -> PathBuf {
    daemon_dir().join("daemon.json")
}

fn daemon_dir() -> PathBuf {
    std::env::var_os(DAEMON_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_refact_dir().join("daemon"))
}

fn cache_refact_dir() -> PathBuf {
    home_dir().join(".cache").join("refact")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn connect_host(bind: &str) -> &str {
    match bind {
        "" | "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    }
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
    fn chat_event_parses_type_chat_id_and_seq() {
        let event =
            parse_chat_event(r#"{"chat_id":"c","seq":"7","type":"stream_started"}"#).unwrap();
        assert_eq!(event.chat_id.as_deref(), Some("c"));
        assert_eq!(event.seq, Some(7));
        assert_eq!(event.kind, "stream_started");
    }
}
