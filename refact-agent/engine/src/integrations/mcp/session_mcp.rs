use std::any::Any;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::future::Future;
use std::time::Instant;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::task::{AbortHandle, JoinHandle};
use rmcp::{RoleClient, service::RunningService};
use rmcp::transport::auth::AuthorizationManager;
use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    Tool as McpTool, Resource as McpResource, Prompt as McpPrompt, ServerInfo, ClientInfo,
    ClientCapabilities,
};
use rmcp::service::{Peer, RequestContext, NotificationContext};
use tokio::time::{timeout, sleep, Duration};
use serde::{Deserialize, Serialize};

use crate::global_context::GlobalContext;
use crate::integrations::sessions::IntegrationSession;
use crate::integrations::process_io_utils::read_file_with_cursor;
use super::mcp_sampling::mcp_sampling_create_message;
use super::mcp_metrics::SharedMetrics;
#[cfg(test)]
use super::mcp_metrics::new_shared_metrics;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MCPConnectionStatus {
    Connected,
    Connecting,
    Reconnecting { attempt: u32 },
    Failed { message: String },
    Disconnected,
    NeedsAuth,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MCPAuthStatus {
    NotApplicable,
    Authenticated,
    NeedsLogin,
    NeedsReauth,
    Refreshing,
    Error(String),
}

pub type McpRunningService = RunningService<RoleClient, McpClientHandler>;

pub struct McpClientHandler {
    pub peer_arc: Arc<AMutex<Option<Peer<RoleClient>>>>,
    pub session_arc: Arc<AMutex<Box<dyn IntegrationSession>>>,
    pub logs: Arc<AMutex<Vec<String>>>,
    pub debug_name: String,
    pub request_timeout: u64,
    pub gcx: Weak<ARwLock<GlobalContext>>,
    pub tool_refresh_handle: Arc<AMutex<Option<tokio::task::AbortHandle>>>,
    pub resource_refresh_handle: Arc<AMutex<Option<tokio::task::AbortHandle>>>,
    pub prompt_refresh_handle: Arc<AMutex<Option<tokio::task::AbortHandle>>>,
}

pub fn redact_sensitive_value(key: &str, value: &str) -> String {
    let key_lower = key.to_lowercase();
    if key_lower.contains("token")
        || key_lower.contains("secret")
        || key_lower.contains("password")
        || key_lower.contains("key")
        || key_lower.contains("authorization")
        || key_lower.contains("cookie")
    {
        if value.len() > 8 {
            format!("{}...{}", &value[..4], &value[value.len() - 4..])
        } else {
            "***REDACTED***".to_string()
        }
    } else {
        value.to_string()
    }
}

pub fn redact_sensitive_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let redacted: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| {
                    let new_v = match v {
                        serde_json::Value::String(s) => {
                            serde_json::Value::String(redact_sensitive_value(k, s))
                        }
                        other => redact_sensitive_json(other),
                    };
                    (k.clone(), new_v)
                })
                .collect();
            serde_json::Value::Object(redacted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redact_sensitive_json).collect())
        }
        other => other.clone(),
    }
}

