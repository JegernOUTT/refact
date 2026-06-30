use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::events_pane::{parse_daemon_event, DaemonEventRecord};
use crate::protocol::SseEvent;
use crate::sessions::{PaginatedTrajectories, TrajectoryMeta};

const DEFAULT_DAEMON_PORT: u16 = 8488;
const PLAIN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PLAIN_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SSE_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(not(test))]
const SSE_HEADER_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(test)]
const SSE_HEADER_TIMEOUT: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const SSE_ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(test)]
const SSE_ERROR_BODY_TIMEOUT: Duration = Duration::from_millis(100);
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
    #[error("SSE stream disconnected unexpectedly: {0}")]
    SseDisconnect(String),
    #[error("worker not ready: {0}")]
    WorkerNotReady(String),
}

impl ClientError {
    pub fn is_unreachable(&self) -> bool {
        match self {
            ClientError::Status { status, .. } => matches!(*status, 502 | 503 | 504),
            ClientError::Http(message) => message_indicates_unreachable(message),
            ClientError::SseDisconnect(_) => true,
            ClientError::Sse(message) => message_indicates_unreachable(message),
            ClientError::Json(_) | ClientError::WorkerNotReady(_) => false,
        }
    }

    pub fn is_auth_stale(&self) -> bool {
        match self {
            ClientError::Status { status, body } => {
                matches!(*status, 401 | 403) || message_indicates_auth_stale(body)
            }
            ClientError::Http(message) | ClientError::Sse(message) => {
                message_indicates_auth_stale(message)
            }
            ClientError::Json(_)
            | ClientError::SseDisconnect(_)
            | ClientError::WorkerNotReady(_) => false,
        }
    }
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
pub struct ProviderListResponse {
    #[serde(default)]
    pub providers: Vec<ProviderListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderListItem {
    pub name: String,
    #[serde(default)]
    pub base_provider: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub has_credentials: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub model_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderOAuthLogoutResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub auth_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HooksResponse {
    #[serde(default)]
    pub hooks: Vec<HookInfo>,
    #[serde(default)]
    pub raw_content: String,
    #[serde(default)]
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookInfo {
    pub event: String,
    #[serde(default)]
    pub matcher: Option<String>,
    pub command: String,
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompetitorImportInfoResponse {
    #[serde(default)]
    pub sources: Vec<CompetitorImportSourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompetitorImportSourceInfo {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Created,
    Updated,
    Unchanged,
    Stale,
    Conflict,
    UserModified,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReportCounts {
    #[serde(default)]
    pub discovered: usize,
    #[serde(default)]
    pub created: usize,
    #[serde(default)]
    pub updated: usize,
    #[serde(default)]
    pub unchanged: usize,
    #[serde(default)]
    pub stale: usize,
    #[serde(default)]
    pub conflicts: usize,
    #[serde(default)]
    pub user_modified: usize,
    #[serde(default)]
    pub unsupported: usize,
    #[serde(default)]
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportReportIssue {
    #[serde(default)]
    pub competitor: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub status: ImportStatus,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportReport {
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub reported_sources: Vec<Value>,
    #[serde(default)]
    pub discovered_candidates: usize,
    #[serde(default)]
    pub status_counts: std::collections::BTreeMap<ImportStatus, usize>,
    #[serde(default)]
    pub competitor_counts: std::collections::BTreeMap<String, ImportReportCounts>,
    #[serde(default)]
    pub kind_counts: std::collections::BTreeMap<String, ImportReportCounts>,
    #[serde(default)]
    pub top_issues: Vec<ImportReportIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompetitorImportRunResponse {
    pub scope: String,
    #[serde(default)]
    pub source: Option<String>,
    pub report: ImportReport,
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
            base_url: daemon_base_url_from_bind(&info.bind, info.port),
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
    resolve_daemon_endpoint_from_path(&daemon_json_path(), explicit_base_url, None)
}

pub fn resolve_daemon_endpoint_with_auth(
    explicit_base_url: Option<String>,
    explicit_auth_token: Option<String>,
) -> Result<DaemonEndpoint, DaemonDiscoveryWarning> {
    resolve_daemon_endpoint_from_path(&daemon_json_path(), explicit_base_url, explicit_auth_token)
}

fn resolve_daemon_endpoint_from_path(
    path: &Path,
    explicit_base_url: Option<String>,
    explicit_auth_token: Option<String>,
) -> Result<DaemonEndpoint, DaemonDiscoveryWarning> {
    let endpoint = discover_daemon_endpoint_from(path)?.unwrap_or_else(DaemonEndpoint::fallback);
    let explicit_auth_token = explicit_auth_token.and_then(non_empty_string);
    Ok(match explicit_base_url.and_then(non_empty_string) {
        Some(base_url) => {
            let base_url = trim_base_url(base_url);
            let auth_token = explicit_auth_token.or_else(|| {
                same_origin(&endpoint.base_url, &base_url).then_some(endpoint.auth_token)?
            });
            DaemonEndpoint {
                base_url,
                auth_token,
            }
        }
        None => DaemonEndpoint {
            base_url: endpoint.base_url,
            auth_token: explicit_auth_token.or(endpoint.auth_token),
        },
    })
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
        let base_url = trim_base_url(base_url.into());
        let bypass_proxy = should_bypass_proxy_for_base_url(&base_url);
        let client = build_plain_http_client(bypass_proxy)?;
        let sse_client = build_sse_http_client(bypass_proxy)?;
        Ok(Self {
            base_url,
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
        let project: OpenProjectResponse = decode_response(response).await?;
        validate_open_project_response(&project)?;
        Ok(project)
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

    pub async fn providers(&self, project_id: &str) -> Result<ProviderListResponse, ClientError> {
        let path = providers_path(project_id);
        self.get_json(&path).await
    }

    pub async fn provider_oauth_logout(
        &self,
        project_id: &str,
        provider: &str,
    ) -> Result<ProviderOAuthLogoutResponse, ClientError> {
        let path = provider_oauth_logout_path(project_id, provider);
        self.post_json(&path, &json!({})).await
    }

    pub async fn hooks(&self, project_id: &str) -> Result<HooksResponse, ClientError> {
        let path = hooks_path(project_id);
        self.get_json(&path).await
    }

    pub async fn competitor_import_info(
        &self,
        project_id: &str,
    ) -> Result<CompetitorImportInfoResponse, ClientError> {
        let path = competitor_import_path(project_id);
        self.get_json(&path).await
    }

    pub async fn competitor_import_run(
        &self,
        project_id: &str,
        source: Option<&str>,
        scope: &str,
    ) -> Result<CompetitorImportRunResponse, ClientError> {
        let path = competitor_import_path(project_id);
        self.post_json(&path, &competitor_import_body(source, scope))
            .await
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
        let url = self.url("/daemon/v1/events?follow=true");
        let response = send_sse_request(
            self.with_auth(self.sse_client.get(url.clone())),
            &url,
            "failed to subscribe to daemon events",
        )
        .await?;
        if !response.status().is_success() {
            return Err(sse_status_error(response).await);
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
        let url = self.url(&path);
        let response = send_sse_request(
            self.with_auth(self.sse_client.get(url.clone())),
            &url,
            "failed to subscribe to chat",
        )
        .await?;
        if !response.status().is_success() {
            return Err(sse_status_error(response).await);
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

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, ClientError> {
        let response = self
            .with_auth(self.client.post(self.url(path)).json(body))
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
            let line = self.buffer[..newline].to_string();
            self.buffer.drain(..=newline);
            self.push_line(line, &mut events);
        }
        events
    }

    pub fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.push_line(line, &mut events);
        }
        if !self.data_lines.is_empty() {
            events.push(self.data_lines.join("\n"));
            self.data_lines.clear();
        }
        events
    }

    fn push_line(&mut self, mut line: String, events: &mut Vec<String>) {
        if line.ends_with('\r') {
            line.pop();
        }
        if line.is_empty() {
            if !self.data_lines.is_empty() {
                events.push(self.data_lines.join("\n"));
                self.data_lines.clear();
            }
            return;
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.data_lines.push(data.trim_start().to_string());
        }
    }
}

fn sse_data_stream(response: reqwest::Response) -> BoxStream<'static, Result<String, ClientError>> {
    let byte_stream = response.bytes_stream().boxed();
    let state = (
        byte_stream,
        SseLineParser::default(),
        Vec::<u8>::new(),
        VecDeque::<Result<String, ClientError>>::new(),
        false,
    );
    stream::unfold(
        state,
        |(mut byte_stream, mut parser, mut pending_utf8, mut pending_events, mut done)| async move {
            loop {
                if let Some(event) = pending_events.pop_front() {
                    return Some((
                        event,
                        (byte_stream, parser, pending_utf8, pending_events, done),
                    ));
                }
                if done {
                    return None;
                }
                match byte_stream.next().await {
                    Some(Ok(chunk)) => match drain_utf8_chunk(&mut pending_utf8, &chunk) {
                        Ok(text) => {
                            pending_events.extend(
                                parser
                                    .push(&text)
                                    .into_iter()
                                    .filter(|data| !data.trim().is_empty())
                                    .map(Ok),
                            );
                        }
                        Err(error) => {
                            done = true;
                            return Some((
                                Err(error),
                                (byte_stream, parser, pending_utf8, pending_events, done),
                            ));
                        }
                    },
                    Some(Err(error)) => {
                        done = true;
                        return Some((
                            Err(ClientError::Sse(error.to_string())),
                            (byte_stream, parser, pending_utf8, pending_events, done),
                        ));
                    }
                    None => {
                        done = true;
                        if !pending_utf8.is_empty() {
                            return Some((
                                Err(ClientError::Sse(
                                    "incomplete UTF-8 sequence at SSE EOF".to_string(),
                                )),
                                (byte_stream, parser, pending_utf8, pending_events, done),
                            ));
                        }
                        pending_events.extend(
                            parser
                                .finish()
                                .into_iter()
                                .filter(|data| !data.trim().is_empty())
                                .map(Ok),
                        );
                        pending_events.push_back(Err(ClientError::SseDisconnect(
                            "stream ended before the subscription was closed cleanly".to_string(),
                        )));
                    }
                }
            }
        },
    )
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

fn validate_open_project_response(project: &OpenProjectResponse) -> Result<(), ClientError> {
    let Some(worker) = project.worker.as_ref() else {
        return Err(ClientError::WorkerNotReady(format!(
            "project {} opened without worker details",
            project.project_id
        )));
    };
    let state = worker_state_label(Some(worker));
    if !state.eq_ignore_ascii_case("ready") {
        let last_error = worker
            .last_error
            .as_deref()
            .filter(|error| !error.trim().is_empty())
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        return Err(ClientError::WorkerNotReady(format!(
            "project {} worker state is {state}{last_error}",
            project.project_id
        )));
    }
    if worker.http_port == 0 || worker.lsp_port == 0 {
        return Err(ClientError::WorkerNotReady(format!(
            "project {} worker has invalid ports http={} lsp={}",
            project.project_id, worker.http_port, worker.lsp_port
        )));
    }
    Ok(())
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

async fn sse_status_error(response: reqwest::Response) -> ClientError {
    let status = response.status().as_u16();
    let body = match tokio::time::timeout(SSE_ERROR_BODY_TIMEOUT, response.text()).await {
        Ok(Ok(body)) => sanitize_status_body(body),
        Ok(Err(error)) => sanitize_status_body(error.to_string()),
        Err(_) => format!("status {status}"),
    };
    ClientError::Status { status, body }
}

async fn send_sse_request(
    request: reqwest::RequestBuilder,
    url: &str,
    failure_context: &str,
) -> Result<reqwest::Response, ClientError> {
    match tokio::time::timeout(SSE_HEADER_TIMEOUT, request.send()).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(ClientError::Http(format!("{failure_context}: {error}"))),
        Err(_) => Err(ClientError::Http(format!(
            "timed out waiting for SSE response headers from {url}"
        ))),
    }
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

fn message_indicates_unreachable(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("status 502")
        || message.contains("status 503")
        || message.contains("status 504")
        || message.contains("bad gateway")
        || message.contains("service unavailable")
        || message.contains("gateway timeout")
        || message.contains("error sending request")
        || message.contains("error trying to connect")
        || message.contains("connect error")
        || message.contains("tcp connect error")
        || message.contains("connection refused")
        || message.contains("connection reset")
        || message.contains("connection aborted")
        || message.contains("connection closed")
        || message.contains("network is unreachable")
        || message.contains("failed to lookup address")
        || message.contains("dns error")
        || message.contains("timed out")
        || message.contains("timeout")
        || message.contains("broken pipe")
        || message.contains("eof")
        || message.contains("end of file")
}

fn message_indicates_auth_stale(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("status 401")
        || message.contains("status 403")
        || message.contains("unauthorized")
        || message.contains("forbidden")
        || message.contains("invalid token")
        || message.contains("expired token")
        || message.contains("stale token")
        || message.contains("invalid bearer")
        || message.contains("authorization failed")
}

fn build_plain_http_client(bypass_proxy: bool) -> Result<reqwest::Client, ClientError> {
    let builder = reqwest::Client::builder()
        .connect_timeout(PLAIN_HTTP_CONNECT_TIMEOUT)
        .timeout(PLAIN_HTTP_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    apply_proxy_bypass(builder, bypass_proxy)
        .build()
        .map_err(|error| ClientError::Http(format!("failed to build HTTP client: {error}")))
}

fn build_sse_http_client(bypass_proxy: bool) -> Result<reqwest::Client, ClientError> {
    let builder = reqwest::Client::builder()
        .connect_timeout(SSE_HTTP_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    apply_proxy_bypass(builder, bypass_proxy)
        .build()
        .map_err(|error| ClientError::Http(format!("failed to build SSE client: {error}")))
}

fn apply_proxy_bypass(
    builder: reqwest::ClientBuilder,
    bypass_proxy: bool,
) -> reqwest::ClientBuilder {
    if bypass_proxy {
        builder.no_proxy()
    } else {
        builder
    }
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

fn daemon_base_url_from_bind(bind: &str, port: u16) -> String {
    let host = connect_host(bind);
    format!("http://{}:{port}", host_for_url(host))
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

fn host_for_url(host: &str) -> String {
    if host.parse::<IpAddr>().is_ok_and(|addr| addr.is_ipv6()) {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn trim_base_url(base_url: String) -> String {
    base_url.trim_end_matches('/').to_string()
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn same_origin(left: &str, right: &str) -> bool {
    base_url_origin(left)
        .zip(base_url_origin(right))
        .is_some_and(|(left, right)| left == right)
}

fn base_url_origin(base_url: &str) -> Option<(String, String, u16)> {
    let url = Url::parse(base_url).ok()?;
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    Some((scheme, host, port))
}

fn should_bypass_proxy_for_base_url(base_url: &str) -> bool {
    let Some((scheme, host, _)) = base_url_origin(base_url) else {
        return false;
    };
    if !matches!(scheme.as_str(), "http" | "https") {
        return false;
    }
    if host == "localhost" {
        return true;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse::<IpAddr>().is_ok_and(|addr| addr.is_loopback())
}

fn encode_query_value(value: &str) -> String {
    url_encode(value)
}

fn encode_path_segment(value: &str) -> String {
    url_encode(value).replace('+', "%20")
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

fn providers_path(project_id: &str) -> String {
    format!("/p/{}/v1/providers", encode_path_segment(project_id))
}

fn provider_oauth_logout_path(project_id: &str, provider: &str) -> String {
    format!(
        "/p/{}/v1/providers/{}/oauth/logout",
        encode_path_segment(project_id),
        encode_path_segment(provider)
    )
}

fn hooks_path(project_id: &str) -> String {
    format!("/p/{}/v1/ext/hooks", encode_path_segment(project_id))
}

fn competitor_import_path(project_id: &str) -> String {
    format!(
        "/p/{}/v1/ext/competitor-import",
        encode_path_segment(project_id)
    )
}

fn competitor_import_body(source: Option<&str>, scope: &str) -> Value {
    match source.filter(|source| !source.trim().is_empty()) {
        Some(source) => json!({"source": source, "scope": scope}),
        None => json!({"scope": scope}),
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    struct TestServer {
        base_url: String,
        stop: mpsc::Sender<()>,
        handle: thread::JoinHandle<()>,
    }

    impl TestServer {
        fn stop(self) {
            let _ = self.stop.send(());
            let _ = self.handle.join();
        }
    }

    fn spawn_stalled_header_server() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                let _ = stopped.recv_timeout(Duration::from_secs(5));
            }
        });
        TestServer {
            base_url: format!("http://{addr}"),
            stop,
            handle,
        }
    }

    fn spawn_stalled_error_body_server() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                read_request_headers(&mut stream);
                let _ = stream.write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 1024\r\n\r\n",
                );
                let _ = stream.flush();
                let _ = stopped.recv_timeout(Duration::from_secs(5));
            }
        });
        TestServer {
            base_url: format!("http://{addr}"),
            stop,
            handle,
        }
    }

    fn spawn_delayed_sse_body_server() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                read_request_headers(&mut stream);
                let _ =
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
                let _ = stream.flush();
                thread::sleep(SSE_HEADER_TIMEOUT + Duration::from_millis(100));
                let _ = stream
                    .write_all(b"data: {\"chat_id\":\"chat\",\"seq\":1,\"type\":\"snapshot\"}\n\n");
                let _ = stream.flush();
                let _ = stopped.recv_timeout(Duration::from_secs(5));
            }
        });
        TestServer {
            base_url: format!("http://{addr}"),
            stop,
            handle,
        }
    }

    fn spawn_chat_sse_bytes_server(body: &'static [u8]) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_vec();
        let (stop, stopped) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                read_request_headers(&mut stream);
                let _ =
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
            let _ = stopped.recv_timeout(Duration::from_millis(10));
        });
        TestServer {
            base_url: format!("http://{addr}"),
            stop,
            handle,
        }
    }

    fn spawn_open_project_server(response: Value) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                read_request_headers(&mut stream);
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(), body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            let _ = stopped.recv_timeout(Duration::from_millis(10));
        });
        TestServer {
            base_url: format!("http://{addr}"),
            stop,
            handle,
        }
    }

    fn read_request_headers(stream: &mut std::net::TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut request = Vec::new();
        let mut buffer = [0; 256];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => request.extend_from_slice(&buffer[..read]),
                Err(_) => break,
            }
        }
        let _ = stream.set_read_timeout(None);
    }

    #[test]
    fn sse_parser_handles_partial_frames() {
        let mut parser = SseLineParser::default();
        assert!(parser.push("data: {\"a\"").is_empty());
        assert_eq!(parser.push(":1}\n\n"), vec!["{\"a\":1}".to_string()]);
        assert!(parser.push("data: one\n").is_empty());
        assert_eq!(parser.push("data: two\n\n"), vec!["one\ntwo".to_string()]);
        assert!(parser.push("data: three\n").is_empty());
        assert_eq!(parser.finish(), vec!["three".to_string()]);
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

    #[tokio::test]
    async fn subscribe_chat_times_out_waiting_for_sse_headers() {
        let server = spawn_stalled_header_server();
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client.subscribe_chat("project", "chat"),
        )
        .await;
        server.stop();
        let error = match result {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("stalled SSE header subscription unexpectedly succeeded"),
            Err(_) => panic!("stalled SSE header subscription exceeded outer timeout"),
        };
        assert!(started.elapsed() < Duration::from_secs(2));
        match error {
            ClientError::Http(message) => {
                assert!(message.contains("timed out waiting for SSE response headers"));
                assert!(message.contains("/p/project/v1/chats/subscribe"));
            }
            other => panic!("expected HTTP timeout error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn subscribe_daemon_events_times_out_waiting_for_sse_headers() {
        let server = spawn_stalled_header_server();
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let started = Instant::now();
        let result =
            tokio::time::timeout(Duration::from_secs(2), client.subscribe_daemon_events()).await;
        server.stop();
        let error = match result {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("stalled daemon event subscription unexpectedly succeeded"),
            Err(_) => panic!("stalled daemon event subscription exceeded outer timeout"),
        };
        assert!(started.elapsed() < Duration::from_secs(2));
        match error {
            ClientError::Http(message) => {
                assert!(message.contains("timed out waiting for SSE response headers"));
                assert!(message.contains("/daemon/v1/events"));
            }
            other => panic!("expected HTTP timeout error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sse_error_body_read_times_out_to_status_only_message() {
        let server = spawn_stalled_error_body_server();
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client.subscribe_chat("project", "chat"),
        )
        .await;
        server.stop();
        let error = match result {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("stalled error-body subscription unexpectedly succeeded"),
            Err(_) => panic!("stalled error-body subscription exceeded outer timeout"),
        };
        match error {
            ClientError::Status { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "status 500");
            }
            other => panic!("expected status error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sse_body_stream_remains_unbounded_after_headers() {
        let server = spawn_delayed_sse_body_server();
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let mut stream = client.subscribe_chat("project", "chat").await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.stop();
        assert_eq!(event.chat_id.as_deref(), Some("chat"));
        assert_eq!(event.seq, Some(1));
        assert_eq!(event.kind, "snapshot");
    }

    #[tokio::test]
    async fn sse_eof_flushes_buffered_event_then_reports_disconnect() {
        let server = spawn_chat_sse_bytes_server(
            b"data: {\"chat_id\":\"chat\",\"seq\":0,\"type\":\"snapshot\"}\n",
        );
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let mut stream = client.subscribe_chat("project", "chat").await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        let error = stream.next().await.unwrap().unwrap_err();
        server.stop();

        assert_eq!(event.kind, "snapshot");
        assert_eq!(event.seq, Some(0));
        assert!(matches!(error, ClientError::SseDisconnect(_)));
        assert!(error.is_unreachable());
    }

    #[tokio::test]
    async fn sse_mid_event_eof_surfaces_parse_error() {
        let server = spawn_chat_sse_bytes_server(b"data: {\"chat_id\":\"chat\"");
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let mut stream = client.subscribe_chat("project", "chat").await.unwrap();

        let error = stream.next().await.unwrap().unwrap_err();
        server.stop();

        assert!(matches!(error, ClientError::Json(_)));
    }

    #[tokio::test]
    async fn client_error_classifiers_cover_recovery_cases() {
        for status in [502, 503, 504] {
            assert!(ClientError::Status {
                status,
                body: "worker waking".to_string(),
            }
            .is_unreachable());
        }
        assert!(ClientError::SseDisconnect("eof".to_string()).is_unreachable());
        assert!(ClientError::Status {
            status: 401,
            body: "Unauthorized".to_string(),
        }
        .is_auth_stale());
        assert!(ClientError::Status {
            status: 403,
            body: "Forbidden".to_string(),
        }
        .is_auth_stale());

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client = DaemonClient::new(format!("http://{addr}"), None).unwrap();
        let error = client.status().await.unwrap_err();

        assert!(error.is_unreachable(), "{error:?}");
    }

    #[tokio::test]
    async fn open_project_requires_ready_worker_with_ports() {
        let server = spawn_open_project_server(json!({
            "project_id": "p1",
            "slug": "fixture",
            "root": "/tmp/fixture",
            "pinned": false,
            "worker": {
                "project_id": "p1",
                "pid": 7,
                "http_port": 31000,
                "lsp_port": 31001,
                "state": "starting",
                "last_error": null
            },
            "cron_pending": null
        }));
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let error = client
            .open_project(Path::new("/tmp/fixture"))
            .await
            .unwrap_err();
        server.stop();

        match error {
            ClientError::WorkerNotReady(message) => assert!(message.contains("starting")),
            other => panic!("expected worker readiness error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn open_project_accepts_ready_worker_with_nonzero_ports() {
        let server = spawn_open_project_server(json!({
            "project_id": "p1",
            "slug": "fixture",
            "root": "/tmp/fixture",
            "pinned": false,
            "worker": {
                "project_id": "p1",
                "pid": 7,
                "http_port": 31000,
                "lsp_port": 31001,
                "state": "ready",
                "last_error": null
            },
            "cron_pending": null
        }));
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let project = client
            .open_project(Path::new("/tmp/fixture"))
            .await
            .unwrap();
        server.stop();

        assert_eq!(project.project_id, "p1");
        assert_eq!(project.worker.unwrap().http_port, 31000);
    }

    #[test]
    fn explicit_base_url_origin_change_drops_discovered_token_unless_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        std::fs::write(
            &path,
            r#"{"pid":7,"port":43123,"bind":"127.0.0.1","version":"9.9.9","auth_token":"secret-token"}"#,
        )
        .unwrap();

        let changed = resolve_daemon_endpoint_from_path(
            &path,
            Some("http://127.0.0.1:45454".to_string()),
            None,
        )
        .unwrap();
        let same = resolve_daemon_endpoint_from_path(
            &path,
            Some("http://127.0.0.1:43123".to_string()),
            None,
        )
        .unwrap();
        let explicit = resolve_daemon_endpoint_from_path(
            &path,
            Some("http://127.0.0.1:45454".to_string()),
            Some("explicit-token".to_string()),
        )
        .unwrap();

        assert_eq!(changed.base_url, "http://127.0.0.1:45454");
        assert_eq!(changed.auth_token, None);
        assert_eq!(same.auth_token.as_deref(), Some("secret-token"));
        assert_eq!(explicit.auth_token.as_deref(), Some("explicit-token"));
    }

    #[test]
    fn loopback_base_urls_bypass_ambient_proxies() {
        assert!(should_bypass_proxy_for_base_url("http://127.0.0.1:8488"));
        assert!(should_bypass_proxy_for_base_url("http://[::1]:8488"));
        assert!(should_bypass_proxy_for_base_url("http://localhost:8488"));
        assert!(!should_bypass_proxy_for_base_url("http://192.0.2.10:8488"));
        assert!(!should_bypass_proxy_for_base_url(
            "https://daemon.example:8488"
        ));
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

    #[test]
    fn provider_logout_client_paths_encode_project_and_provider() {
        assert_eq!(providers_path("abc/def"), "/p/abc%2Fdef/v1/providers");
        assert_eq!(
            provider_oauth_logout_path("abc/def", "openai codex"),
            "/p/abc%2Fdef/v1/providers/openai%20codex/oauth/logout"
        );
    }

    #[test]
    fn hooks_and_competitor_import_client_paths_use_project_proxy() {
        assert_eq!(hooks_path("p1"), "/p/p1/v1/ext/hooks");
        assert_eq!(
            competitor_import_path("p1"),
            "/p/p1/v1/ext/competitor-import"
        );
        assert_eq!(
            competitor_import_body(Some("claude_code"), "project"),
            json!({"source":"claude_code","scope":"project"})
        );
        assert_eq!(
            competitor_import_body(None, "global"),
            json!({"scope":"global"})
        );
    }

    #[test]
    fn hooks_and_import_fixtures_parse_backend_responses() {
        let hooks: HooksResponse = serde_json::from_str(
            r#"{
                "hooks":[{"event":"PreToolUse","matcher":"Bash","command":"./check.sh","timeout":30}],
                "raw_content":"hooks: {}",
                "file_path":"/repo/.refact/hooks.yaml"
            }"#,
        )
        .unwrap();
        assert_eq!(hooks.hooks[0].event, "PreToolUse");
        assert_eq!(hooks.hooks[0].timeout, Some(30));

        let info: CompetitorImportInfoResponse = serde_json::from_str(
            r#"{"sources":[{"id":"claude_code","label":"Claude Code","roots":["~/.claude"]}]}"#,
        )
        .unwrap();
        assert_eq!(info.sources[0].id, "claude_code");

        let run: CompetitorImportRunResponse = serde_json::from_str(
            r#"{
                "scope":"project",
                "source":"claude_code",
                "report":{"discovered_candidates":1,"status_counts":{"created":1},"competitor_counts":{},"kind_counts":{},"top_issues":[]}
            }"#,
        )
        .unwrap();
        assert_eq!(
            run.report.status_counts.get(&ImportStatus::Created),
            Some(&1)
        );
    }
}
