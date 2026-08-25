use std::collections::{HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::events_pane::{parse_daemon_event, DaemonEventRecord};
use crate::protocol::SseEvent;
use crate::sessions::{PaginatedTrajectories, TrajectoryMeta};

const DEFAULT_DAEMON_PORT: u16 = 8488;
const PLAIN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PLAIN_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const OPEN_PROJECT_READINESS_TIMEOUT: Duration = Duration::from_secs(125);
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
#[cfg(not(test))]
const OPEN_PROJECT_STARTING_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
#[cfg(test)]
const OPEN_PROJECT_STARTING_INITIAL_BACKOFF: Duration = Duration::from_millis(1);
const OPEN_PROJECT_STARTING_MAX_BACKOFF: Duration = Duration::from_secs(2);
const OPEN_PROJECT_STARTING_MAX_ATTEMPTS: u32 = 65;
const TRAJECTORIES_PAGE_SIZE: usize = 200;

#[derive(Debug, Clone, Copy)]
struct OpenProjectRetryPolicy {
    deadline: Duration,
    initial_backoff: Duration,
    max_backoff: Duration,
    max_attempts: u32,
}

impl OpenProjectRetryPolicy {
    fn production() -> Self {
        Self {
            deadline: OPEN_PROJECT_READINESS_TIMEOUT,
            initial_backoff: OPEN_PROJECT_STARTING_INITIAL_BACKOFF,
            max_backoff: OPEN_PROJECT_STARTING_MAX_BACKOFF,
            max_attempts: OPEN_PROJECT_STARTING_MAX_ATTEMPTS,
        }
    }
}

fn deserialize_default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonInfoFile {
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub bind: Option<String>,
    pub version: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub auth_token: Option<String>,
}

#[derive(Clone)]
pub struct DaemonClient {
    base_url: String,
    auth_token: Option<String>,
    client: reqwest::Client,
    readiness_client: reqwest::Client,
    sse_client: reqwest::Client,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonStatus {
    pub pid: Option<u32>,
    pub version: Option<String>,
    pub port: Option<u16>,
    pub started_at_ms: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub workers: Option<u64>,
    pub cron_pending: Option<HashMap<String, u64>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub slug: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub root: PathBuf,
    pub pinned: Option<bool>,
    pub last_active_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub settings: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenProjectResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub project_id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub slug: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub root: PathBuf,
    pub pinned: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub worker: Option<WorkerInfo>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub cron_pending: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub project_id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub slug: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub root: PathBuf,
    pub root_exists: Option<bool>,
    pub pinned: Option<bool>,
    pub last_active_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub state: Value,
    pub pid: Option<u32>,
    pub rss_bytes: Option<u64>,
    pub cpu_percent: Option<f32>,
    pub uptime_secs: Option<u64>,
    pub http_port: Option<u16>,
    pub lsp_port: Option<u16>,
    pub lsp_clients: Option<usize>,
    pub busy_chats: Option<usize>,
    pub exec_running: Option<usize>,
    pub live_proxy_streams: Option<u64>,
    pub cron_next_fire_ms: Option<u64>,
    pub idle_deadline_ms: Option<u64>,
    pub last_status_report_ms: Option<u64>,
    pub last_error: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub log_path: String,
}

impl Default for WorkerInfo {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            slug: String::new(),
            root: PathBuf::new(),
            root_exists: None,
            pinned: None,
            last_active_ms: None,
            state: Value::Null,
            pid: None,
            rss_bytes: None,
            cpu_percent: None,
            uptime_secs: None,
            http_port: None,
            lsp_port: None,
            lsp_clients: None,
            busy_chats: None,
            exec_running: None,
            live_proxy_streams: None,
            cron_next_fire_ms: None,
            idle_deadline_ms: None,
            last_status_report_ms: None,
            last_error: None,
            log_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct AtCommandCompletionResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    completions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IntegrationListResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub integrations: Vec<IntegrationRecord>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub error_log: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IntegrationRecord {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub project_path: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub integr_name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub integr_config_path: String,
    pub integr_config_exists: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub config_unparsed: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpViewData {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub servers: Vec<McpServerSummary>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub error_log: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpServerSummary {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub transport: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub project_path: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub config_path: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<McpServerInfoResponse>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpServerInfoResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub config_path: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub status: Value,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub auth_status: Value,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub server_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub server_version: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub protocol_version: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub tools: Vec<McpToolInfo>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub resources: Vec<McpResourceInfo>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub prompts: Vec<McpPromptInfo>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub capabilities: Value,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub logs_tail: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub metrics: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpToolInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub description: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub input_schema: Value,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub annotations: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub internal_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpResourceInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub uri: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpPromptInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SlashCommandsListResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub commands: Vec<SlashCommandInfo>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub skills: Vec<SkillInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlashCommandInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub description: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub argument_hint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub description: String,
    pub user_invocable: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderListResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub providers: Vec<ProviderListItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderListItem {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub base_provider: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub display_name: String,
    pub enabled: Option<bool>,
    pub readonly: Option<bool>,
    pub has_credentials: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub status: String,
    pub model_count: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderOAuthLogoutResponse {
    pub success: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub auth_status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HooksResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub hooks: Vec<HookInfo>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub raw_content: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub file_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub event: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub matcher: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub command: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompetitorImportInfoResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub sources: Vec<CompetitorImportSourceInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompetitorImportSourceInfo {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub label: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReportCounts {
    pub discovered: Option<usize>,
    pub created: Option<usize>,
    pub updated: Option<usize>,
    pub unchanged: Option<usize>,
    pub stale: Option<usize>,
    pub conflicts: Option<usize>,
    pub user_modified: Option<usize>,
    pub unsupported: Option<usize>,
    pub errors: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportReportIssue {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub competitor: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub path: Option<String>,
    pub status: Option<ImportStatus>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportReport {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub completed_at: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub reported_sources: Vec<Value>,
    pub discovered_candidates: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub status_counts: std::collections::BTreeMap<ImportStatus, usize>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub competitor_counts: std::collections::BTreeMap<String, ImportReportCounts>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub kind_counts: std::collections::BTreeMap<String, ImportReportCounts>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub top_issues: Vec<ImportReportIssue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CompetitorImportRunResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub scope: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub source: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub report: ImportReport,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeGraphResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub nodes: Vec<KnowledgeNode>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub edges: Vec<KnowledgeEdge>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub stats: KnowledgeStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeNode {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub node_type: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub label: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub created: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub file_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeEdge {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub source: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub target: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub edge_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeStats {
    pub doc_count: Option<usize>,
    pub tag_count: Option<usize>,
    pub file_count: Option<usize>,
    pub entity_count: Option<usize>,
    pub edge_count: Option<usize>,
    pub active_docs: Option<usize>,
    pub deprecated_docs: Option<usize>,
    pub trajectory_count: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskBoardTask {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskBoardColumn {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskBoardCard {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub column: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub priority: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub depends_on: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub instructions: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub assignee: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub agent_chat_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub final_report: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub final_report_structured: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub verifier_report: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskBoardResponse {
    #[serde(default)]
    pub rev: u64,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub columns: Vec<TaskBoardColumn>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub cards: Vec<TaskBoardCard>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskBoardReadyCards {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub ready: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub blocked: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub in_progress: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub completed: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct TaskBoardReadyResponse {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    ready: TaskBoardReadyCards,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskBoardViewData {
    pub task: TaskBoardTask,
    pub board: TaskBoardResponse,
    pub ready: TaskBoardReadyCards,
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
    Suppress,
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
        if matches!(
            event.protocol_event(),
            SseEvent::MalformedStreamDelta { .. }
        ) {
            return ChatSeqDecision::Resubscribe(format!(
                "malformed stream_delta at seq {seq}: missing or non-array ops"
            ));
        }
        if let SseEvent::Unknown { event } = event.protocol_event() {
            if let Some(reason) = event.malformed_reason {
                return ChatSeqDecision::Resubscribe(format!(
                    "malformed {} at seq {seq}: {reason}",
                    event.kind
                ));
            }
        }
        if event.kind == "snapshot" {
            if self.last_seq.is_some_and(|last_seq| seq <= last_seq) {
                return ChatSeqDecision::Suppress;
            }
            self.last_seq = Some(seq);
            return ChatSeqDecision::Apply;
        }
        match self.last_seq {
            Some(last_seq) if seq <= last_seq => ChatSeqDecision::Suppress,
            Some(last_seq) => match last_seq.checked_add(1) {
                Some(expected) if seq == expected => {
                    self.last_seq = Some(seq);
                    ChatSeqDecision::Apply
                }
                Some(expected) => ChatSeqDecision::Resubscribe(format!(
                    "SSE seq mismatch: expected {expected}, got {seq} for {} event",
                    event.kind
                )),
                None => ChatSeqDecision::Resubscribe(format!(
                    "SSE seq overflow after {last_seq} for {} event",
                    event.kind
                )),
            },
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

    fn from_info(info: DaemonInfoFile) -> Option<Self> {
        let port = info.port.filter(|port| *port > 0)?;
        Some(Self {
            base_url: daemon_base_url_from_bind(info.bind.as_deref().unwrap_or_default(), port),
            auth_token: info.auth_token.filter(|token| !token.is_empty()),
        })
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
            Ok(info) => Ok(DaemonEndpoint::from_info(info)),
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
        let readiness_client = build_readiness_http_client(bypass_proxy)?;
        let sse_client = build_sse_http_client(bypass_proxy)?;
        Ok(Self {
            base_url,
            auth_token,
            client,
            readiness_client,
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
        let projects: Vec<ProjectEntry> = self.get_json("/daemon/v1/projects").await?;
        Ok(projects
            .into_iter()
            .filter(|project| project_has_root(project))
            .collect())
    }

    pub async fn open_project(&self, root: &Path) -> Result<OpenProjectResponse, ClientError> {
        self.open_project_with_retry_policy(root, OpenProjectRetryPolicy::production())
            .await
    }

    async fn open_project_with_retry_policy(
        &self,
        root: &Path,
        policy: OpenProjectRetryPolicy,
    ) -> Result<OpenProjectResponse, ClientError> {
        let deadline = tokio::time::Instant::now() + policy.deadline;
        let mut last_starting = None;

        for attempt in 0..policy.max_attempts {
            let project = tokio::time::timeout_at(deadline, async {
                let response = self
                    .with_auth(
                        self.readiness_client
                            .post(self.url("/daemon/v1/projects/open")),
                    )
                    .json(&json!({"root": root.to_string_lossy()}))
                    .send()
                    .await
                    .map_err(|error| {
                        ClientError::Http(format!("failed to open project: {error}"))
                    })?;
                decode_response(response).await
            })
            .await
            .map_err(|_| ClientError::WorkerNotReady("worker readiness timed out".to_string()))??;

            validate_open_project_root(&project)?;
            if !open_project_worker_is_starting(&project) {
                validate_open_project_response(&project)?;
                return Ok(project);
            }

            last_starting = Some(project);
            if attempt + 1 == policy.max_attempts {
                break;
            }
            let backoff = open_project_starting_backoff(policy, attempt);
            if tokio::time::Instant::now() + backoff >= deadline {
                break;
            }
            tokio::time::sleep(backoff).await;
        }

        match last_starting {
            Some(project) => validate_open_project_response(&project).map(|()| project),
            None => Err(ClientError::WorkerNotReady(
                "worker readiness attempt limit is zero".to_string(),
            )),
        }
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

    pub async fn task_board_view(
        &self,
        project_id: &str,
        task_id: Option<&str>,
    ) -> Result<TaskBoardViewData, ClientError> {
        let tasks: Vec<TaskBoardTask> = self.get_json(&tasks_path(project_id)).await?;
        let task = match task_id.filter(|task_id| !task_id.trim().is_empty()) {
            Some(task_id) => tasks
                .into_iter()
                .find(|task| task.id == task_id)
                .ok_or_else(|| {
                    ClientError::Json(format!("task board is unavailable for task {task_id}"))
                })?,
            None => tasks
                .iter()
                .find(|task| task.status == "active")
                .cloned()
                .or_else(|| tasks.into_iter().next())
                .ok_or_else(|| ClientError::Json("no task boards are available".to_string()))?,
        };
        let board = self
            .get_json(&task_board_path(project_id, &task.id))
            .await?;
        let ready: TaskBoardReadyResponse = self
            .get_json(&task_board_ready_path(project_id, &task.id))
            .await?;
        Ok(TaskBoardViewData {
            task,
            board,
            ready: ready.ready,
        })
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
        cursor: Option<&str>,
    ) -> Result<PaginatedTrajectories, ClientError> {
        let mut path = format!(
            "/p/{}/v1/trajectories?displayable_only=true&limit={}",
            encode_path_segment(project_id),
            limit.clamp(1, 200)
        );
        if let Some(cursor) = cursor {
            path.push_str("&cursor=");
            path.push_str(&encode_query_value(cursor));
        }
        self.get_json(&path).await
    }

    pub async fn list_all_trajectories(
        &self,
        project_id: &str,
    ) -> Result<Vec<TrajectoryMeta>, ClientError> {
        let mut items = Vec::new();
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();

        loop {
            let page = self
                .list_trajectories(project_id, TRAJECTORIES_PAGE_SIZE, cursor.as_deref())
                .await?;
            items.extend(page.items);
            if page.has_more != Some(true) {
                return Ok(items);
            }
            let next_cursor = page.next_cursor.ok_or_else(|| {
                ClientError::Json("trajectory page has_more without next_cursor".to_string())
            })?;
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(ClientError::Json(format!(
                    "trajectory page repeated next_cursor: {next_cursor}"
                )));
            }
            cursor = Some(next_cursor);
        }
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
        client_request_id: &str,
        client_message_id: &str,
        content: &str,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "client_message_id": client_message_id,
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

    pub async fn send_set_goal(
        &self,
        project_id: &str,
        chat_id: &str,
        content: &str,
        budget: Option<GoalBudget>,
    ) -> Result<(), ClientError> {
        self.send_set_goal_with_id(
            project_id,
            chat_id,
            &request_id("set-goal"),
            content,
            budget,
        )
        .await
    }

    pub async fn send_set_goal_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        content: &str,
        budget: Option<GoalBudget>,
    ) -> Result<(), ClientError> {
        let body = match budget {
            Some(budget) => json!({
                "client_request_id": client_request_id,
                "type": "set_goal",
                "content": content,
                "budget": budget,
            }),
            None => json!({
                "client_request_id": client_request_id,
                "type": "set_goal",
                "content": content,
            }),
        };
        self.send_command(project_id, chat_id, body).await
    }

    pub async fn send_set_goal_budget(
        &self,
        project_id: &str,
        chat_id: &str,
        budget: GoalBudget,
    ) -> Result<(), ClientError> {
        self.send_set_goal_budget_with_id(
            project_id,
            chat_id,
            &request_id("set-goal-budget"),
            budget,
        )
        .await
    }

    pub async fn send_set_goal_budget_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        budget: GoalBudget,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "set_goal_budget",
                "budget": budget,
            }),
        )
        .await
    }

    pub async fn send_update_goal(
        &self,
        project_id: &str,
        chat_id: &str,
        note: &str,
    ) -> Result<(), ClientError> {
        self.send_update_goal_with_id(project_id, chat_id, &request_id("update-goal"), note)
            .await
    }

    pub async fn send_update_goal_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        note: &str,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "update_goal",
                "note": note,
            }),
        )
        .await
    }

    pub async fn send_goal_control(
        &self,
        project_id: &str,
        chat_id: &str,
        action: &str,
    ) -> Result<(), ClientError> {
        self.send_goal_control_with_id(project_id, chat_id, &request_id("goal-control"), action)
            .await
    }

    pub async fn send_goal_control_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        action: &str,
    ) -> Result<(), ClientError> {
        let action = GoalControlAction::try_from(action)
            .map_err(|error| ClientError::Json(error.to_string()))?;
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "goal_control",
                "action": action,
            }),
        )
        .await
    }

    pub async fn send_regenerate(
        &self,
        project_id: &str,
        chat_id: &str,
    ) -> Result<(), ClientError> {
        self.send_regenerate_with_id(project_id, chat_id, &request_id("regenerate"))
            .await
    }

    pub async fn send_regenerate_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "regenerate",
            }),
        )
        .await
    }

    pub async fn send_update_message(
        &self,
        project_id: &str,
        chat_id: &str,
        message_id: &str,
        content: Value,
        attachments: Vec<Value>,
        regenerate: bool,
    ) -> Result<(), ClientError> {
        self.send_update_message_with_id(
            project_id,
            chat_id,
            &request_id("update-message"),
            message_id,
            content,
            attachments,
            regenerate,
        )
        .await
    }

    pub async fn send_update_message_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        message_id: &str,
        content: Value,
        attachments: Vec<Value>,
        regenerate: bool,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "update_message",
                "message_id": message_id,
                "content": content,
                "attachments": attachments,
                "regenerate": regenerate,
            }),
        )
        .await
    }

    pub async fn send_remove_message(
        &self,
        project_id: &str,
        chat_id: &str,
        message_id: &str,
        regenerate: bool,
    ) -> Result<(), ClientError> {
        self.send_remove_message_with_id(
            project_id,
            chat_id,
            &request_id("remove-message"),
            message_id,
            regenerate,
        )
        .await
    }

    pub async fn send_remove_message_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        message_id: &str,
        regenerate: bool,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "remove_message",
                "message_id": message_id,
                "regenerate": regenerate,
            }),
        )
        .await
    }

    pub async fn send_restore_messages(
        &self,
        project_id: &str,
        chat_id: &str,
        messages: Vec<Value>,
    ) -> Result<(), ClientError> {
        self.send_restore_messages_with_id(
            project_id,
            chat_id,
            &request_id("restore-messages"),
            messages,
        )
        .await
    }

    pub async fn send_restore_messages_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        messages: Vec<Value>,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "restore_messages",
                "messages": messages,
            }),
        )
        .await
    }

    pub async fn send_ide_tool_result(
        &self,
        project_id: &str,
        chat_id: &str,
        tool_call_id: &str,
        content: &str,
        tool_failed: bool,
    ) -> Result<(), ClientError> {
        self.send_ide_tool_result_with_id(
            project_id,
            chat_id,
            &request_id("ide-tool-result"),
            tool_call_id,
            content,
            tool_failed,
        )
        .await
    }

    pub async fn send_ide_tool_result_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        tool_call_id: &str,
        content: &str,
        tool_failed: bool,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "ide_tool_result",
                "tool_call_id": tool_call_id,
                "content": content,
                "tool_failed": tool_failed,
            }),
        )
        .await
    }

    pub async fn send_tool_decision(
        &self,
        project_id: &str,
        chat_id: &str,
        tool_call_id: &str,
        accepted: bool,
    ) -> Result<(), ClientError> {
        self.send_tool_decision_with_id(
            project_id,
            chat_id,
            &request_id("tool-decision"),
            tool_call_id,
            accepted,
        )
        .await
    }

    pub async fn send_tool_decision_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        tool_call_id: &str,
        accepted: bool,
    ) -> Result<(), ClientError> {
        self.send_tool_decisions_with_id(
            project_id,
            chat_id,
            client_request_id,
            vec![ToolDecision {
                tool_call_id: tool_call_id.to_string(),
                accepted,
            }],
        )
        .await
    }

    pub async fn send_tool_decisions(
        &self,
        project_id: &str,
        chat_id: &str,
        decisions: Vec<ToolDecision>,
    ) -> Result<(), ClientError> {
        self.send_tool_decisions_with_id(
            project_id,
            chat_id,
            &request_id("tool-decisions"),
            decisions,
        )
        .await
    }

    pub async fn send_tool_decisions_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        decisions: Vec<ToolDecision>,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "tool_decisions",
                "decisions": decisions,
            }),
        )
        .await
    }

    pub async fn send_clean_background_processes(
        &self,
        project_id: &str,
        chat_id: &str,
        include_services: bool,
    ) -> Result<(), ClientError> {
        self.send_clean_background_processes_with_id(
            project_id,
            chat_id,
            &request_id("clean-background-processes"),
            include_services,
        )
        .await
    }

    pub async fn send_clean_background_processes_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        include_services: bool,
    ) -> Result<(), ClientError> {
        self.send_command(
            project_id,
            chat_id,
            json!({
                "client_request_id": client_request_id,
                "type": "clean_background_processes",
                "include_services": include_services,
            }),
        )
        .await
    }

    pub async fn send_browser_context_decision(
        &self,
        project_id: &str,
        chat_id: &str,
        decision: BrowserContextDecision,
    ) -> Result<(), ClientError> {
        self.send_browser_context_decision_with_id(
            project_id,
            chat_id,
            &request_id("browser-context-decision"),
            decision,
        )
        .await
    }

    pub async fn send_browser_context_decision_with_id(
        &self,
        project_id: &str,
        chat_id: &str,
        client_request_id: &str,
        decision: BrowserContextDecision,
    ) -> Result<(), ClientError> {
        let body = serde_json::to_value(decision).map_err(|error| {
            ClientError::Json(format!("failed to serialize browser decision: {error}"))
        })?;
        let Value::Object(mut body) = body else {
            return Err(ClientError::Json(
                "browser context decision did not serialize to an object".to_string(),
            ));
        };
        body.insert(
            "client_request_id".to_string(),
            Value::String(client_request_id.to_string()),
        );
        body.insert(
            "type".to_string(),
            Value::String("browser_context_decision".to_string()),
        );
        self.send_command(project_id, chat_id, Value::Object(body))
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
                        pending_events.extend(
                            parser
                                .finish()
                                .into_iter()
                                .filter(|data| !data.trim().is_empty())
                                .map(Ok),
                        );
                        if !pending_utf8.is_empty() {
                            pending_events.push_back(Err(ClientError::Sse(
                                "incomplete UTF-8 sequence at SSE EOF".to_string(),
                            )));
                        } else {
                            pending_events.push_back(Err(ClientError::SseDisconnect(
                                "stream ended before the subscription was closed cleanly"
                                    .to_string(),
                            )));
                        }
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
    validate_open_project_root(project)?;
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
    let http_port = worker.http_port.unwrap_or_default();
    let lsp_port = worker.lsp_port.unwrap_or_default();
    if http_port == 0 || lsp_port == 0 {
        return Err(ClientError::WorkerNotReady(format!(
            "project {} worker has invalid ports http={} lsp={}",
            project.project_id, http_port, lsp_port
        )));
    }
    Ok(())
}