impl ClientHandler for McpClientHandler {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.capabilities = ClientCapabilities::builder().enable_sampling().build();
        info
    }

    fn create_message(
        &self,
        params: rmcp::model::CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> impl Future<Output = Result<rmcp::model::CreateMessageResult, rmcp::ErrorData>> + Send + '_
    {
        let gcx_weak = self.gcx.clone();
        let debug_name = self.debug_name.clone();
        async move { mcp_sampling_create_message(gcx_weak, params, &debug_name).await }
    }

    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let peer_arc = self.peer_arc.clone();
        let session_arc = self.session_arc.clone();
        let logs = self.logs.clone();
        let debug_name = self.debug_name.clone();
        let request_timeout = self.request_timeout;
        let handle_arc = self.tool_refresh_handle.clone();
        async move {
            {
                let mut handle = handle_arc.lock().await;
                if let Some(h) = handle.take() {
                    h.abort();
                }
            }
            let task = tokio::spawn(async move {
                sleep(Duration::from_millis(200)).await;
                let peer = {
                    let locked = peer_arc.lock().await;
                    locked.clone()
                };
                let peer = match peer {
                    Some(p) => p,
                    None => {
                        tracing::warn!("tools/list_changed: no peer available for {}", debug_name);
                        return;
                    }
                };
                let new_tools = match timeout(
                    Duration::from_secs(request_timeout),
                    peer.list_all_tools(),
                )
                .await
                {
                    Ok(Ok(tools)) => tools,
                    Ok(Err(e)) => {
                        let msg = format!("tools/list_changed: failed to list tools: {:?}", e);
                        tracing::error!("{} for {}", msg, debug_name);
                        add_log_entry(logs, msg).await;
                        return;
                    }
                    Err(_) => {
                        let msg = format!(
                            "tools/list_changed: list_tools timed out after {}s",
                            request_timeout
                        );
                        tracing::error!("{} for {}", msg, debug_name);
                        add_log_entry(logs, msg).await;
                        return;
                    }
                };
                let old_count;
                let new_count = new_tools.len();
                {
                    let mut session_locked = session_arc.lock().await;
                    let session_downcasted = session_locked
                        .as_any_mut()
                        .downcast_mut::<SessionMCP>()
                        .unwrap();
                    old_count = session_downcasted.mcp_tools.len();
                    let old_names: std::collections::HashSet<_> = session_downcasted
                        .mcp_tools
                        .iter()
                        .map(|t| t.name.clone())
                        .collect();
                    let new_names: std::collections::HashSet<_> =
                        new_tools.iter().map(|t| t.name.clone()).collect();
                    let added: Vec<_> = new_names.difference(&old_names).collect();
                    let removed: Vec<_> = old_names.difference(&new_names).collect();
                    session_downcasted.mcp_tools = new_tools;
                    let msg = format!(
                        "tools/list_changed: {} → {} tools, added: {:?}, removed: {:?}",
                        old_count, new_count, added, removed
                    );
                    tracing::info!("{} for {}", msg, debug_name);
                    add_log_entry(logs, msg).await;
                }
            });
            let mut handle = handle_arc.lock().await;
            *handle = Some(task.abort_handle());
        }
    }

    fn on_resource_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let peer_arc = self.peer_arc.clone();
        let session_arc = self.session_arc.clone();
        let logs = self.logs.clone();
        let debug_name = self.debug_name.clone();
        let request_timeout = self.request_timeout;
        let gcx = self.gcx.clone();
        let handle_arc = self.resource_refresh_handle.clone();
        async move {
            {
                let mut handle = handle_arc.lock().await;
                if let Some(h) = handle.take() {
                    h.abort();
                }
            }
            let task = tokio::spawn(async move {
                sleep(Duration::from_millis(200)).await;
                let msg = "resources/list_changed: re-fetching resource list".to_string();
                tracing::info!("{} for {}", msg, debug_name);
                add_log_entry(logs.clone(), msg).await;

                let peer = {
                    let locked = peer_arc.lock().await;
                    locked.clone()
                };
                let peer = match peer {
                    Some(p) => p,
                    None => {
                        tracing::warn!(
                            "resources/list_changed: no peer available for {}",
                            debug_name
                        );
                        return;
                    }
                };

                let new_resources = match timeout(
                    Duration::from_secs(request_timeout),
                    peer.list_all_resources(),
                )
                .await
                {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        let msg =
                            format!("resources/list_changed: failed to list resources: {:?}", e);
                        tracing::error!("{} for {}", msg, debug_name);
                        add_log_entry(logs, msg).await;
                        return;
                    }
                    Err(_) => {
                        let msg = format!(
                            "resources/list_changed: list_resources timed out after {}s",
                            request_timeout
                        );
                        tracing::error!("{} for {}", msg, debug_name);
                        add_log_entry(logs, msg).await;
                        return;
                    }
                };

                let (old_count, config_path) = {
                    let mut session_locked = session_arc.lock().await;
                    let session_downcasted = session_locked
                        .as_any_mut()
                        .downcast_mut::<SessionMCP>()
                        .unwrap();
                    let old_count = session_downcasted.mcp_resources.len();
                    session_downcasted.mcp_resources = new_resources.clone();
                    (old_count, session_downcasted.config_path.clone())
                };

                let msg = format!(
                    "resources/list_changed: {} → {} resources",
                    old_count,
                    new_resources.len()
                );
                tracing::info!("{} for {}", msg, debug_name);
                add_log_entry(logs.clone(), msg).await;

                if !new_resources.is_empty() {
                    tokio::spawn(super::mcp_resources::index_mcp_resources(
                        gcx,
                        config_path,
                        peer,
                        new_resources,
                        logs,
                    ));
                }
            });
            let mut handle = handle_arc.lock().await;
            *handle = Some(task.abort_handle());
        }
    }

    fn on_prompt_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let peer_arc = self.peer_arc.clone();
        let session_arc = self.session_arc.clone();
        let logs = self.logs.clone();
        let debug_name = self.debug_name.clone();
        let request_timeout = self.request_timeout;
        let handle_arc = self.prompt_refresh_handle.clone();
        async move {
            {
                let mut handle = handle_arc.lock().await;
                if let Some(h) = handle.take() {
                    h.abort();
                }
            }
            let task = tokio::spawn(async move {
                sleep(Duration::from_millis(200)).await;
                let peer = {
                    let locked = peer_arc.lock().await;
                    locked.clone()
                };
                let peer = match peer {
                    Some(p) => p,
                    None => {
                        tracing::warn!(
                            "prompts/list_changed: no peer available for {}",
                            debug_name
                        );
                        return;
                    }
                };
                let new_prompts = match timeout(
                    Duration::from_secs(request_timeout),
                    peer.list_all_prompts(),
                )
                .await
                {
                    Ok(Ok(prompts)) => prompts,
                    Ok(Err(e)) => {
                        let msg = format!("prompts/list_changed: failed to list prompts: {:?}", e);
                        tracing::error!("{} for {}", msg, debug_name);
                        add_log_entry(logs, msg).await;
                        return;
                    }
                    Err(_) => {
                        let msg = format!(
                            "prompts/list_changed: list_prompts timed out after {}s",
                            request_timeout
                        );
                        tracing::error!("{} for {}", msg, debug_name);
                        add_log_entry(logs, msg).await;
                        return;
                    }
                };
                let new_count = new_prompts.len();
                {
                    let mut session_locked = session_arc.lock().await;
                    let session_downcasted = session_locked
                        .as_any_mut()
                        .downcast_mut::<SessionMCP>()
                        .unwrap();
                    let old_count = session_downcasted.mcp_prompts.len();
                    session_downcasted.mcp_prompts = new_prompts;
                    let msg = format!(
                        "prompts/list_changed: {} → {} prompts",
                        old_count, new_count
                    );
                    tracing::info!("{} for {}", msg, debug_name);
                    add_log_entry(logs, msg).await;
                }
                crate::http::routers::v1::at_commands::invalidate_slash_cache().await;
            });
            let mut handle = handle_arc.lock().await;
            *handle = Some(task.abort_handle());
        }
    }
}

