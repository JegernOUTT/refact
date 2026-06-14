use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events_pane::{parse_daemon_event, DaemonEventRecord};
use crate::protocol::SseEvent;
use crate::sessions::{PaginatedTrajectories, TrajectoryMeta};

const DEFAULT_DAEMON_PORT: u16 = 8488;
const PLAIN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PLAIN_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SSE_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_DIR_ENV: &str = "REFACT_DAEMON_DIR";
const STATUS_BODY_NOTICE_MAX_CHARS: usize = 300;

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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct AtCommandCompletionResponse {
    #[serde(default)]
    completions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntegrationListResponse {
    #[serde(default)]
    pub integrations: Vec<IntegrationRecord>,
    #[serde(default)]
    pub error_log: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntegrationRecord {
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub integr_name: String,
    #[serde(default)]
    pub integr_config_path: String,
    #[serde(default)]
    pub integr_config_exists: bool,
    #[serde(default)]
    pub config_unparsed: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpViewData {
    pub servers: Vec<McpServerSummary>,
    pub error_log: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerSummary {
    pub name: String,
    pub transport: String,
    pub project_path: String,
    pub config_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<McpServerInfoResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerInfoResponse {
    pub config_path: String,
    #[serde(default)]
    pub status: Value,
    #[serde(default)]
    pub auth_status: Value,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub protocol_version: Option<String>,
    #[serde(default)]
    pub tools: Vec<McpToolInfo>,
    #[serde(default)]
    pub resources: Vec<McpResourceInfo>,
    #[serde(default)]
    pub prompts: Vec<McpPromptInfo>,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub logs_tail: Vec<String>,
    #[serde(default)]
    pub metrics: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<Value>,
    #[serde(default)]
    pub internal_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpResourceInfo {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpPromptInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlashCommandsListResponse {
    #[serde(default)]
    pub commands: Vec<SlashCommandInfo>,
    #[serde(default)]
    pub skills: Vec<SkillInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlashCommandInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub user_invocable: bool,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeGraphResponse {
    #[serde(default)]
    pub nodes: Vec<KnowledgeNode>,
    #[serde(default)]
    pub edges: Vec<KnowledgeEdge>,
    #[serde(default)]
    pub stats: KnowledgeStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeNode {
    pub id: String,
    #[serde(default)]
    pub node_type: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeStats {
    #[serde(default)]
    pub doc_count: usize,
    #[serde(default)]
    pub tag_count: usize,
    #[serde(default)]
    pub file_count: usize,
    #[serde(default)]
    pub entity_count: usize,
    #[serde(default)]
    pub edge_count: usize,
    #[serde(default)]
    pub active_docs: usize,
    #[serde(default)]
    pub deprecated_docs: usize,
    #[serde(default)]
    pub trajectory_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatEvent {
    pub chat_id: Option<String>,
    pub seq: Option<u64>,
    pub kind: String,
    pub raw: Value,
}

impl ChatEvent {
    pub fn protocol_event(&self) -> SseEvent {
        let mut raw = self.raw.clone();
        if raw.get("type").is_none() {
            if let Value::Object(map) = &mut raw {
                map.insert("type".to_string(), Value::String(self.kind.clone()));
            }
        }
        SseEvent::from_raw(&raw)
    }
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

    pub async fn mcp_view_data(&self, project_id: &str) -> Result<McpViewData, ClientError> {
        let integrations = self.list_integrations(project_id).await?;
        let mut servers = Vec::new();
        for integration in integrations
            .integrations
            .into_iter()
            .filter(is_configured_mcp_integration)
        {
            let info = self
                .mcp_server_info(project_id, &integration.integr_config_path)
                .await;
            let (info, error) = match info {
                Ok(info) => (Some(info), None),
                Err(error) => (None, Some(error.to_string())),
            };
            servers.push(McpServerSummary {
                name: integration.integr_name.clone(),
                transport: mcp_transport(&integration.integr_name)
                    .unwrap_or("mcp")
                    .to_string(),
                project_path: integration.project_path,
                config_path: integration.integr_config_path,
                info,
                error,
            });
        }
        servers.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(McpViewData {
            servers,
            error_log: integrations.error_log,
        })
    }

    pub async fn list_integrations(
        &self,
        project_id: &str,
    ) -> Result<IntegrationListResponse, ClientError> {
        let path = format!("/p/{}/v1/integrations", encode_path_segment(project_id));
        self.get_json(&path).await
    }

    pub async fn mcp_server_info(
        &self,
        project_id: &str,
        config_path: &str,
    ) -> Result<McpServerInfoResponse, ClientError> {
        let path = format!(
            "/p/{}/v1/mcp-server-info?config_path={}",
            encode_path_segment(project_id),
            encode_query_value(config_path)
        );
        self.get_json(&path).await
    }

    pub async fn slash_commands(
        &self,
        project_id: &str,
    ) -> Result<SlashCommandsListResponse, ClientError> {
        let path = format!("/p/{}/v1/slash-commands", encode_path_segment(project_id));
        self.get_json(&path).await
    }

    pub async fn knowledge_graph(
        &self,
        project_id: &str,
    ) -> Result<KnowledgeGraphResponse, ClientError> {
        let path = format!("/p/{}/v1/knowledge-graph", encode_path_segment(project_id));
        self.get_json(&path).await
    }

    pub async fn at_command_completion(
        &self,
        project_id: &str,
        query: &str,
        cursor: i64,
        top_n: usize,
    ) -> Result<Vec<String>, ClientError> {
        let path = format!(
            "/p/{}/v1/at-command-completion",
            encode_path_segment(project_id)
        );
        let response = self
            .with_auth(self.client.post(self.url(&path)))
            .json(&json!({
                "query": query,
                "cursor": cursor,
                "top_n": top_n,
            }))
            .send()
            .await
            .map_err(|error| {
                ClientError::Http(format!("failed to load at-command completions: {error}"))
            })?;
        let response: AtCommandCompletionResponse = decode_response(response).await?;
        Ok(response.completions)
    }

    pub async fn list_trajectories(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<TrajectoryMeta>, ClientError> {
        let path = format!(
            "/p/{}/v1/trajectories?displayable_only=true&limit={}",
            encode_path_segment(project_id),
            limit.clamp(1, 200)
        );
        let response: PaginatedTrajectories = self.get_json(&path).await?;
        Ok(response.items)
    }

    pub async fn send_branch_from_chat(
        &self,
        project_id: &str,
        chat_id: &str,
        source_chat_id: &str,
        up_to_message_id: &str,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": request_id("branch-from-chat"),
                "type": "branch_from_chat",
                "source_chat_id": source_chat_id,
                "up_to_message_id": up_to_message_id,
            }),
        )
        .await
    }

    pub async fn delete_trajectory(
        &self,
        project_id: &str,
        chat_id: &str,
    ) -> Result<(), ClientError> {
        let path = format!(
            "/p/{}/v1/trajectories/{}",
            encode_path_segment(project_id),
            encode_path_segment(chat_id)
        );
        let response = self
            .with_auth(self.client.delete(self.url(&path)))
            .send()
            .await
            .map_err(|error| ClientError::Http(format!("failed to delete trajectory: {error}")))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(status_error(response).await)
        }
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

    pub async fn send_retry_from_index(
        &self,
        project_id: &str,
        chat_id: &str,
        index: usize,
        content: Value,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": request_id("retry-from-index"),
                "type": "retry_from_index",
                "index": index,
                "content": content,
                "attachments": [],
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
        .map(sanitize_status_body)
        .unwrap_or_else(|error| sanitize_status_body(error.to_string()));
    ClientError::Status { status, body }
}

fn sanitize_status_body(body: impl AsRef<str>) -> String {
    let mut sanitized = body
        .as_ref()
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if sanitized.chars().count() > STATUS_BODY_NOTICE_MAX_CHARS {
        sanitized = sanitized
            .chars()
            .take(STATUS_BODY_NOTICE_MAX_CHARS)
            .collect::<String>();
        sanitized.push('…');
    }
    sanitized
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

fn is_configured_mcp_integration(integration: &IntegrationRecord) -> bool {
    integration.integr_config_exists && mcp_transport(&integration.integr_name).is_some()
}

fn mcp_transport(name: &str) -> Option<&'static str> {
    if name.starts_with("mcp_stdio_") {
        Some("stdio")
    } else if name.starts_with("mcp_sse_") {
        Some("sse")
    } else if name.starts_with("mcp_http_") {
        Some("http")
    } else {
        None
    }
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

    #[test]
    fn status_body_notice_is_sanitized_and_truncated() {
        let input = format!("bad\u{1b}[31m\n{}", "x".repeat(400));
        let body = sanitize_status_body(input);
        assert!(!body.contains('\u{1b}'));
        assert!(!body.contains('\n'));
        assert!(body.ends_with('…'));
        assert_eq!(body.chars().count(), STATUS_BODY_NOTICE_MAX_CHARS + 1);
    }

    #[test]
    fn slash_commands_fixture_parses_typed_lists() {
        let response: SlashCommandsListResponse = serde_json::from_str(
            r#"{
                "commands": [{"name":"review","description":"Review","argument_hint":"[path]","source":"project_refact"}],
                "skills": [{"name":"explain","description":"Explain","user_invocable":true,"source":"global_refact"}]
            }"#,
        )
        .unwrap();
        assert_eq!(response.commands[0].name, "review");
        assert_eq!(
            response.commands[0].argument_hint.as_deref(),
            Some("[path]")
        );
        assert_eq!(response.skills[0].name, "explain");
        assert!(response.skills[0].user_invocable);
    }

    #[test]
    fn knowledge_graph_fixture_parses_stats_and_docs() {
        let response: KnowledgeGraphResponse = serde_json::from_str(
            r#"{
                "nodes": [{"id":"doc1","node_type":"doc_decision","label":"Decision","tags":["ui"],"file_path":".refact/knowledge/d.md","kind":"decision"}],
                "edges": [{"source":"doc1","target":"tag:ui","edge_type":"tagged_with"}],
                "stats": {"doc_count":1,"tag_count":1,"file_count":0,"entity_count":0,"edge_count":1,"active_docs":1,"deprecated_docs":0,"trajectory_count":0}
            }"#,
        )
        .unwrap();
        assert_eq!(response.stats.doc_count, 1);
        assert_eq!(response.nodes[0].kind.as_deref(), Some("decision"));
        assert_eq!(response.edges[0].edge_type, "tagged_with");
    }

    #[test]
    fn mcp_fixtures_filter_configured_servers_and_parse_info() {
        let integrations: IntegrationListResponse = serde_json::from_str(
            r#"{
                "integrations": [
                    {"integr_name":"mcp_stdio_demo","integr_config_path":"/tmp/mcp.yaml","integr_config_exists":true},
                    {"integr_name":"mcp_sse_missing","integr_config_path":"/tmp/missing.yaml","integr_config_exists":false},
                    {"integr_name":"github","integr_config_path":"/tmp/github.yaml","integr_config_exists":true}
                ],
                "error_log": []
            }"#,
        )
        .unwrap();
        let servers = integrations
            .integrations
            .iter()
            .filter(|integration| is_configured_mcp_integration(integration))
            .collect::<Vec<_>>();
        assert_eq!(servers.len(), 1);
        assert_eq!(mcp_transport(&servers[0].integr_name), Some("stdio"));

        let info: McpServerInfoResponse = serde_json::from_str(
            r#"{
                "config_path":"/tmp/mcp.yaml",
                "status":{"status":"connected"},
                "auth_status":"not_applicable",
                "tools":[{"name":"lookup","description":"Lookup","input_schema":{"type":"object"},"internal_name":"demo_lookup"}],
                "resources":[],
                "prompts":[],
                "capabilities":{"tools":true},
                "logs_tail":[],
                "metrics":{}
            }"#,
        )
        .unwrap();
        assert_eq!(info.tools[0].internal_name, "demo_lookup");
        assert_eq!(info.status["status"], "connected");
    }
}