fn validate_open_project_root(project: &OpenProjectResponse) -> Result<(), ClientError> {
    if project.root.as_os_str().is_empty() {
        return Err(ClientError::Json(format!(
            "project {} opened with an empty root",
            project.project_id
        )));
    }
    Ok(())
}

fn project_has_root(project: &ProjectEntry) -> bool {
    !project.root.as_os_str().is_empty()
}

fn open_project_worker_is_starting(project: &OpenProjectResponse) -> bool {
    worker_state_label(project.worker.as_ref()).eq_ignore_ascii_case("starting")
}

fn open_project_starting_backoff(policy: OpenProjectRetryPolicy, retry: u32) -> Duration {
    policy
        .initial_backoff
        .saturating_mul(1u32 << retry.min(8))
        .min(policy.max_backoff)
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
        || has_word(message.as_bytes(), b"eof")
        || message.contains("end of file")
}

fn has_word(message: &[u8], word: &[u8]) -> bool {
    message
        .windows(word.len())
        .enumerate()
        .any(|(start, part)| {
            part == word
                && message
                    .get(start.wrapping_sub(1))
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
                && message
                    .get(start + word.len())
                    .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        })
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

fn build_readiness_http_client(bypass_proxy: bool) -> Result<reqwest::Client, ClientError> {
    let builder = reqwest::Client::builder()
        .connect_timeout(PLAIN_HTTP_CONNECT_TIMEOUT)
        .timeout(OPEN_PROJECT_READINESS_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    apply_proxy_bypass(builder, bypass_proxy)
        .build()
        .map_err(|error| {
            ClientError::Http(format!("failed to build readiness HTTP client: {error}"))
        })
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
    integration.integr_config_exists == Some(true)
        && mcp_transport(&integration.integr_name).is_some()
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

fn tasks_path(project_id: &str) -> String {
    format!("/p/{}/v1/tasks", encode_path_segment(project_id))
}

fn task_board_path(project_id: &str, task_id: &str) -> String {
    format!(
        "/p/{}/v1/tasks/{}/board",
        encode_path_segment(project_id),
        encode_path_segment(task_id)
    )
}

fn task_board_ready_path(project_id: &str, task_id: &str) -> String {
    format!(
        "/p/{}/v1/tasks/{}/board/ready",
        encode_path_segment(project_id),
        encode_path_segment(task_id)
    )
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

pub(crate) fn request_id(prefix: &str) -> String {
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
                .unwrap_or_else(|| "unknown".to_string()),
            _ => "unknown".to_string(),
        },
        None => "unknown".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolDecision {
    pub tool_call_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalControlAction {
    Pause,
    Resume,
    Stop,
}

impl TryFrom<&str> for GoalControlAction {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pause" => Ok(Self::Pause),
            "resume" => Ok(Self::Resume),
            "stop" => Ok(Self::Stop),
            _ => Err("goal control action must be pause, resume, or stop"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GoalBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_cents: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_progress_token_threshold: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_progress_turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrowserContextDecision {
    pub pending_message_id: String,
    pub include_actions: bool,
    pub include_console: bool,
    pub include_network: bool,
    pub include_mutations: bool,
    pub include_screenshot: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_n_actions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_n_console: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_n_network: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
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

    fn spawn_json_response_server(responses: Vec<Value>) -> TestServer {
        spawn_json_response_server_with_requests(responses).0
    }

    fn spawn_json_response_server_with_requests(
        responses: Vec<Value>,
    ) -> (TestServer, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = mpsc::channel();
        let (requests, received) = mpsc::channel();
        let handle = thread::spawn(move || {
            for response in responses {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if stopped.recv_timeout(Duration::from_millis(10)).is_ok() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                };
                let _ = requests.send(read_request_headers(&mut stream));
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
        (
            TestServer {
                base_url: format!("http://{addr}"),
                stop,
                handle,
            },
            received,
        )
    }

    fn open_project_response(state: &str, http_port: Option<u16>, lsp_port: Option<u16>) -> Value {
        json!({
            "project_id": "p1",
            "slug": "fixture",
            "root": "/tmp/fixture",
            "pinned": false,
            "worker": {
                "project_id": "p1",
                "pid": 7,
                "http_port": http_port,
                "lsp_port": lsp_port,
                "state": state,
                "last_error": null
            },
            "cron_pending": null
        })
    }

    fn read_request_headers(stream: &mut std::net::TcpStream) -> String {
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
        String::from_utf8_lossy(&request).to_string()
    }

    fn read_request_body(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut request = Vec::new();
        let mut buffer = [0; 256];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..read]);
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or_default();
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..read]);
        }
        let _ = stream.set_read_timeout(None);
        request[header_end..header_end + content_length].to_vec()
    }

    async fn capture_command<F, Fut>(send: F) -> Value
    where
        F: FnOnce(DaemonClient) -> Fut,
        Fut: Future<Output = Result<(), ClientError>>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (sent, received) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            sent.send(read_request_body(&mut stream)).unwrap();
            stream
                .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        });
        let client = DaemonClient::new(format!("http://{addr}"), None).unwrap();

        send(client).await.unwrap();
        let body = serde_json::from_slice(&received.recv().unwrap()).unwrap();
        handle.join().unwrap();
        body
    }

    fn assert_command(body: &Value, expected: Value) {
        assert_eq!(body, &expected);
        assert!(body["client_request_id"].is_string());
    }

    #[tokio::test]
    async fn branch_from_chat_serializes_required_identifiers() {
        let branch = capture_command(|client| async move {
            client
                .send_branch_from_chat("project", "chat", "source", "message-1")
                .await
        })
        .await;
        let mut expected = json!({
            "type": "branch_from_chat",
            "source_chat_id": "source",
            "up_to_message_id": "message-1",
        });
        expected["client_request_id"] = branch["client_request_id"].clone();
        assert_command(&branch, expected);
    }

    #[tokio::test]
    async fn set_params_serializes_exact_wire_shape() {
        let set_params = capture_command(|client| async move {
            client
                .send_set_params("project", "chat", json!({"model": "gpt-demo"}))
                .await
        })
        .await;
        let mut expected = json!({
            "type": "set_params",
            "patch": {"model": "gpt-demo"},
        });
        expected["client_request_id"] = set_params["client_request_id"].clone();
        assert_command(&set_params, expected);
    }

    #[tokio::test]
    async fn user_message_serializes_explicit_correlation_pair() {
        let user = capture_command(|client| async move {
            client
                .send_user_message("project", "chat", "request-1", "message-1", "hello")
                .await
        })
        .await;
        assert_command(
            &user,
            json!({
                "client_request_id": "request-1",
                "client_message_id": "message-1",
                "type": "user_message",
                "content": "hello",
            }),
        );
    }

    #[tokio::test]
    async fn retry_from_index_serializes_exact_wire_shape() {
        let retry = capture_command(|client| async move {
            client
                .send_retry_from_index("project", "chat", 3, json!("retry"))
                .await
        })
        .await;
        let mut expected = json!({
            "type": "retry_from_index",
            "index": 3,
            "content": "retry",
            "attachments": [],
        });
        expected["client_request_id"] = retry["client_request_id"].clone();
        assert_command(&retry, expected);
    }

    #[tokio::test]
    async fn abort_serializes_exact_wire_shape() {
        let abort =
            capture_command(|client| async move { client.send_abort("project", "chat").await })
                .await;
        let mut expected = json!({"type": "abort"});
        expected["client_request_id"] = abort["client_request_id"].clone();
        assert_command(&abort, expected);
    }

    #[tokio::test]
    async fn set_goal_serializes_unlimited_and_explicit_budget() {
        let unlimited = capture_command(|client| async move {
            client
                .send_set_goal_with_id("project", "chat", "set-goal-unlimited", "ship", None)
                .await
        })
        .await;
        assert_command(
            &unlimited,
            json!({
                "client_request_id": "set-goal-unlimited",
                "type": "set_goal",
                "content": "ship",
            }),
        );
        for key in [
            "max_turns",
            "max_minutes",
            "max_tokens",
            "max_cost_cents",
            "no_progress_turns",
        ] {
            assert!(unlimited.get(key).is_none());
        }

        let budgeted = capture_command(|client| async move {
            client
                .send_set_goal_with_id(
                    "project",
                    "chat",
                    "set-goal-budgeted",
                    "ship",
                    Some(GoalBudget {
                        max_turns: Some(3),
                        ..GoalBudget::default()
                    }),
                )
                .await
        })
        .await;
        assert_command(
            &budgeted,
            json!({
                "client_request_id": "set-goal-budgeted",
                "type": "set_goal",
                "content": "ship",
                "budget": {"max_turns": 3},
            }),
        );
    }

    #[tokio::test]
    async fn set_goal_budget_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_set_goal_budget_with_id(
                    "project",
                    "chat",
                    "set-goal-budget",
                    GoalBudget {
                        max_tokens: Some(42),
                        no_progress_turns: Some(2),
                        ..GoalBudget::default()
                    },
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "set-goal-budget",
                "type": "set_goal_budget",
                "budget": {"max_tokens": 42, "no_progress_turns": 2},
            }),
        );
    }

    #[tokio::test]
    async fn update_goal_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_update_goal_with_id("project", "chat", "update-goal", "tiny victory")
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "update-goal",
                "type": "update_goal",
                "note": "tiny victory",
            }),
        );
    }

    #[tokio::test]
    async fn goal_control_accepts_only_wire_actions() {
        for action in ["pause", "resume", "stop"] {
            let body = capture_command(|client| async move {
                client
                    .send_goal_control_with_id("project", "chat", "goal-control", action)
                    .await
            })
            .await;
            assert_command(
                &body,
                json!({
                    "client_request_id": "goal-control",
                    "type": "goal_control",
                    "action": action,
                }),
            );
        }
        assert_eq!(
            GoalControlAction::try_from("snooze").unwrap_err(),
            "goal control action must be pause, resume, or stop"
        );
    }

    #[tokio::test]
    async fn regenerate_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_regenerate_with_id("project", "chat", "regenerate")
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "regenerate",
                "type": "regenerate",
            }),
        );
    }

    #[tokio::test]
    async fn update_message_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_update_message_with_id(
                    "project",
                    "chat",
                    "update-message",
                    "message-1",
                    json!("rewritten"),
                    vec![json!({"path": "snack.txt"})],
                    true,
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "update-message",
                "type": "update_message",
                "message_id": "message-1",
                "content": "rewritten",
                "attachments": [{"path": "snack.txt"}],
                "regenerate": true,
            }),
        );
    }

    #[tokio::test]
    async fn remove_message_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_remove_message_with_id(
                    "project",
                    "chat",
                    "remove-message",
                    "message-1",
                    false,
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "remove-message",
                "type": "remove_message",
                "message_id": "message-1",
                "regenerate": false,
            }),
        );
    }

    #[tokio::test]
    async fn restore_messages_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_restore_messages_with_id(
                    "project",
                    "chat",
                    "restore-messages",
                    vec![json!({"role": "user", "content": "restore me"})],
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "restore-messages",
                "type": "restore_messages",
                "messages": [{"role": "user", "content": "restore me"}],
            }),
        );
    }

    #[tokio::test]
    async fn ide_tool_result_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_ide_tool_result_with_id(
                    "project",
                    "chat",
                    "ide-tool-result",
                    "tool-1",
                    "result",
                    true,
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "ide-tool-result",
                "type": "ide_tool_result",
                "tool_call_id": "tool-1",
                "content": "result",
                "tool_failed": true,
            }),
        );
    }

    #[tokio::test]
    async fn singular_tool_decision_serializes_plural_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_tool_decision_with_id("project", "chat", "tool-decision", "tool-1", true)
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "tool-decision",
                "type": "tool_decisions",
                "decisions": [{"tool_call_id": "tool-1", "accepted": true}],
            }),
        );
    }

    #[tokio::test]
    async fn tool_decisions_serializes_exact_plural_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_tool_decisions_with_id(
                    "project",
                    "chat",
                    "tool-decisions",
                    vec![
                        ToolDecision {
                            tool_call_id: "tool-1".to_string(),
                            accepted: true,
                        },
                        ToolDecision {
                            tool_call_id: "tool-2".to_string(),
                            accepted: false,
                        },
                    ],
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "tool-decisions",
                "type": "tool_decisions",
                "decisions": [
                    {"tool_call_id": "tool-1", "accepted": true},
                    {"tool_call_id": "tool-2", "accepted": false},
                ],
            }),
        );
    }

    #[tokio::test]
    async fn clean_background_processes_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_clean_background_processes_with_id(
                    "project",
                    "chat",
                    "clean-background-processes",
                    true,
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "clean-background-processes",
                "type": "clean_background_processes",
                "include_services": true,
            }),
        );
    }

    #[tokio::test]
    async fn browser_context_decision_serializes_exact_wire_shape() {
        let body = capture_command(|client| async move {
            client
                .send_browser_context_decision_with_id(
                    "project",
                    "chat",
                    "browser-context-decision",
                    BrowserContextDecision {
                        pending_message_id: "pending-1".to_string(),
                        include_actions: true,
                        include_console: false,
                        include_network: true,
                        include_mutations: false,
                        include_screenshot: true,
                        last_n_actions: Some(5),
                        last_n_console: None,
                        last_n_network: Some(7),
                    },
                )
                .await
        })
        .await;
        assert_command(
            &body,
            json!({
                "client_request_id": "browser-context-decision",
                "type": "browser_context_decision",
                "pending_message_id": "pending-1",
                "include_actions": true,
                "include_console": false,
                "include_network": true,
                "include_mutations": false,
                "include_screenshot": true,
                "last_n_actions": 5,
                "last_n_network": 7,
            }),
        );
        assert!(body.get("last_n_console").is_none());
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
    fn stale_snapshot_does_not_rewind_sequence_cursor() {
        let mut tracker = ChatSeqTracker::new();
        let snapshot = ChatEvent {
            chat_id: Some("chat".to_string()),
            seq: Some(100),
            kind: "snapshot".to_string(),
            raw: json!({"messages": []}),
        };
        let stale_snapshot = ChatEvent {
            seq: Some(40),
            ..snapshot.clone()
        };
        let next_event = ChatEvent {
            seq: Some(101),
            kind: "runtime_updated".to_string(),
            ..snapshot.clone()
        };

        assert_eq!(tracker.observe(&snapshot), ChatSeqDecision::Apply);
        assert_eq!(tracker.observe(&stale_snapshot), ChatSeqDecision::Suppress);
        assert_eq!(tracker.observe(&next_event), ChatSeqDecision::Apply);
    }

    #[test]
    fn duplicate_event_is_suppressed_without_resubscription() {
        let mut tracker = ChatSeqTracker::new();
        let snapshot = ChatEvent {
            chat_id: Some("chat".to_string()),
            seq: Some(1),
            kind: "snapshot".to_string(),
            raw: json!({"messages": []}),
        };
        let event = ChatEvent {
            seq: Some(2),
            kind: "runtime_updated".to_string(),
            ..snapshot.clone()
        };

        assert_eq!(tracker.observe(&snapshot), ChatSeqDecision::Apply);
        assert_eq!(tracker.observe(&event), ChatSeqDecision::Apply);
        assert_eq!(tracker.observe(&event), ChatSeqDecision::Suppress);
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
    async fn sse_utf8_eof_flushes_buffered_event_before_error() {
        let server = spawn_chat_sse_bytes_server(
            b"data: {\"chat_id\":\"chat\",\"seq\":0,\"type\":\"snapshot\"}\n\xE2",
        );
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let mut stream = client.subscribe_chat("project", "chat").await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        let error = stream.next().await.unwrap().unwrap_err();
        server.stop();

        assert_eq!(event.kind, "snapshot");
        assert_eq!(event.seq, Some(0));
        assert!(matches!(error, ClientError::Sse(message) if message.contains("incomplete UTF-8")));
    }

    #[tokio::test]
    async fn send_user_message_reuses_explicit_correlation_pair_for_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (requests, received) = mpsc::channel();
        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let body = read_request_body(&mut stream);
                requests.send(body).unwrap();
                stream
                    .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
                stream.flush().unwrap();
            }
        });
        let client = DaemonClient::new(format!("http://{addr}"), None).unwrap();

        client
            .send_user_message("project", "chat", "request-1", "message-1", "hello")
            .await
            .unwrap();
        client
            .send_user_message("project", "chat", "request-1", "message-1", "hello")
            .await
            .unwrap();

        let first: Value = serde_json::from_slice(&received.recv().unwrap()).unwrap();
        let second: Value = serde_json::from_slice(&received.recv().unwrap()).unwrap();
        handle.join().unwrap();

        assert_eq!(first["client_request_id"], "request-1");
        assert_eq!(second["client_request_id"], "request-1");
        assert_eq!(first["client_message_id"], "message-1");
        assert_eq!(second["client_message_id"], "message-1");
        assert_eq!(first["content"], "hello");
        assert_eq!(second["content"], "hello");
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
        assert!(!ClientError::Http("TypeScript typeof check failed".to_string()).is_unreachable());
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
    async fn open_project_retries_starting_worker_until_ready() {
        let server = spawn_json_response_server(vec![
            open_project_response("starting", None, None),
            open_project_response("ready", Some(31000), Some(31001)),
        ]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let project = client
            .open_project(Path::new("/tmp/fixture"))
            .await
            .unwrap();
        server.stop();

        assert_eq!(
            project.worker.unwrap().state,
            Value::String("ready".to_string())
        );
    }

    #[tokio::test]
    async fn open_project_accepts_ready_worker_near_retry_deadline() {
        let server = spawn_json_response_server(vec![
            open_project_response("starting", None, None),
            open_project_response("starting", None, None),
            open_project_response("starting", None, None),
            open_project_response("ready", Some(31000), Some(31001)),
        ]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();
        let started = Instant::now();

        let project = client
            .open_project_with_retry_policy(
                Path::new("/tmp/fixture"),
                OpenProjectRetryPolicy {
                    deadline: Duration::from_millis(400),
                    initial_backoff: Duration::from_millis(100),
                    max_backoff: Duration::from_millis(100),
                    max_attempts: 8,
                },
            )
            .await
            .unwrap();
        server.stop();

        assert!(started.elapsed() >= Duration::from_millis(200));
        assert_eq!(worker_state_label(project.worker.as_ref()), "ready");
    }

    #[tokio::test]
    async fn open_project_stops_after_exact_maximum_starting_attempts() {
        let (server, requests) = spawn_json_response_server_with_requests(
            (0..8)
                .map(|_| open_project_response("starting", None, None))
                .collect(),
        );
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let error = client
            .open_project_with_retry_policy(
                Path::new("/tmp/fixture"),
                OpenProjectRetryPolicy {
                    deadline: Duration::from_millis(100),
                    initial_backoff: Duration::from_millis(1),
                    max_backoff: Duration::from_millis(1),
                    max_attempts: 8,
                },
            )
            .await
            .unwrap_err();
        server.stop();

        assert!(
            matches!(error, ClientError::WorkerNotReady(message) if message.contains("starting"))
        );
        assert_eq!(
            (0..8)
                .map(|_| requests.recv().unwrap())
                .collect::<Vec<_>>()
                .len(),
            8
        );
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn list_projects_drops_rows_without_roots() {
        let server = spawn_json_response_server(vec![json!([
            {"id": "missing", "slug": "missing"},
            {"id": "empty", "slug": "empty", "root": ""},
            {"id": "valid", "slug": "valid", "root": "/tmp/valid"}
        ])]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let projects = client.list_projects().await.unwrap();
        server.stop();

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "valid");
    }

    #[test]
    fn open_project_rejects_empty_root() {
        let mut project: OpenProjectResponse =
            serde_json::from_value(open_project_response("ready", Some(31000), Some(31001)))
                .unwrap();
        project.root = PathBuf::new();

        assert!(matches!(
            validate_open_project_response(&project),
            Err(ClientError::Json(message)) if message.contains("empty root")
        ));
    }

    #[tokio::test]
    async fn open_project_accepts_ready_worker_with_nonzero_ports() {
        let server = spawn_json_response_server(vec![open_project_response(
            "ready",
            Some(31000),
            Some(31001),
        )]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let project = client
            .open_project(Path::new("/tmp/fixture"))
            .await
            .unwrap();
        server.stop();

        assert_eq!(project.project_id, "p1");
        assert_eq!(project.worker.unwrap().http_port, Some(31000));
    }

    #[tokio::test]
    async fn worker_list_deserializes_non_ready_rows_and_all_telemetry() {
        let server = spawn_json_response_server(vec![json!([
            {
                "project_id": "stopping",
                "slug": "stopping-project",
                "root": "/tmp/stopping",
                "root_exists": false,
                "pinned": true,
                "last_active_ms": 1,
                "state": "stopping",
                "pid": null,
                "rss_bytes": null,
                "cpu_percent": null,
                "uptime_secs": null,
                "http_port": null,
                "lsp_port": null,
                "lsp_clients": 0,
                "busy_chats": 0,
                "exec_running": 0,
                "live_proxy_streams": 0,
                "cron_next_fire_ms": null,
                "idle_deadline_ms": null,
                "last_status_report_ms": null,
                "last_error": null,
                "log_path": "/tmp/stopping.log"
            },
            {
                "project_id": "ready",
                "slug": "ready-project",
                "root": "/tmp/ready",
                "root_exists": true,
                "pinned": false,
                "last_active_ms": 2,
                "state": "ready",
                "pid": 42,
                "rss_bytes": 100,
                "cpu_percent": 12.5,
                "uptime_secs": 3,
                "http_port": 31000,
                "lsp_port": 31001,
                "lsp_clients": 4,
                "busy_chats": 5,
                "exec_running": 6,
                "live_proxy_streams": 7,
                "cron_next_fire_ms": 8,
                "idle_deadline_ms": 9,
                "last_status_report_ms": 10,
                "last_error": "none",
                "log_path": "/tmp/ready.log"
            }
        ])]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let workers = client.list_workers().await.unwrap();
        server.stop();

        assert_eq!(workers.len(), 2);
        let stopped = &workers[0];
        assert_eq!(stopped.http_port, None);
        assert_eq!(stopped.lsp_port, None);
        let ready = &workers[1];
        assert_eq!(ready.slug, "ready-project");
        assert_eq!(ready.root, PathBuf::from("/tmp/ready"));
        assert_eq!(ready.root_exists, Some(true));
        assert_eq!(ready.pinned, Some(false));
        assert_eq!(ready.last_active_ms, Some(2));
        assert_eq!(ready.rss_bytes, Some(100));
        assert_eq!(ready.cpu_percent, Some(12.5));
        assert_eq!(ready.uptime_secs, Some(3));
        assert_eq!(ready.http_port, Some(31000));
        assert_eq!(ready.lsp_port, Some(31001));
        assert_eq!(ready.lsp_clients, Some(4));
        assert_eq!(ready.busy_chats, Some(5));
        assert_eq!(ready.exec_running, Some(6));
        assert_eq!(ready.live_proxy_streams, Some(7));
        assert_eq!(ready.cron_next_fire_ms, Some(8));
        assert_eq!(ready.idle_deadline_ms, Some(9));
        assert_eq!(ready.last_status_report_ms, Some(10));
        assert_eq!(ready.last_error.as_deref(), Some("none"));
        assert_eq!(ready.log_path, "/tmp/ready.log");
        let round_trip = serde_json::to_value(ready).unwrap();
        assert_eq!(round_trip["slug"], "ready-project");
        assert_eq!(round_trip["root"], "/tmp/ready");
        assert_eq!(round_trip["root_exists"], true);
        assert_eq!(round_trip["pinned"], false);
        assert_eq!(round_trip["last_active_ms"], 2);
        assert_eq!(round_trip["rss_bytes"], 100);
        assert_eq!(round_trip["cpu_percent"], 12.5);
        assert_eq!(round_trip["uptime_secs"], 3);
        assert_eq!(round_trip["lsp_clients"], 4);
        assert_eq!(round_trip["busy_chats"], 5);
        assert_eq!(round_trip["exec_running"], 6);
        assert_eq!(round_trip["live_proxy_streams"], 7);
        assert_eq!(round_trip["cron_next_fire_ms"], 8);
        assert_eq!(round_trip["idle_deadline_ms"], 9);
        assert_eq!(round_trip["last_status_report_ms"], 10);
        assert_eq!(round_trip["log_path"], "/tmp/ready.log");
        assert_eq!(
            serde_json::from_value::<WorkerInfo>(round_trip).unwrap(),
            *ready
        );
    }

    fn trajectory_response(
        items: Vec<Value>,
        next_cursor: Option<&str>,
        has_more: bool,
        total_count: usize,
    ) -> Value {
        json!({
            "items": items,
            "next_cursor": next_cursor,
            "has_more": has_more,
            "total_count": total_count,
        })
    }

    fn trajectory_items(start: usize, end: usize) -> Vec<Value> {
        (start..end)
            .map(|index| json!({"id": format!("chat-{index}")}))
            .collect()
    }

    fn assert_tolerant_response<T>(empty: Value, populated: Value)
    where
        T: for<'de> Deserialize<'de> + Serialize + Default,
    {
        assert!(serde_json::from_value::<T>(empty).is_ok());
        let mut nulls = serde_json::to_value(T::default()).unwrap();
        if let Value::Object(values) = &mut nulls {
            values.values_mut().for_each(|value| *value = Value::Null);
        }
        assert!(serde_json::from_value::<T>(nulls).is_ok());
        let mut populated = populated;
        populated["future_field"] = json!(true);
        assert!(serde_json::from_value::<T>(populated).is_ok());
    }

    #[test]
    fn every_response_struct_accepts_nulls_and_unknown_fields() {
        macro_rules! response_cases {
            ($case:ident $(, $rest:ident)*) => {
                response_cases!($($rest),*);
                $case!(assert_tolerant_response);
            };
            () => {};
        }

        macro_rules! daemon_info_file {
            ($assert:ident) => {
                $assert::<DaemonInfoFile>(json!({}), json!({"pid": 1, "port": 8488}));
            };
        }
        macro_rules! daemon_status {
            ($assert:ident) => {
                $assert::<DaemonStatus>(json!({}), json!({"pid": 1, "workers": 2}));
            };
        }
        macro_rules! project_entry {
            ($assert:ident) => {
                $assert::<ProjectEntry>(json!({}), json!({"id": "project", "root": "/tmp/project"}));
            };
        }
        macro_rules! open_project_response {
            ($assert:ident) => {
                $assert::<OpenProjectResponse>(json!({}), json!({"project_id": "project"}));
            };
        }
        macro_rules! worker_info {
            ($assert:ident) => {
                $assert::<WorkerInfo>(json!({}), json!({"project_id": "project", "state": "ready"}));
            };
        }
        macro_rules! at_command_completion_response {
            ($assert:ident) => {
                $assert::<AtCommandCompletionResponse>(json!({}), json!({"completions": ["README.md"]}));
            };
        }
        macro_rules! integration_list_response {
            ($assert:ident) => {
                $assert::<IntegrationListResponse>(json!({}), json!({"integrations": [{}]}));
            };
        }
        macro_rules! integration_record {
            ($assert:ident) => {
                $assert::<IntegrationRecord>(json!({}), json!({"integr_name": "mcp_stdio_demo"}));
            };
        }
        macro_rules! mcp_view_data {
            ($assert:ident) => {
                $assert::<McpViewData>(json!({}), json!({"servers": [{}]}));
            };
        }
        macro_rules! mcp_server_summary {
            ($assert:ident) => {
                $assert::<McpServerSummary>(json!({}), json!({"name": "demo"}));
            };
        }
        macro_rules! mcp_server_info_response {
            ($assert:ident) => {
                $assert::<McpServerInfoResponse>(json!({}), json!({"tools": [{}]}));
            };
        }
        macro_rules! mcp_tool_info {
            ($assert:ident) => {
                $assert::<McpToolInfo>(json!({}), json!({"name": "lookup"}));
            };
        }
        macro_rules! mcp_resource_info {
            ($assert:ident) => {
                $assert::<McpResourceInfo>(json!({}), json!({"uri": "file:///tmp/readme"}));
            };
        }
        macro_rules! mcp_prompt_info {
            ($assert:ident) => {
                $assert::<McpPromptInfo>(json!({}), json!({"name": "review"}));
            };
        }
        macro_rules! slash_commands_list_response {
            ($assert:ident) => {
                $assert::<SlashCommandsListResponse>(json!({}), json!({"commands": [{}], "skills": [{}]}));
            };
        }
        macro_rules! slash_command_info {
            ($assert:ident) => {
                $assert::<SlashCommandInfo>(json!({}), json!({"name": "review"}));
            };
        }
        macro_rules! skill_info {
            ($assert:ident) => {
                $assert::<SkillInfo>(json!({}), json!({"name": "explain"}));
            };
        }
        macro_rules! provider_list_response {
            ($assert:ident) => {
                $assert::<ProviderListResponse>(json!({}), json!({"providers": [{}]}));
            };
        }
        macro_rules! provider_list_item {
            ($assert:ident) => {
                $assert::<ProviderListItem>(json!({}), json!({"name": "openai_codex"}));
            };
        }
        macro_rules! provider_oauth_logout_response {
            ($assert:ident) => {
                $assert::<ProviderOAuthLogoutResponse>(json!({}), json!({"success": true}));
            };
        }
        macro_rules! hooks_response {
            ($assert:ident) => {
                $assert::<HooksResponse>(json!({}), json!({"hooks": [{}]}));
            };
        }
        macro_rules! hook_info {
            ($assert:ident) => {
                $assert::<HookInfo>(json!({}), json!({"event": "PreToolUse"}));
            };
        }
        macro_rules! competitor_import_info_response {
            ($assert:ident) => {
                $assert::<CompetitorImportInfoResponse>(json!({}), json!({"sources": [{}]}));
            };
        }
        macro_rules! competitor_import_source_info {
            ($assert:ident) => {
                $assert::<CompetitorImportSourceInfo>(json!({}), json!({"id": "claude_code"}));
            };
        }
        macro_rules! import_report_counts {
            ($assert:ident) => {
                $assert::<ImportReportCounts>(json!({}), json!({"created": 1}));
            };
        }
        macro_rules! import_report_issue {
            ($assert:ident) => {
                $assert::<ImportReportIssue>(json!({}), json!({"status": "created"}));
            };
        }
        macro_rules! import_report {
            ($assert:ident) => {
                $assert::<ImportReport>(json!({}), json!({"discovered_candidates": 1}));
            };
        }
        macro_rules! competitor_import_run_response {
            ($assert:ident) => {
                $assert::<CompetitorImportRunResponse>(json!({}), json!({"scope": "project", "report": {}}));
            };
        }
        macro_rules! knowledge_graph_response {
            ($assert:ident) => {
                $assert::<KnowledgeGraphResponse>(json!({}), json!({"nodes": [{}], "edges": [{}]}));
            };
        }
        macro_rules! knowledge_node {
            ($assert:ident) => {
                $assert::<KnowledgeNode>(json!({}), json!({"id": "doc-1"}));
            };
        }
        macro_rules! knowledge_edge {
            ($assert:ident) => {
                $assert::<KnowledgeEdge>(json!({}), json!({"source": "doc-1", "target": "tag:ui"}));
            };
        }
        macro_rules! knowledge_stats {
            ($assert:ident) => {
                $assert::<KnowledgeStats>(json!({}), json!({"doc_count": 1}));
            };
        }
        macro_rules! daemon_event_record {
            ($assert:ident) => {
                $assert::<DaemonEventRecord>(json!({}), json!({"kind": "worker_ready"}));
            };
        }

        response_cases!(
            daemon_info_file,
            daemon_status,
            project_entry,
            open_project_response,
            worker_info,
            at_command_completion_response,
            integration_list_response,
            integration_record,
            mcp_view_data,
            mcp_server_summary,
            mcp_server_info_response,
            mcp_tool_info,
            mcp_resource_info,
            mcp_prompt_info,
            slash_commands_list_response,
            slash_command_info,
            skill_info,
            provider_list_response,
            provider_list_item,
            provider_oauth_logout_response,
            hooks_response,
            hook_info,
            competitor_import_info_response,
            competitor_import_source_info,
            import_report_counts,
            import_report_issue,
            import_report,
            competitor_import_run_response,
            knowledge_graph_response,
            knowledge_node,
            knowledge_edge,
            knowledge_stats,
            daemon_event_record
        );
    }

    #[test]
    fn missing_response_values_remain_unknown_in_surfaces() {
        assert_eq!(worker_state_label(Some(&WorkerInfo::default())), "unknown");
        assert_eq!(DaemonEndpoint::from_info(DaemonInfoFile::default()), None);
        assert_eq!(ProviderOAuthLogoutResponse::default().success, None);
        assert_eq!(ProviderListItem::default().has_credentials, None);
    }

    #[tokio::test]
    async fn trajectories_client_preserves_envelope_and_fetches_cursor_page() {
        let (server, requests) = spawn_json_response_server_with_requests(vec![
            trajectory_response(trajectory_items(0, 1), Some("cursor/for page 2"), true, 2),
            trajectory_response(trajectory_items(1, 2), None, false, 2),
        ]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let first_page = client
            .list_trajectories("project/id", 1, None)
            .await
            .unwrap();
        assert_eq!(first_page.has_more, Some(true));
        assert_eq!(first_page.next_cursor.as_deref(), Some("cursor/for page 2"));
        assert_eq!(first_page.total_count, Some(2));
        assert_eq!(first_page.items[0].id, "chat-0");

        let second_page = client
            .list_trajectories("project/id", 1, first_page.next_cursor.as_deref())
            .await
            .unwrap();
        server.stop();

        let first_request = requests.recv().unwrap();
        let second_request = requests.recv().unwrap();
        assert!(first_request
            .contains("GET /p/project%2Fid/v1/trajectories?displayable_only=true&limit=1"));
        assert!(second_request.contains(&format!(
            "cursor={}",
            encode_query_value("cursor/for page 2")
        )));
        assert_eq!(second_page.has_more, Some(false));
        assert_eq!(second_page.items[0].id, "chat-1");
    }

    #[tokio::test]
    async fn trajectories_client_reaches_every_chat_beyond_fifty() {
        let server = spawn_json_response_server(vec![
            trajectory_response(trajectory_items(0, 200), Some("page-2"), true, 201),
            trajectory_response(trajectory_items(200, 201), None, false, 201),
        ]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let trajectories = client.list_all_trajectories("project").await.unwrap();
        server.stop();

        assert_eq!(trajectories.len(), 201);
        assert_eq!(trajectories.first().unwrap().id, "chat-0");
        assert_eq!(trajectories.last().unwrap().id, "chat-200");
    }

    #[tokio::test]
    async fn trajectories_client_rejects_repeated_cursor_without_a_third_request() {
        let (server, requests) = spawn_json_response_server_with_requests(vec![
            trajectory_response(trajectory_items(0, 1), Some("page-a"), true, 3),
            trajectory_response(trajectory_items(1, 2), Some("page-a"), true, 3),
            trajectory_response(trajectory_items(2, 3), None, false, 3),
        ]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let error = client.list_all_trajectories("project").await.unwrap_err();
        server.stop();

        assert!(
            matches!(error, ClientError::Json(message) if message.contains("repeated next_cursor: page-a"))
        );
        assert!(requests.recv().unwrap().contains("limit=200"));
        assert!(requests.recv().unwrap().contains("cursor=page-a"));
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn trajectories_client_follows_distinct_cursors_until_terminal_page() {
        let (server, requests) = spawn_json_response_server_with_requests(vec![
            trajectory_response(trajectory_items(0, 1), Some("page-a"), true, 3),
            trajectory_response(trajectory_items(1, 2), Some("page-b"), true, 3),
            trajectory_response(trajectory_items(2, 3), None, false, 3),
        ]);
        let client = DaemonClient::new(&server.base_url, None).unwrap();

        let trajectories = client.list_all_trajectories("project").await.unwrap();
        server.stop();

        assert_eq!(
            trajectories
                .iter()
                .map(|trajectory| trajectory.id.as_str())
                .collect::<Vec<_>>(),
            ["chat-0", "chat-1", "chat-2"]
        );
        assert!(requests.recv().unwrap().contains("limit=200"));
        assert!(requests.recv().unwrap().contains("cursor=page-a"));
        assert!(requests.recv().unwrap().contains("cursor=page-b"));
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
        assert_eq!(response.skills[0].user_invocable, Some(true));
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
        assert_eq!(response.stats.doc_count, Some(1));
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
        assert_eq!(tasks_path("p1"), "/p/p1/v1/tasks");
        assert_eq!(
            task_board_path("p1", "task/a"),
            "/p/p1/v1/tasks/task%2Fa/board"
        );
        assert_eq!(
            task_board_ready_path("p1", "task/a"),
            "/p/p1/v1/tasks/task%2Fa/board/ready"
        );
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