pub struct SessionMCP {
    pub debug_name: String,
    pub config_path: String,
    pub launched_cfg: serde_json::Value,
    pub mcp_client: Option<Arc<AMutex<Option<McpRunningService>>>>,
    pub mcp_tools: Vec<McpTool>,
    pub mcp_resources: Vec<McpResource>,
    pub mcp_prompts: Vec<McpPrompt>,
    pub server_info: Option<ServerInfo>,
    pub startup_task_handles: Option<(Arc<AMutex<Option<JoinHandle<()>>>>, AbortHandle)>,
    pub health_task_handle: Option<AbortHandle>,
    pub logs: Arc<AMutex<Vec<String>>>,
    pub stderr_file_path: Option<PathBuf>,
    pub stderr_cursor: Arc<AMutex<u64>>,
    pub connection_status: MCPConnectionStatus,
    pub last_successful_connection: Option<Instant>,
    pub metrics: SharedMetrics,
    pub auth_manager: Option<Arc<AMutex<AuthorizationManager>>>,
    pub auth_status: MCPAuthStatus,
    pub oauth_refresh_task_handle: Option<AbortHandle>,
}

impl IntegrationSession for SessionMCP {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_expired(&self) -> bool {
        !std::path::Path::new(&self.config_path).exists()
    }

    fn try_stop(
        &mut self,
        self_arc: Arc<AMutex<Box<dyn IntegrationSession>>>,
    ) -> Box<dyn Future<Output = String> + Send> {
        Box::new(async move {
            let (
                debug_name,
                client,
                logs,
                startup_task_handles,
                health_task_handle,
                oauth_refresh_task_handle,
                stderr_file,
            ) = {
                let mut session_locked = self_arc.lock().await;
                let session_downcasted = session_locked
                    .as_any_mut()
                    .downcast_mut::<SessionMCP>()
                    .unwrap();
                (
                    session_downcasted.debug_name.clone(),
                    session_downcasted.mcp_client.clone(),
                    session_downcasted.logs.clone(),
                    session_downcasted.startup_task_handles.clone(),
                    session_downcasted.health_task_handle.clone(),
                    session_downcasted.oauth_refresh_task_handle.clone(),
                    session_downcasted.stderr_file_path.clone(),
                )
            };

            if let Some((_, abort_handle)) = startup_task_handles {
                add_log_entry(logs.clone(), "Aborted startup task".to_string()).await;
                abort_handle.abort();
            }

            if let Some(abort_handle) = health_task_handle {
                abort_handle.abort();
            }

            if let Some(abort_handle) = oauth_refresh_task_handle {
                abort_handle.abort();
            }

            if let Some(client) = client {
                cancel_mcp_client(&debug_name, client, logs).await;
            }
            if let Some(stderr_file) = &stderr_file {
                if let Err(e) = tokio::fs::remove_file(stderr_file).await {
                    tracing::error!("Failed to remove {}: {}", stderr_file.to_string_lossy(), e);
                }
            }

            "".to_string()
        })
    }
}

pub async fn add_log_entry(session_logs: Arc<AMutex<Vec<String>>>, entry: String) {
    let timestamp = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let log_entry = format!("[{}] {}", timestamp, entry);

    let mut session_logs_locked = session_logs.lock().await;
    session_logs_locked.extend(log_entry.lines().into_iter().map(|s| s.to_string()));

    if session_logs_locked.len() > 100 {
        let excess = session_logs_locked.len() - 100;
        session_logs_locked.drain(0..excess);
    }
}

pub async fn update_logs_from_stderr(
    stderr_file_path: &PathBuf,
    stderr_cursor: Arc<AMutex<u64>>,
    session_logs: Arc<AMutex<Vec<String>>>,
) -> Result<(), String> {
    let (buffer, bytes_read) = read_file_with_cursor(stderr_file_path, stderr_cursor.clone())
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;
    if bytes_read > 0 && !buffer.trim().is_empty() {
        add_log_entry(session_logs, buffer.trim().to_string()).await;
    }
    Ok(())
}

pub async fn cancel_mcp_client(
    debug_name: &str,
    mcp_client: Arc<AMutex<Option<McpRunningService>>>,
    session_logs: Arc<AMutex<Vec<String>>>,
) {
    tracing::info!("Stopping MCP Server for {}", debug_name);
    add_log_entry(session_logs.clone(), "Stopping MCP Server".to_string()).await;

    let client_to_cancel = {
        let mut mcp_client_locked = mcp_client.lock().await;
        mcp_client_locked.take()
    };

    if let Some(client) = client_to_cancel {
        match timeout(Duration::from_secs(3), client.cancel()).await {
            Ok(Ok(reason)) => {
                let success_msg = format!("MCP server stopped: {:?}", reason);
                tracing::info!("{} for {}", success_msg, debug_name);
                add_log_entry(session_logs, success_msg).await;
            }
            Ok(Err(e)) => {
                let error_msg = format!("Failed to stop MCP: {:?}", e);
                tracing::error!("{} for {}", error_msg, debug_name);
                add_log_entry(session_logs, error_msg).await;
            }
            Err(_) => {
                let error_msg = "MCP server stop operation timed out after 3 seconds".to_string();
                tracing::error!("{} for {}", error_msg, debug_name);
                add_log_entry(session_logs, error_msg).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session_mcp(debug_name: &str) -> SessionMCP {
        SessionMCP {
            debug_name: debug_name.to_string(),
            config_path: "/tmp/test.yaml".to_string(),
            launched_cfg: serde_json::Value::Null,
            mcp_client: None,
            mcp_tools: Vec::new(),
            mcp_resources: Vec::new(),
            mcp_prompts: Vec::new(),
            server_info: None,
            startup_task_handles: None,
            health_task_handle: None,
            logs: Arc::new(AMutex::new(Vec::new())),
            stderr_file_path: None,
            stderr_cursor: Arc::new(AMutex::new(0)),
            connection_status: MCPConnectionStatus::Disconnected,
            last_successful_connection: None,
            metrics: new_shared_metrics(),
            auth_manager: None,
            auth_status: MCPAuthStatus::NotApplicable,
            oauth_refresh_task_handle: None,
        }
    }

    #[test]
    fn test_mcp_client_handler_fields() {
        let peer_arc: Arc<AMutex<Option<rmcp::service::Peer<rmcp::RoleClient>>>> =
            Arc::new(AMutex::new(None));
        let session: Box<dyn IntegrationSession> = Box::new(make_session_mcp("test"));
        let session_arc = Arc::new(AMutex::new(session));
        let logs = Arc::new(AMutex::new(Vec::new()));
        let handler = McpClientHandler {
            peer_arc: peer_arc.clone(),
            session_arc,
            logs,
            debug_name: "test".to_string(),
            request_timeout: 30,
            gcx: Weak::new(),
            tool_refresh_handle: Arc::new(AMutex::new(None)),
            resource_refresh_handle: Arc::new(AMutex::new(None)),
            prompt_refresh_handle: Arc::new(AMutex::new(None)),
        };
        assert_eq!(handler.debug_name, "test");
        assert_eq!(handler.request_timeout, 30);
        assert!(handler
            .peer_arc
            .try_lock()
            .ok()
            .and_then(|g| g.clone())
            .is_none());
    }

    #[test]
    fn test_redact_sensitive_value() {
        assert_eq!(
            redact_sensitive_value("Authorization", "Bearer sk-1234567890"),
            "Bear...7890"
        );
        assert_eq!(redact_sensitive_value("api_key", "short"), "***REDACTED***");
        assert_eq!(
            redact_sensitive_value("description", "not secret"),
            "not secret"
        );
        assert_eq!(redact_sensitive_value("token", "abcdefghij"), "abcd...ghij");
        assert_eq!(redact_sensitive_value("password", "abc"), "***REDACTED***");
        assert_eq!(
            redact_sensitive_value("cookie", "session=xyz123456"),
            "sess...3456"
        );
        assert_eq!(
            redact_sensitive_value("Content-Type", "application/json"),
            "application/json"
        );
    }

    #[test]
    fn test_mcp_running_service_type_alias_exists() {
        fn _accepts_type_alias(_: Option<McpRunningService>) {}
        _accepts_type_alias(None);
    }

    #[test]
    fn test_redact_sensitive_json_nested() {
        let input = serde_json::json!({
            "name": "test",
            "credentials": {
                "token": "my_secret_token_value",
                "username": "admin"
            }
        });
        let result = redact_sensitive_json(&input);
        assert_eq!(result["credentials"]["token"], "my_s...alue");
        assert_eq!(result["credentials"]["username"], "admin");
        assert_eq!(result["name"], "test");
    }

    #[test]
    fn test_redact_sensitive_json_array() {
        let input = serde_json::json!([
            {"api_key": "secret123456", "name": "service1"},
            {"api_key": "another_key_val", "name": "service2"}
        ]);
        let result = redact_sensitive_json(&input);
        assert_eq!(result[0]["api_key"], "secr...3456");
        assert_eq!(result[0]["name"], "service1");
        assert_eq!(result[1]["api_key"], "anot..._val");
    }

    #[test]
    fn test_redact_sensitive_json_flat() {
        let input = serde_json::json!({"password": "abc123def", "host": "localhost"});
        let result = redact_sensitive_json(&input);
        assert_eq!(result["password"], "abc1...3def");
        assert_eq!(result["host"], "localhost");
    }

    #[test]
    fn test_redact_sensitive_json_primitives() {
        assert_eq!(redact_sensitive_json(&serde_json::json!("hello")), "hello");
        assert_eq!(redact_sensitive_json(&serde_json::json!(42)), 42);
        assert_eq!(
            redact_sensitive_json(&serde_json::json!(null)),
            serde_json::Value::Null
        );
    }

    #[test]
    fn test_mcp_auth_status_serialization() {
        let not_applicable = MCPAuthStatus::NotApplicable;
        let json = serde_json::to_value(&not_applicable).unwrap();
        assert_eq!(json, serde_json::json!("not_applicable"));

        let authenticated = MCPAuthStatus::Authenticated;
        let json = serde_json::to_value(&authenticated).unwrap();
        assert_eq!(json, serde_json::json!("authenticated"));

        let needs_login = MCPAuthStatus::NeedsLogin;
        let json = serde_json::to_value(&needs_login).unwrap();
        assert_eq!(json, serde_json::json!("needs_login"));

        let needs_reauth = MCPAuthStatus::NeedsReauth;
        let json = serde_json::to_value(&needs_reauth).unwrap();
        assert_eq!(json, serde_json::json!("needs_reauth"));

        let refreshing = MCPAuthStatus::Refreshing;
        let json = serde_json::to_value(&refreshing).unwrap();
        assert_eq!(json, serde_json::json!("refreshing"));

        let error = MCPAuthStatus::Error("something went wrong".to_string());
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["error"], "something went wrong");
    }

    #[test]
    fn test_mcp_auth_status_deserialization_roundtrip() {
        let statuses = vec![
            MCPAuthStatus::NotApplicable,
            MCPAuthStatus::Authenticated,
            MCPAuthStatus::NeedsLogin,
            MCPAuthStatus::NeedsReauth,
            MCPAuthStatus::Refreshing,
        ];
        for status in statuses {
            let json = serde_json::to_value(&status).unwrap();
            let roundtrip: MCPAuthStatus = serde_json::from_value(json).unwrap();
            assert_eq!(status, roundtrip);
        }
    }
}

pub async fn mcp_session_wait_startup(session_arc: Arc<AMutex<Box<dyn IntegrationSession>>>) {
    let startup_task_handles = {
        let mut session_locked = session_arc.lock().await;
        let session_downcasted = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session_downcasted.startup_task_handles.clone()
    };

    if let Some((join_handler_arc, _)) = startup_task_handles {
        let mut join_handler_locked = join_handler_arc.lock().await;
        if let Some(join_handler) = join_handler_locked.take() {
            let _ = join_handler.await;
        }
    }
}
