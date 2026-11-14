use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::time::Instant;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tokio::time::timeout;
use tokio::time::Duration;
use rmcp::{RoleClient, service::Peer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationCommon;
use crate::integrations::utils::{serialize_num_to_str, deserialize_str_to_num};
use rmcp::transport::auth::AuthClient;
use super::session_mcp::{
    SessionMCP, McpClientHandler, McpRunningService, MCPConnectionStatus, MCPAuthStatus,
    add_log_entry, cancel_mcp_client, redact_sensitive_value,
};
use super::mcp_auth::{
    MCPAuthSettings, MCPTokenManager, AuthType, create_auth_manager_from_tokens,
    load_tokens_from_config, mcp_oauth_refresh_task, probe_mcp_auth, save_tokens_to_config,
};
use super::mcp_metrics::new_shared_metrics;
use super::tool_mcp::ToolMCP;

#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct CommonMCPSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_tools: Option<Vec<String>>,
    #[serde(
        default = "default_init_timeout",
        serialize_with = "serialize_num_to_str",
        deserialize_with = "deserialize_str_to_num"
    )]
    pub init_timeout: u64,
    #[serde(
        default = "default_request_timeout",
        serialize_with = "serialize_num_to_str",
        deserialize_with = "deserialize_str_to_num"
    )]
    pub request_timeout: u64,
    #[serde(
        default = "default_health_check_interval",
        serialize_with = "serialize_num_to_str",
        deserialize_with = "deserialize_str_to_num"
    )]
    pub health_check_interval: u64,
    #[serde(
        default = "default_reconnect_max_attempts",
        serialize_with = "serialize_num_to_str",
        deserialize_with = "deserialize_str_to_num"
    )]
    pub reconnect_max_attempts: u64,
    #[serde(default = "default_reconnect_enabled")]
    pub reconnect_enabled: bool,
    #[serde(default = "default_sampling_policy")]
    pub sampling_policy: String,
    #[serde(default, deserialize_with = "super::mcp_auth::deserialize_string_list")]
    pub disabled_tools: Vec<String>,
    #[serde(default, deserialize_with = "super::mcp_auth::deserialize_string_list")]
    pub auto_approve_tools: Vec<String>,
}

pub fn default_sampling_policy() -> String {
    "ask".to_string()
}

pub fn default_init_timeout() -> u64 {
    60
}

pub fn default_request_timeout() -> u64 {
    30
}

pub fn default_health_check_interval() -> u64 {
    30
}

pub fn default_reconnect_max_attempts() -> u64 {
    7
}

pub fn default_reconnect_enabled() -> bool {
    true
}

impl Default for CommonMCPSettings {
    fn default() -> Self {
        Self {
            operator_tools: None,
            init_timeout: default_init_timeout(),
            request_timeout: default_request_timeout(),
            health_check_interval: default_health_check_interval(),
            reconnect_max_attempts: default_reconnect_max_attempts(),
            reconnect_enabled: default_reconnect_enabled(),
            sampling_policy: default_sampling_policy(),
            disabled_tools: Vec::new(),
            auto_approve_tools: Vec::new(),
        }
    }
}

#[async_trait]
pub trait MCPTransportInitializer: Send + Sync {
    async fn init_mcp_transport(
        &self,
        logs: Arc<AMutex<Vec<String>>>,
        debug_name: String,
        init_timeout: u64,
        request_timeout: u64,
        session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
        handler: McpClientHandler,
    ) -> Option<McpRunningService>;

    fn remote_probe_info(&self) -> Option<(String, HashMap<String, String>, MCPAuthSettings)> {
        None
    }
}

/// Server prefix used for model-facing tool names, derived from the config
/// filename. stdio configs keep the historical short form
/// (`mcp_stdio_github.yaml` -> `mcp_github`); remote configs keep their full
/// stem (`mcp_http_github.yaml` -> `mcp_http_github`) so a stdio and a remote
/// config for the same server never publish colliding tool names.
pub fn tool_name_server_prefix(config_path: &str) -> String {
    let yaml_name = std::path::Path::new(config_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    if let Some(stripped) = yaml_name.strip_prefix("mcp_stdio_") {
        return format!("mcp_{}", stripped);
    }
    yaml_name.to_string()
}

/// Process-scoped cache of the last successfully listed tool catalog per
/// config path. Lets `mcp_integr_tools` expose tools while a slow server is
/// still starting or reconnecting; execution against a not-yet-connected
/// client fails gracefully in `ToolMCP::tool_execute`.
static MCP_TOOL_CATALOG_CACHE: std::sync::Mutex<
    Option<HashMap<String, (std::time::Instant, Arc<Vec<rmcp::model::Tool>>)>>,
> = std::sync::Mutex::new(None);
const TOOL_CATALOG_CACHE_TTL_SECS: u64 = 1800;
const TOOL_CATALOG_CACHE_MAX_ENTRIES: usize = 64;
const MCP_CONNECT_TIMEOUT_SECS: u64 = 15;

static MCP_RETRY_DEADLINES: std::sync::Mutex<Option<HashMap<String, Instant>>> =
    std::sync::Mutex::new(None);

pub fn mcp_next_retry_in_secs(config_path: &str) -> Option<u64> {
    MCP_RETRY_DEADLINES
        .lock()
        .ok()?
        .as_ref()?
        .get(config_path)
        .map(|deadline| deadline.saturating_duration_since(Instant::now()).as_secs())
}

fn set_retry_deadline(config_path: &str, delay: Option<u64>) {
    if let Ok(mut deadlines) = MCP_RETRY_DEADLINES.lock() {
        let deadlines = deadlines.get_or_insert_with(HashMap::new);
        if let Some(delay) = delay {
            if deadlines.len() >= TOOL_CATALOG_CACHE_MAX_ENTRIES
                && !deadlines.contains_key(config_path)
            {
                if let Some(key) = deadlines
                    .iter()
                    .min_by_key(|(_, at)| *at)
                    .map(|(key, _)| key.clone())
                {
                    deadlines.remove(&key);
                }
            }
            deadlines.insert(
                config_path.into(),
                Instant::now() + Duration::from_secs(delay),
            );
        } else {
            deadlines.remove(config_path);
        }
    }
}

pub fn tool_catalog_cache_store(config_path: &str, tools: &[rmcp::model::Tool]) {
    let mut guard = match MCP_TOOL_CATALOG_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let cache = guard.get_or_insert_with(HashMap::new);
    if cache.len() >= TOOL_CATALOG_CACHE_MAX_ENTRIES && !cache.contains_key(config_path) {
        if let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(
        config_path.to_string(),
        (std::time::Instant::now(), Arc::new(tools.to_vec())),
    );
}

fn advance_mcp_catalog_generation(gcx: &Arc<GlobalContext>) {
    gcx.tool_catalog_generations.advance_mcp();
}

pub fn tool_catalog_cache_get(config_path: &str) -> Option<Arc<Vec<rmcp::model::Tool>>> {
    let mut guard = match MCP_TOOL_CATALOG_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let cache = guard.as_mut()?;
    match cache.get(config_path) {
        Some((at, tools)) if at.elapsed() < Duration::from_secs(TOOL_CATALOG_CACHE_TTL_SECS) => {
            Some(Arc::clone(tools))
        }
        Some(_) => {
            cache.remove(config_path);
            None
        }
        None => None,
    }
}

pub fn tool_catalog_cache_remove(config_path: &str) {
    let mut guard = match MCP_TOOL_CATALOG_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(cache) = guard.as_mut() {
        cache.remove(config_path);
    }
}

type SessionArc = Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>;

static MCP_LIFECYCLE_LOCKS: std::sync::Mutex<Option<HashMap<String, Weak<AMutex<()>>>>> =
    std::sync::Mutex::new(None);

/// Serializes setup and removal of one MCP config. Never hold it across
/// `load_integrations` (setup and reconcile take it themselves).
pub async fn mcp_lifecycle_lock(config_path: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = match MCP_LIFECYCLE_LOCKS.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let locks = locks.get_or_insert_with(HashMap::new);
        // Entries without a live Arc have no holder or waiter, so pruning them is safe.
        locks.retain(|_, lock| lock.strong_count() > 0);
        match locks.get(config_path).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(AMutex::new(()));
                locks.insert(config_path.to_string(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
}

#[cfg(test)]
fn lifecycle_lock_registered(config_path: &str) -> bool {
    let locks = match MCP_LIFECYCLE_LOCKS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    locks
        .as_ref()
        .and_then(|locks| locks.get(config_path))
        .is_some_and(|lock| lock.strong_count() > 0)
}

/// Aborts a spawned child when its owner is dropped (e.g. the startup task is aborted),
/// instead of detaching it.
pub(crate) struct AbortOnDrop(pub(crate) tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn remove_mcp_session_for_config(gcx: Arc<GlobalContext>, config_path: &str) {
    let _lifecycle = mcp_lifecycle_lock(config_path).await;
    remove_mcp_session_locked(gcx, config_path).await;
}

/// Reconcile variant: rechecks under the lifecycle lock that the config is still missing,
/// so a session recreated by a racing save is left intact.
pub(crate) async fn remove_mcp_session_if_config_missing(
    gcx: Arc<GlobalContext>,
    config_path: &str,
) -> bool {
    let _lifecycle = mcp_lifecycle_lock(config_path).await;
    if !matches!(tokio::fs::metadata(config_path).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return false;
    }
    remove_mcp_session_locked(gcx, config_path).await;
    true
}

async fn native_mcp_config_missing(config_path: &str) -> bool {
    let native = std::path::Path::new(config_path)
        .parent()
        .and_then(|dir| dir.file_name())
        .is_some_and(|name| name == "integrations.d");
    native
        && matches!(tokio::fs::metadata(config_path).await,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

async fn remove_mcp_session_locked(gcx: Arc<GlobalContext>, config_path: &str) {
    set_retry_deadline(config_path, None);
    let session = gcx
        .integration_sessions
        .lock()
        .await
        .get(config_path)
        .cloned();
    if let Some(session) = session {
        if session
            .lock()
            .await
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .is_none()
        {
            return;
        }
        {
            let mut sessions = gcx.integration_sessions.lock().await;
            if sessions
                .get(config_path)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                sessions.remove(config_path);
            }
        }
        let (stop, startup) = {
            let mut locked = session.lock().await;
            let startup = locked
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .and_then(|mcp| mcp.startup_task_handles.clone());
            (locked.try_stop(session.clone()), startup)
        };
        Box::into_pin(stop).await;
        if let Some((join, _)) = startup {
            if let Some(task) = join.lock().await.take() {
                let _ = timeout(Duration::from_secs(5), task).await;
            }
        }
        {
            let mut locked = session.lock().await;
            if let Some(mcp) = locked.as_any_mut().downcast_mut::<SessionMCP>() {
                mcp.mcp_client = None;
                mcp.mcp_tools.clear();
                mcp.mcp_resources.clear();
                mcp.mcp_prompts.clear();
                mcp.server_info = None;
            }
        }
        set_retry_deadline(config_path, None);
    }
    tool_catalog_cache_remove(config_path);
    gcx.tool_catalog_generations.advance_integrations();
    advance_mcp_catalog_generation(&gcx);
    super::mcp_resources::remove_indexed_resources(Arc::downgrade(&gcx), config_path.into()).await;
}

fn build_mcp_tool_boxes(
    tools: &[rmcp::model::Tool],
    config_path: &str,
    common: &IntegrationCommon,
    mcp_common: &CommonMCPSettings,
    mcp_client: Arc<AMutex<Option<McpRunningService>>>,
    request_timeout: u64,
) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
    let server_prefix = tool_name_server_prefix(config_path);
    let kept: Vec<&rmcp::model::Tool> = tools
        .iter()
        .filter(|tool| {
            mcp_common
                .operator_tools
                .as_ref()
                .is_none_or(|names| names.iter().any(|name| name == tool.name.as_ref()))
        })
        .filter(|tool| {
            !mcp_common
                .disabled_tools
                .iter()
                .any(|d| d == tool.name.as_ref())
        })
        .collect();
    let candidates: Vec<(String, String)> = kept
        .iter()
        .map(|tool| {
            (
                super::mcp_naming::model_tool_name(&server_prefix, tool.name.as_ref()),
                tool.name.to_string(),
            )
        })
        .collect();
    let resolved_names = super::mcp_naming::disambiguate_model_tool_names(candidates);

    kept.into_iter()
        .zip(resolved_names)
        .map(|(tool, model_name)| {
            let auto_approve = mcp_common
                .auto_approve_tools
                .iter()
                .any(|a| a == tool.name.as_ref());
            Box::new(ToolMCP {
                model_name,
                common: common.clone(),
                config_path: config_path.to_string(),
                mcp_client: mcp_client.clone(),
                mcp_tool: tool.clone(),
                request_timeout,
                init_timeout: mcp_common.init_timeout,
                auto_approve,
            }) as Box<dyn crate::tools::tools_description::Tool + Send>
        })
        .collect()
}

fn describe_connection_status(status: &MCPConnectionStatus) -> String {
    match status {
        MCPConnectionStatus::Connected => "connected".to_string(),
        MCPConnectionStatus::Connecting => "connecting".to_string(),
        MCPConnectionStatus::Reconnecting {
            attempt,
        } => {
            format!("reconnecting, attempt {}", attempt)
        }
        MCPConnectionStatus::Failed {
            message,
        } => message.clone(),
        MCPConnectionStatus::Disconnected => "disconnected".to_string(),
        MCPConnectionStatus::NeedsAuth => "needs authentication".to_string(),
    }
}

fn should_log_status_change(session: &mut SessionMCP) -> bool {
    if session.last_logged_status.as_ref() == Some(&session.connection_status) {
        return false;
    }
    session.last_logged_status = Some(session.connection_status.clone());
    true
}

fn log_serving_no_tools_once(session: &mut SessionMCP, session_key: &str, starting: bool) {
    if !should_log_status_change(session) {
        return;
    }
    let reason = describe_connection_status(&session.connection_status);
    if starting {
        tracing::debug!(
            "MCP {} is still starting ({}) and has no cached tool catalog yet, serving no tools",
            session_key,
            reason
        );
    } else {
        tracing::warn!(
            "MCP {} is unavailable ({}), serving no tools",
            session_key,
            reason
        );
    }
}

pub async fn mcp_integr_tools(
    gcx_option: Option<Weak<GlobalContext>>,
    config_path: &str,
    common: &IntegrationCommon,
    mcp_common: &CommonMCPSettings,
) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
    let request_timeout = mcp_common.request_timeout;
    let session_key = format!("{}", config_path);

    let gcx = match gcx_option {
        Some(gcx_weak) => match gcx_weak.upgrade() {
            Some(gcx) => gcx,
            None => {
                tracing::error!("Error: System is shutting down");
                return vec![];
            }
        },
        None => {
            tracing::error!("Error: MCP integration is not set up yet");
            return vec![];
        }
    };

    let session_maybe = {
        let integration_sessions = gcx.integration_sessions.clone();
        let integration_sessions = integration_sessions.lock().await;
        integration_sessions.get(&session_key).cloned()
    };
    let session = match session_maybe {
        Some(session) => session,
        None => {
            tracing::error!("No session for {:?}, strange (1)", session_key);
            return vec![];
        }
    };

    let result: Vec<Box<dyn crate::tools::tools_description::Tool + Send>> = {
        let mut session_locked = session.lock().await;
        let session_downcasted: &mut SessionMCP =
            match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                Some(s) => s,
                None => {
                    tracing::error!(
                        "Session for {:?} is not a SessionMCP, strange (3)",
                        session_key
                    );
                    return vec![];
                }
            };
        let startup_in_progress = matches!(
            session_downcasted.connection_status,
            MCPConnectionStatus::Connecting | MCPConnectionStatus::Reconnecting { .. }
        );
        if !startup_in_progress {
            match session_downcasted.mcp_client.clone() {
                Some(mcp_client) => {
                    // Connected (or terminally failed) sessions are authoritative,
                    // including a legitimately empty tool list.
                    session_downcasted.last_logged_status = None;
                    build_mcp_tool_boxes(
                        &session_downcasted.mcp_tools,
                        config_path,
                        common,
                        mcp_common,
                        mcp_client,
                        request_timeout,
                    )
                }
                None => {
                    log_serving_no_tools_once(session_downcasted, &session_key, false);
                    vec![]
                }
            }
        } else {
            // Startup or reconnect in progress: serve the last known tool
            // catalog so the model does not lose the server's tools while
            // it is coming up. Execution before the client is ready fails
            // gracefully inside ToolMCP::tool_execute.
            match tool_catalog_cache_get(config_path) {
                Some(cached_tools) => {
                    if should_log_status_change(session_downcasted) {
                        tracing::info!(
                            "serving {} cached MCP tools for {} while the server is starting ({})",
                            cached_tools.len(),
                            session_key,
                            describe_connection_status(&session_downcasted.connection_status)
                        );
                    }
                    let placeholder_client: Arc<AMutex<Option<McpRunningService>>> =
                        session_downcasted
                            .mcp_client
                            .clone()
                            .unwrap_or_else(|| Arc::new(AMutex::new(None)));
                    build_mcp_tool_boxes(
                        cached_tools.as_slice(),
                        config_path,
                        common,
                        mcp_common,
                        placeholder_client,
                        request_timeout,
                    )
                }
                None => {
                    log_serving_no_tools_once(session_downcasted, &session_key, true);
                    vec![]
                }
            }
        }
    };

    result
}

pub(crate) async fn build_reqwest_client_for_mcp(
    url: &str,
    headers: &HashMap<String, String>,
    auth: &MCPAuthSettings,
    transport_name: &str,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: &str,
) -> Option<reqwest::Client> {
    if url.is_empty() {
        let msg = format!("URL is empty for {} transport", transport_name);
        tracing::error!("{msg} for {debug_name}");
        add_log_entry(logs, msg).await;
        return None;
    }

    let mut effective_headers = headers.clone();
    let token_manager = MCPTokenManager::new(auth.clone());
    if let Err(e) = token_manager.apply_auth(&mut effective_headers).await {
        if auth.auth_type != AuthType::None {
            let msg = format!("Auth failed: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }
    }

    let mut header_map = reqwest::header::HeaderMap::new();
    for (k, v) in &effective_headers {
        match (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            (Ok(name), Ok(value)) => {
                header_map.insert(name, value);
            }
            _ => {
                let msg = format!("Invalid header: {}: {}", k, redact_sensitive_value(k, v));
                tracing::warn!("{msg} for {debug_name}");
                add_log_entry(logs.clone(), msg).await;
            }
        }
    }

    match reqwest::Client::builder()
        .default_headers(header_map)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(MCP_CONNECT_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => Some(client),
        Err(e) => {
            let msg = format!("Failed to build reqwest client: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
    }
}

pub(crate) async fn build_auth_client_for_mcp(
    url: &str,
    headers: &HashMap<String, String>,
    config_path: &str,
    transport_name: &str,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: &str,
    session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
) -> Option<AuthClient<reqwest::Client>> {
    let tokens = load_tokens_from_config(config_path).await;
    let mut tokens = match tokens {
        Some(t) if !t.access_token.is_empty() => t,
        _ => {
            let msg = format!(
                "No OAuth tokens found for {} transport; re-authentication required",
                transport_name
            );
            tracing::warn!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            {
                let mut session_locked = session.lock().await;
                let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                    Some(s) => s,
                    None => {
                        tracing::error!(
                            "Session for {debug_name} is not a SessionMCP, cannot set auth status"
                        );
                        return None;
                    }
                };
                mcp_session.connection_status = MCPConnectionStatus::NeedsAuth;
                mcp_session.auth_status = MCPAuthStatus::NeedsLogin;
            }
            return None;
        }
    };

    if tokens.bound_url.is_none() {
        tokens.bound_url = Some(url.to_string());
        if let Err(e) = save_tokens_to_config(config_path, &tokens).await {
            tracing::warn!(
                "Failed to bind legacy OAuth tokens to '{}' for {}: {}",
                url,
                debug_name,
                e
            );
        }
    }

    let auth_manager = match create_auth_manager_from_tokens(url, &tokens).await {
        Ok(m) => m,
        Err(e) => {
            let msg = format!(
                "Failed to restore OAuth session for {} transport: {}",
                transport_name, e
            );
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }
    };

    let mut header_map = reqwest::header::HeaderMap::new();
    for (k, v) in headers {
        match (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            (Ok(name), Ok(value)) => {
                header_map.insert(name, value);
            }
            _ => {
                let msg = format!("Invalid header: {}: {}", k, redact_sensitive_value(k, v));
                tracing::warn!("{msg} for {debug_name}");
                add_log_entry(logs.clone(), msg).await;
            }
        }
    }

    let base_client = match reqwest::Client::builder()
        .default_headers(header_map)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(MCP_CONNECT_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to build reqwest client: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }
    };

    let auth_client = AuthClient::new(base_client, auth_manager);
    let auth_manager_arc = auth_client.auth_manager.clone();
    {
        let mut session_locked = session.lock().await;
        let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
            Some(s) => s,
            None => {
                tracing::error!(
                    "Session for {debug_name} is not a SessionMCP, cannot set auth manager"
                );
                return None;
            }
        };
        mcp_session.auth_manager = Some(auth_manager_arc);
        mcp_session.auth_status = MCPAuthStatus::Authenticated;
    }
    Some(auth_client)
}

pub(crate) async fn serve_client_with_timeout<Fut, E>(
    serve_fut: Fut,
    init_timeout: u64,
    transport_name: &str,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: &str,
) -> Option<McpRunningService>
where
    Fut: std::future::Future<Output = Result<McpRunningService, E>> + Send,
    E: std::fmt::Display,
{
    match timeout(Duration::from_secs(init_timeout), serve_fut).await {
        Ok(Ok(client)) => Some(client),
        Ok(Err(e)) => {
            let msg = format!("Failed to init {} server: {}", transport_name, e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
        Err(_) => {
            let msg = format!("Request timed out after {} seconds", init_timeout);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
    }
}

/// Connects to a remote MCP server over the modern Streamable HTTP transport.
/// Shared by the `mcp_http_*` (primary) and `mcp_sse_*` (fallback) integrations.
pub(crate) async fn serve_streamable_http_transport(
    url: String,
    headers: HashMap<String, String>,
    auth: MCPAuthSettings,
    config_path: String,
    transport_name: &'static str,
    init_timeout: u64,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: String,
    session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
    handler: McpClientHandler,
) -> Option<McpRunningService> {
    use rmcp::transport::common::client_side_sse::ExponentialBackoff;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };

    let mut retry_config = ExponentialBackoff::default();
    retry_config.max_times = Some(3);
    retry_config.base_duration = Duration::from_millis(500);
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.trim());
    config.retry_config = Arc::new(retry_config);

    if crate::providers::litellm_mcp::gateway_is_managed(&config_path) {
        let mut retry_config = ExponentialBackoff::default();
        retry_config.max_times = Some(0);
        config.retry_config = Arc::new(retry_config);
        let client = build_reqwest_client_for_mcp(
            url.trim(),
            &super::integr_mcp_http::default_http_headers(),
            &MCPAuthSettings::default(),
            transport_name,
            logs.clone(),
            &debug_name,
        )
        .await?;
        let client = super::litellm_transport::GatewayClient::new(
            client,
            &config_path,
            super::litellm_transport::ProviderGatewaySource,
        );
        let transport = StreamableHttpClientTransport::with_client(client, config);
        return serve_client_with_timeout(
            rmcp::serve_client(handler, transport),
            init_timeout,
            transport_name,
            logs,
            &debug_name,
        )
        .await;
    }

    if auth.auth_type == AuthType::Oauth2Pkce {
        let auth_client = build_auth_client_for_mcp(
            url.trim(),
            &headers,
            &config_path,
            transport_name,
            logs.clone(),
            &debug_name,
            session,
        )
        .await?;
        let auth_client = super::litellm_transport::GatewayClient::new(
            auth_client,
            &config_path,
            super::litellm_transport::ProviderGatewaySource,
        );
        let transport = StreamableHttpClientTransport::with_client(auth_client, config);
        serve_client_with_timeout(
            rmcp::serve_client(handler, transport),
            init_timeout,
            transport_name,
            logs,
            &debug_name,
        )
        .await
    } else {
        let client = build_reqwest_client_for_mcp(
            url.trim(),
            &headers,
            &auth,
            transport_name,
            logs.clone(),
            &debug_name,
        )
        .await?;
        // Trusted managed-LiteLLM credentials are injected per outgoing request
        // (never stored in `headers`/cfg); no-op for manual configs.
        let client = super::litellm_transport::GatewayClient::new(
            client,
            &config_path,
            super::litellm_transport::ProviderGatewaySource,
        );
        let transport = StreamableHttpClientTransport::with_client(client, config);
        serve_client_with_timeout(
            rmcp::serve_client(handler, transport),
            init_timeout,
            transport_name,
            logs,
            &debug_name,
        )
        .await
    }
}

/// Connects to a remote MCP server over the legacy HTTP+SSE transport.
/// Shared by the `mcp_sse_*` (primary) and `mcp_http_*` (fallback) integrations.
pub(crate) async fn serve_legacy_sse_transport(
    url: String,
    headers: HashMap<String, String>,
    auth: MCPAuthSettings,
    config_path: String,
    init_timeout: u64,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: String,
    session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
    handler: McpClientHandler,
) -> Option<McpRunningService> {
    let client = super::mcp_sse_transport::build_legacy_sse_client(
        url.trim(),
        &headers,
        &auth,
        &config_path,
        logs.clone(),
        &debug_name,
        session,
    )
    .await?;
    let transport = match super::mcp_sse_transport::connect_legacy_sse(
        client,
        url.trim(),
        logs.clone(),
        &debug_name,
    )
    .await
    {
        Ok(transport) => transport,
        Err(e) => {
            let msg = format!("Failed to open legacy SSE connection: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }
    };
    // rustc's auto-trait solver rejects awaiting rmcp's serve future directly
    // inside this (already async-trait-boxed) fn with a spurious
    // "implementation of `Send` is not general enough" error. Spawning the
    // serve future checks Send at a monomorphic boundary and sidesteps that;
    // the guard aborts it on timeout or when this future is dropped, so it never
    // outlives this call.
    let serve_handle = tokio::spawn(rmcp::serve_client(handler, transport));
    let _abort_guard = AbortOnDrop(serve_handle.abort_handle());
    match timeout(Duration::from_secs(init_timeout), serve_handle).await {
        Ok(Ok(Ok(client))) => Some(client),
        Ok(Ok(Err(e))) => {
            let msg = format!("Failed to init legacy SSE server: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
        Ok(Err(join_error)) => {
            let msg = format!("Legacy SSE init task failed: {}", join_error);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
        Err(_) => {
            let msg = format!("Request timed out after {} seconds", init_timeout);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
    }
}

macro_rules! impl_mcp_integration_trait {
    ($struct_name:ty, $schema_yaml:expr) => {
        #[async_trait::async_trait]
        impl crate::integrations::integr_abstract::IntegrationTrait for $struct_name {
            async fn integr_settings_apply(
                &mut self,
                gcx: std::sync::Arc<crate::global_context::GlobalContext>,
                config_path: String,
                value: &serde_json::Value,
            ) -> Result<(), serde_json::Error> {
                self.gcx_option = Some(std::sync::Arc::downgrade(&gcx));
                self.cfg = serde_json::from_value(value.clone())?;
                self.common = serde_json::from_value(value.clone())?;
                self.config_path = config_path.clone();
                // Operator-owned restricted entries are durable on disk, but a normal
                // settings reload/restart is not authority to reconnect or republish them.
                // Only McpSessionOwner may create their live session after a fresh grant.
                if self.cfg.common.operator_tools.is_some() {
                    return Ok(());
                }
                crate::integrations::mcp::integr_mcp_common::mcp_session_setup(
                    gcx,
                    config_path,
                    serde_json::to_value(&self.cfg).unwrap_or_default(),
                    self.clone(),
                    self.cfg.common.init_timeout,
                    self.cfg.common.request_timeout,
                    self.cfg.common.health_check_interval,
                    self.cfg.common.reconnect_max_attempts,
                    self.cfg.common.reconnect_enabled,
                )
                .await;
                Ok(())
            }

            fn integr_settings_as_json(&self) -> serde_json::Value {
                serde_json::to_value(&self.cfg).unwrap()
            }

            fn integr_common(&self) -> crate::integrations::integr_abstract::IntegrationCommon {
                self.common.clone()
            }

            async fn integr_tools(
                &self,
                _integr_name: &str,
            ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
                crate::integrations::mcp::integr_mcp_common::mcp_integr_tools(
                    self.gcx_option.clone(),
                    &self.config_path,
                    &self.common,
                    &self.cfg.common,
                )
                .await
            }

            fn integr_schema(&self) -> &str {
                include_str!($schema_yaml)
            }
        }
    };
}
pub(crate) use impl_mcp_integration_trait;

fn should_skip_mcp_session_setup(session: &SessionMCP, new_cfg_value: &Value) -> bool {
    if new_cfg_value != &session.launched_cfg {
        return false;
    }

    session.mcp_client.is_some()
        || startup_task_in_flight(session)
        || matches!(session.connection_status, MCPConnectionStatus::NeedsAuth)
}

pub(crate) fn startup_task_in_flight(session: &SessionMCP) -> bool {
    session
        .startup_task_handles
        .as_ref()
        .map_or(false, |handles| !handles.1.is_finished())
}

fn prepare_mcp_session_for_start(
    session: &mut SessionMCP,
    new_cfg_value: Value,
    config_changed: bool,
) {
    session.launched_cfg = new_cfg_value;
    session.connection_status = MCPConnectionStatus::Connecting;
    session.last_logged_status = None;
    if config_changed {
        session.oauth_probe = None;
        session.auth_manager = None;
        session.auth_status = MCPAuthStatus::NotApplicable;
        if let Some(refresh_task) = session.oauth_refresh_task_handle.take() {
            refresh_task.abort();
        }
    }
    session.sampling_session_approved = false;
}

pub async fn mcp_session_setup<T: MCPTransportInitializer + Clone + Send + Sync + 'static>(
    gcx: Arc<GlobalContext>,
    config_path: String,
    new_cfg_value: Value,
    transport_initializer: T,
    init_timeout: u64,
    request_timeout: u64,
    health_check_interval: u64,
    reconnect_max_attempts: u64,
    reconnect_enabled: bool,
) {
    // Persisted operator policy is not a reusable launch grant. Only the exact-authorized
    // owner may connect/replace these sessions. Native readers may serve its active catalog.
    if new_cfg_value.get("operator_tools").is_some() {
        return;
    }

    let _lifecycle = mcp_lifecycle_lock(&config_path).await;
    if native_mcp_config_missing(&config_path).await {
        tracing::debug!(
            "MCP setup skipped: config {:?} no longer exists",
            config_path
        );
        remove_mcp_session_locked(gcx, &config_path).await;
        return;
    }
    let session_key = format!("{}", config_path);

    let session_arc = {
        let integration_sessions = gcx.integration_sessions.clone();
        let mut integration_sessions = integration_sessions.lock().await;
        let session = integration_sessions.get(&session_key).cloned();
        if session.is_none() {
            let new_session: Arc<
                AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>,
            > = Arc::new(AMutex::new(Box::new(SessionMCP {
                debug_name: session_key.clone(),
                config_path: config_path.clone(),
                launched_cfg: new_cfg_value.clone(),
                mcp_client: None,
                mcp_tools: Vec::new(),
                mcp_resources: Vec::new(),
                mcp_prompts: Vec::new(),
                server_info: None,
                startup_task_handles: None,
                health_task_handle: None,
                recovery_task: None,
                lifecycle_epoch: 0,
                logs: Arc::new(AMutex::new(Vec::new())),
                stderr_file_path: None,
                stderr_cursor: Arc::new(AMutex::new(0)),
                connection_status: MCPConnectionStatus::Connecting,
                last_successful_connection: None,
                metrics: new_shared_metrics(),
                #[cfg(target_os = "linux")]
                observation: None,
                auth_manager: None,
                auth_status: MCPAuthStatus::NotApplicable,
                oauth_refresh_task_handle: None,
                resource_index_task: None,
                oauth_probe: None,
                sampling_session_approved: false,
                active_progress: Vec::new(),
                last_logged_status: None,
            })));
            tracing::info!("MCP START SESSION {:?}", session_key);
            integration_sessions.insert(session_key.clone(), new_session.clone());
            new_session
        } else {
            session.unwrap()
        }
    };

    let session_arc_clone = session_arc.clone();
    let gcx_weak = Arc::downgrade(&gcx);

    {
        let mut session_locked = session_arc.lock().await;
        let session_downcasted = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
            Some(s) => s,
            None => {
                tracing::error!(
                    "Session for {:?} is not a SessionMCP, cannot setup MCP",
                    config_path
                );
                return;
            }
        };

        let config_changed = new_cfg_value != session_downcasted.launched_cfg;
        if should_skip_mcp_session_setup(session_downcasted, &new_cfg_value) {
            return;
        }

        if let Some((_, abort_handle)) = session_downcasted.startup_task_handles.take() {
            abort_handle.abort();
        }
        if let Some(health_task) = session_downcasted.health_task_handle.take() {
            health_task.abort();
        }
        if let Some(recovery_task) = session_downcasted.recovery_task.take() {
            recovery_task.abort();
        }
        if let Some(index_task) = session_downcasted.resource_index_task.take() {
            index_task.abort();
        }

        prepare_mcp_session_for_start(session_downcasted, new_cfg_value.clone(), config_changed);
        session_downcasted.lifecycle_epoch = session_downcasted.lifecycle_epoch.wrapping_add(1);
        let epoch = session_downcasted.lifecycle_epoch;
        set_retry_deadline(&config_path, None);

        let peer_arc: Arc<AMutex<Option<Peer<RoleClient>>>> = Arc::new(AMutex::new(None));
        let peer_arc_clone = peer_arc.clone();

        let startup_task_join_handle = tokio::spawn(async move {
            let (mcp_client, logs, debug_name, stderr_file) = {
                let mut session_locked = session_arc_clone.lock().await;
                let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                    Some(s) if s.lifecycle_epoch == epoch => s,
                    _ => return,
                };
                mcp_session.stderr_cursor = Arc::new(AMutex::new(0));
                (
                    std::mem::take(&mut mcp_session.mcp_client),
                    mcp_session.logs.clone(),
                    mcp_session.debug_name.clone(),
                    std::mem::take(&mut mcp_session.stderr_file_path),
                )
            };

            let log = async |level: tracing::Level, msg: String| {
                match level {
                    tracing::Level::ERROR => tracing::error!("{msg} for {debug_name}"),
                    tracing::Level::WARN => tracing::warn!("{msg} for {debug_name}"),
                    _ => tracing::info!("{msg} for {debug_name}"),
                }
                add_log_entry(logs.clone(), msg).await;
            };

            log(tracing::Level::INFO, "Applying new settings".to_string()).await;

            if let Some(mcp_client) = mcp_client {
                cancel_mcp_client(&debug_name, mcp_client, logs.clone()).await;
                // The config changed: the old tool catalog must not be served
                // for the new configuration.
                tool_catalog_cache_remove(&config_path);
                if let Some(gcx) = gcx_weak.upgrade() {
                    advance_mcp_catalog_generation(&gcx);
                }
                super::mcp_resources::remove_indexed_resources(
                    gcx_weak.clone(),
                    config_path.clone(),
                )
                .await;
            }
            if let Some(stderr_file) = &stderr_file {
                if let Err(e) = tokio::fs::remove_file(stderr_file).await {
                    log(
                        tracing::Level::ERROR,
                        format!("Failed to remove {}: {}", stderr_file.to_string_lossy(), e),
                    )
                    .await;
                }
            }

            let operator_tools: Option<Vec<String>> = new_cfg_value
                .get("operator_tools")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let restricted = operator_tools.is_some();
            let handler = McpClientHandler {
                restricted,
                peer_arc: peer_arc_clone.clone(),
                session_arc: session_arc_clone.clone(),
                logs: logs.clone(),
                debug_name: debug_name.clone(),
                request_timeout,
                gcx: gcx_weak.clone(),
                tool_refresh_handle: Arc::new(AMutex::new(None)),
                resource_refresh_handle: Arc::new(AMutex::new(None)),
                prompt_refresh_handle: Arc::new(AMutex::new(None)),
            };

            let client = match transport_initializer
                .init_mcp_transport(
                    logs.clone(),
                    debug_name.clone(),
                    init_timeout,
                    request_timeout,
                    session_arc_clone.clone(),
                    handler,
                )
                .await
            {
                Some(client) => client,
                None => {
                    let probe_result = match if restricted {
                        None
                    } else {
                        transport_initializer.remote_probe_info()
                    } {
                        Some((url, headers, auth))
                            if matches!(auth.auth_type, AuthType::None | AuthType::Bearer) =>
                        {
                            let probe = match super::mcp_auth::gateway_oauth::probe_gateway_auth(
                                &config_path,
                                &url,
                            )
                            .await
                            {
                                Ok(Some(probe)) => Ok(probe),
                                Ok(None) => probe_mcp_auth(&url, &headers, &auth).await,
                                Err(error) => Err(error),
                            };
                            match probe {
                                Ok(p) => Some(p),
                                Err(e) => {
                                    log(tracing::Level::WARN, format!("Auth probe failed: {}", e))
                                        .await;
                                    None
                                }
                            }
                        }
                        _ => None,
                    };
                    if let Some(p) = probe_result.as_ref().filter(|p| p.needs_auth) {
                        if p.oauth_available {
                            log(
                                tracing::Level::WARN,
                                "Server requires authentication and supports OAuth; login required"
                                    .to_string(),
                            )
                            .await;
                        } else {
                            log(tracing::Level::WARN, p.failure_message()).await;
                        }
                    }
                    let mut session_locked = session_arc_clone.lock().await;
                    let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>()
                    {
                        Some(s) if s.lifecycle_epoch == epoch => s,
                        _ => return,
                    };
                    if !matches!(
                        mcp_session.connection_status,
                        MCPConnectionStatus::NeedsAuth
                    ) {
                        match probe_result {
                            Some(p) if p.needs_auth && p.oauth_available => {
                                mcp_session.connection_status = MCPConnectionStatus::NeedsAuth;
                                mcp_session.auth_status = MCPAuthStatus::NeedsLogin;
                                mcp_session.oauth_probe = Some(p);
                            }
                            Some(p) if p.needs_auth => {
                                mcp_session.connection_status = MCPConnectionStatus::Failed {
                                    message: p.failure_message(),
                                };
                                mcp_session.oauth_probe = Some(p);
                            }
                            _ => {
                                mcp_session.connection_status = MCPConnectionStatus::Failed {
                                    message: "Transport initialization failed".to_string(),
                                };
                            }
                        }
                    }
                    drop(session_locked);
                    if reconnect_enabled && reconnect_max_attempts > 0 && !restricted {
                        spawn_initial_recovery(
                            session_arc_clone.clone(),
                            transport_initializer.clone(),
                            logs.clone(),
                            debug_name.clone(),
                            init_timeout,
                            request_timeout,
                            health_check_interval,
                            reconnect_max_attempts,
                            gcx_weak.clone(),
                            epoch,
                        )
                        .await;
                    }
                    return;
                }
            };

            log(tracing::Level::INFO, "Listing tools".to_string()).await;

            let tools = match timeout(
                Duration::from_secs(request_timeout),
                client.list_all_tools(),
            )
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(tools_error)) => {
                    log(
                        tracing::Level::ERROR,
                        format!("Failed to list tools: {:?}", tools_error),
                    )
                    .await;
                    let mut session_locked = session_arc_clone.lock().await;
                    let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>()
                    {
                        Some(s) if s.lifecycle_epoch == epoch => s,
                        _ => return,
                    };
                    mcp_session.connection_status = MCPConnectionStatus::Failed {
                        message: format!("Failed to list tools: {:?}", tools_error),
                    };
                    drop(session_locked);
                    cancel_mcp_client(
                        &debug_name,
                        Arc::new(AMutex::new(Some(client))),
                        logs.clone(),
                    )
                    .await;
                    if reconnect_enabled && reconnect_max_attempts > 0 && !restricted {
                        spawn_initial_recovery(
                            session_arc_clone.clone(),
                            transport_initializer.clone(),
                            logs.clone(),
                            debug_name.clone(),
                            init_timeout,
                            request_timeout,
                            health_check_interval,
                            reconnect_max_attempts,
                            gcx_weak.clone(),
                            epoch,
                        )
                        .await;
                    }
                    return;
                }
                Err(_) => {
                    log(
                        tracing::Level::ERROR,
                        format!("Request timed out after {} seconds", request_timeout),
                    )
                    .await;
                    let mut session_locked = session_arc_clone.lock().await;
                    let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>()
                    {
                        Some(s) if s.lifecycle_epoch == epoch => s,
                        _ => return,
                    };
                    mcp_session.connection_status = MCPConnectionStatus::Failed {
                        message: "List tools timed out".to_string(),
                    };
                    drop(session_locked);
                    cancel_mcp_client(
                        &debug_name,
                        Arc::new(AMutex::new(Some(client))),
                        logs.clone(),
                    )
                    .await;
                    if reconnect_enabled && reconnect_max_attempts > 0 && !restricted {
                        spawn_initial_recovery(
                            session_arc_clone.clone(),
                            transport_initializer.clone(),
                            logs.clone(),
                            debug_name.clone(),
                            init_timeout,
                            request_timeout,
                            health_check_interval,
                            reconnect_max_attempts,
                            gcx_weak.clone(),
                            epoch,
                        )
                        .await;
                    }
                    return;
                }
            };
            let tools: Vec<_> = tools
                .into_iter()
                .filter(|tool| {
                    operator_tools
                        .as_ref()
                        .is_none_or(|names| names.iter().any(|name| name == tool.name.as_ref()))
                })
                .collect();
            let tools_len = tools.len();

            let client_arc = Arc::new(AMutex::new(None));
            if !publish_connected_client(
                &session_arc_clone,
                &client_arc,
                client,
                &peer_arc,
                tools,
                restricted,
                &logs,
                &debug_name,
                request_timeout,
                &gcx_weak,
                epoch,
            )
            .await
            {
                return;
            }

            log(
                tracing::Level::INFO,
                format!("MCP session setup complete with {tools_len} tools"),
            )
            .await;

            if reconnect_enabled && reconnect_max_attempts > 0 && !restricted {
                install_health_monitor(
                    session_arc_clone.clone(),
                    transport_initializer.clone(),
                    client_arc,
                    logs.clone(),
                    debug_name.clone(),
                    init_timeout,
                    request_timeout,
                    health_check_interval,
                    reconnect_max_attempts,
                    gcx_weak.clone(),
                    epoch,
                )
                .await;
            }
        });

        let startup_task_abort_handle = startup_task_join_handle.abort_handle();
        session_downcasted.startup_task_handles = Some((
            Arc::new(AMutex::new(Some(startup_task_join_handle))),
            startup_task_abort_handle,
        ));
    }
}

async fn recover_initial_connection<T: MCPTransportInitializer + Clone + Send + Sync + 'static>(
    session: SessionArc,
    initializer: T,
    logs: Arc<AMutex<Vec<String>>>,
    name: String,
    init_timeout: u64,
    request_timeout: u64,
    health_interval: u64,
    attempts: u64,
    gcx: Weak<GlobalContext>,
    epoch: u64,
) {
    let client = Arc::new(AMutex::new(None));
    {
        let mut locked = session.lock().await;
        let Some(session) = locked.as_any_mut().downcast_mut::<SessionMCP>() else {
            return;
        };
        if session.lifecycle_epoch != epoch
            || matches!(session.connection_status, MCPConnectionStatus::NeedsAuth)
            || session
                .oauth_probe
                .as_ref()
                .is_some_and(|probe| probe.needs_auth)
        {
            return;
        }
        session.mcp_client = Some(client.clone());
    }
    if reconnect_with_backoff(
        session.clone(),
        &initializer,
        client.clone(),
        logs.clone(),
        &name,
        init_timeout,
        request_timeout,
        attempts,
        &[1, 2, 4, 8, 16, 30, 60],
        gcx.clone(),
        epoch,
    )
    .await
    {
        install_health_monitor(
            session,
            initializer,
            client,
            logs,
            name,
            init_timeout,
            request_timeout,
            health_interval,
            attempts,
            gcx,
            epoch,
        )
        .await;
    }
}

async fn spawn_initial_recovery<T: MCPTransportInitializer + Clone + Send + Sync + 'static>(
    session: SessionArc,
    initializer: T,
    logs: Arc<AMutex<Vec<String>>>,
    name: String,
    init_timeout: u64,
    request_timeout: u64,
    health_interval: u64,
    attempts: u64,
    gcx: Weak<GlobalContext>,
    epoch: u64,
) {
    let mut locked = session.lock().await;
    let Some(mcp) = locked.as_any_mut().downcast_mut::<SessionMCP>() else {
        return;
    };
    if mcp.lifecycle_epoch != epoch {
        return;
    }
    let task = tokio::spawn(recover_initial_connection(
        session.clone(),
        initializer,
        logs,
        name,
        init_timeout,
        request_timeout,
        health_interval,
        attempts,
        gcx,
        epoch,
    ));
    if let Some(old) = mcp.recovery_task.replace(task) {
        old.abort();
    }
}

async fn install_health_monitor<T: MCPTransportInitializer + Clone + Send + Sync + 'static>(
    session: SessionArc,
    initializer: T,
    client: Arc<AMutex<Option<McpRunningService>>>,
    logs: Arc<AMutex<Vec<String>>>,
    name: String,
    init_timeout: u64,
    request_timeout: u64,
    health_interval: u64,
    attempts: u64,
    gcx: Weak<GlobalContext>,
    epoch: u64,
) {
    let mut locked = session.lock().await;
    let Some(mcp) = locked.as_any_mut().downcast_mut::<SessionMCP>() else {
        return;
    };
    if mcp.lifecycle_epoch != epoch {
        return;
    }
    let health = tokio::spawn(mcp_health_monitor(
        session.clone(),
        initializer,
        client,
        logs,
        name,
        init_timeout,
        request_timeout,
        health_interval,
        attempts,
        gcx,
        epoch,
    ));
    if let Some(old) = mcp.health_task_handle.replace(health) {
        old.abort();
    }
}

/// Shared tail for every successful connect (initial, initial recovery, health reconnect):
/// loads server info, resources and prompts, publishes the catalog and starts the owned
/// resource-index and OAuth refresh tasks. Returns false when the session moved on.
async fn publish_connected_client(
    session: &SessionArc,
    client_arc: &Arc<AMutex<Option<McpRunningService>>>,
    client: McpRunningService,
    peer_arc: &Arc<AMutex<Option<Peer<RoleClient>>>>,
    tools: Vec<rmcp::model::Tool>,
    restricted: bool,
    logs: &Arc<AMutex<Vec<String>>>,
    debug_name: &str,
    request_timeout: u64,
    gcx_weak: &Weak<GlobalContext>,
    epoch: u64,
) -> bool {
    let peer = client.peer().clone();
    let server_info = client.peer_info().as_deref().cloned();
    *peer_arc.lock().await = Some(peer.clone());
    let capabilities = server_info
        .as_ref()
        .map(|s| s.capabilities.clone())
        .unwrap_or_default();
    let resources = if !restricted && capabilities.resources.is_some() {
        match timeout(
            Duration::from_secs(request_timeout),
            client.list_all_resources(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                add_log_entry(logs.clone(), format!("Failed to list resources: {:?}", e)).await;
                vec![]
            }
            Err(_) => {
                add_log_entry(logs.clone(), "List resources timed out".to_string()).await;
                vec![]
            }
        }
    } else {
        vec![]
    };
    let prompts = if !restricted && capabilities.prompts.is_some() {
        match timeout(
            Duration::from_secs(request_timeout),
            client.list_all_prompts(),
        )
        .await
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                add_log_entry(logs.clone(), format!("Failed to list prompts: {:?}", e)).await;
                vec![]
            }
            Err(_) => {
                add_log_entry(logs.clone(), "List prompts timed out".to_string()).await;
                vec![]
            }
        }
    } else {
        vec![]
    };

    *client_arc.lock().await = Some(client);
    let metrics = {
        let mut locked = session.lock().await;
        let mcp = match locked.as_any_mut().downcast_mut::<SessionMCP>() {
            Some(mcp) if mcp.lifecycle_epoch == epoch => mcp,
            _ => {
                drop(locked);
                cancel_mcp_client(debug_name, client_arc.clone(), logs.clone()).await;
                return false;
            }
        };
        let config_path = mcp.config_path.clone();
        mcp.mcp_client = Some(client_arc.clone());
        tool_catalog_cache_store(&config_path, &tools);
        if let Some(gcx) = gcx_weak.upgrade() {
            advance_mcp_catalog_generation(&gcx);
        }
        mcp.mcp_tools = tools;
        mcp.mcp_resources = resources.clone();
        mcp.mcp_prompts = prompts;
        mcp.server_info = server_info;
        set_retry_deadline(&config_path, None);
        mcp.connection_status = MCPConnectionStatus::Connected;
        mcp.last_logged_status = None;
        mcp.last_successful_connection = Some(Instant::now());
        if let Some(previous) = mcp.resource_index_task.take() {
            previous.abort();
        }
        if !resources.is_empty() {
            let index_task = tokio::spawn(super::mcp_resources::index_mcp_resources(
                gcx_weak.clone(),
                config_path.clone(),
                peer,
                resources,
                logs.clone(),
            ));
            mcp.resource_index_task = Some(index_task.abort_handle());
        }
        if mcp.auth_manager.is_some() {
            let refresh_task = tokio::spawn(mcp_oauth_refresh_task(session.clone(), config_path));
            if let Some(old) = mcp
                .oauth_refresh_task_handle
                .replace(refresh_task.abort_handle())
            {
                old.abort();
            }
        }
        mcp.metrics.clone()
    };
    metrics.lock().await.record_connected();
    true
}

async fn mcp_health_monitor<T: MCPTransportInitializer + Clone>(
    session_arc: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
    transport_initializer: T,
    client_arc: Arc<AMutex<Option<McpRunningService>>>,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: String,
    init_timeout: u64,
    request_timeout: u64,
    health_check_interval: u64,
    reconnect_max_attempts: u64,
    gcx_weak: std::sync::Weak<GlobalContext>,
    epoch: u64,
) {
    let backoff_delays: Vec<u64> = vec![1, 2, 4, 8, 16, 30, 60];

    loop {
        let shutdown_flag = match gcx_weak.upgrade() {
            Some(gcx) => gcx.shutdown_flag.clone(),
            None => return,
        };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(health_check_interval)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("MCP health monitor: shutdown detected, stopping for {}", debug_name);
                return;
            }
        }

        let peer_opt = {
            let client_locked = client_arc.lock().await;
            client_locked.as_ref().map(|c| c.peer().clone())
        };
        let is_alive = if let Some(peer) = peer_opt {
            match timeout(Duration::from_secs(5), peer.list_all_tools()).await {
                Ok(Ok(_)) => true,
                Ok(Err(e)) => {
                    tracing::warn!("MCP health check failed for {}: {}", debug_name, e);
                    add_log_entry(logs.clone(), format!("Health check failed: {}", e)).await;
                    false
                }
                Err(_) => {
                    tracing::warn!("MCP health check timed out for {}", debug_name);
                    add_log_entry(logs.clone(), "Health check timed out".to_string()).await;
                    false
                }
            }
        } else {
            false
        };

        if !is_alive {
            tracing::info!("MCP health monitor: connection lost for {}", debug_name);
            add_log_entry(
                logs.clone(),
                "Health monitor: connection lost, starting reconnect".to_string(),
            )
            .await;

            let reconnected = reconnect_with_backoff(
                session_arc.clone(),
                &transport_initializer,
                client_arc.clone(),
                logs.clone(),
                &debug_name,
                init_timeout,
                request_timeout,
                reconnect_max_attempts,
                &backoff_delays,
                gcx_weak.clone(),
                epoch,
            )
            .await;

            if !reconnected {
                let mut locked = session_arc.lock().await;
                if let Some(session) = locked.as_any_mut().downcast_mut::<SessionMCP>() {
                    if session.lifecycle_epoch != epoch {
                        return;
                    }
                    if matches!(
                        session.auth_status,
                        MCPAuthStatus::NeedsReauth | MCPAuthStatus::NeedsLogin
                    ) {
                        session.connection_status = MCPConnectionStatus::NeedsAuth;
                    }
                    set_retry_deadline(&session.config_path, None);
                }
                return;
            }
        }
    }
}

async fn reconnect_with_backoff<T: MCPTransportInitializer>(
    session_arc: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
    transport_initializer: &T,
    client_arc: Arc<AMutex<Option<McpRunningService>>>,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: &str,
    init_timeout: u64,
    request_timeout: u64,
    reconnect_max_attempts: u64,
    backoff_delays: &[u64],
    gcx_weak: std::sync::Weak<GlobalContext>,
    epoch: u64,
) -> bool {
    if reconnect_max_attempts == 0 || backoff_delays.is_empty() {
        return false;
    }
    let max_attempts = reconnect_max_attempts;
    let mut attempt = 0u64;
    loop {
        let shutdown_flag = match gcx_weak.upgrade() {
            Some(gcx) => gcx.shutdown_flag.clone(),
            None => return false,
        };
        if shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!(
                "MCP reconnect: shutdown detected, aborting reconnect for {}",
                debug_name
            );
            return false;
        }

        let reauthentication_required = {
            let mut session_locked = session_arc.lock().await;
            let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                Some(s) if s.lifecycle_epoch == epoch => s,
                _ => return false,
            };
            matches!(
                mcp_session.auth_status,
                MCPAuthStatus::NeedsReauth | MCPAuthStatus::NeedsLogin
            ) || matches!(
                mcp_session.connection_status,
                MCPConnectionStatus::NeedsAuth
            )
        };
        if reauthentication_required {
            let msg = format!(
                "MCP reconnect: re-authentication is required, stopping reconnect for {}",
                debug_name
            );
            tracing::info!("{}", msg);
            add_log_entry(logs.clone(), msg).await;
            return false;
        }

        let slow = attempt >= max_attempts;
        let delay = if slow {
            *backoff_delays.last().unwrap()
        } else {
            backoff_delays[(attempt as usize).min(backoff_delays.len() - 1)]
        };
        attempt = attempt.saturating_add(1);

        {
            let mut session_locked = session_arc.lock().await;
            let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                Some(s) if s.lifecycle_epoch == epoch => s,
                _ => return false,
            };
            set_retry_deadline(&mcp_session.config_path, Some(delay));
            mcp_session.connection_status = MCPConnectionStatus::Reconnecting {
                attempt: attempt.min(u32::MAX as u64) as u32,
            };
        }

        let msg = format!(
            "Reconnecting to {} (attempt {}/{}{}), waiting {}s",
            debug_name,
            attempt,
            max_attempts,
            if slow { ", slow retry" } else { "" },
            delay
        );
        tracing::info!("{}", msg);
        add_log_entry(logs.clone(), msg).await;

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("MCP reconnect: shutdown detected during backoff, aborting for {}", debug_name);
                return false;
            }
        }

        let peer_arc: Arc<AMutex<Option<Peer<RoleClient>>>> = Arc::new(AMutex::new(None));
        let handler = McpClientHandler {
            restricted: false,
            peer_arc: peer_arc.clone(),
            session_arc: session_arc.clone(),
            logs: logs.clone(),
            debug_name: debug_name.to_string(),
            request_timeout,
            gcx: gcx_weak.clone(),
            tool_refresh_handle: Arc::new(AMutex::new(None)),
            resource_refresh_handle: Arc::new(AMutex::new(None)),
            prompt_refresh_handle: Arc::new(AMutex::new(None)),
        };

        cancel_mcp_client(debug_name, client_arc.clone(), logs.clone()).await;
        let new_client = transport_initializer
            .init_mcp_transport(
                logs.clone(),
                debug_name.to_string(),
                init_timeout,
                request_timeout,
                session_arc.clone(),
                handler,
            )
            .await;

        let new_client = match new_client {
            Some(c) => c,
            None => {
                tracing::warn!("Reconnect attempt {} failed for {}", attempt, debug_name);
                continue;
            }
        };

        let tools = match timeout(
            Duration::from_secs(request_timeout),
            new_client.list_all_tools(),
        )
        .await
        {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                add_log_entry(
                    logs.clone(),
                    format!("Reconnect: failed to list tools: {:?}", e),
                )
                .await;
                cancel_mcp_client(
                    debug_name,
                    Arc::new(AMutex::new(Some(new_client))),
                    logs.clone(),
                )
                .await;
                continue;
            }
            Err(_) => {
                add_log_entry(logs.clone(), "Reconnect: list tools timed out".to_string()).await;
                cancel_mcp_client(
                    debug_name,
                    Arc::new(AMutex::new(Some(new_client))),
                    logs.clone(),
                )
                .await;
                continue;
            }
        };

        let tools_len = tools.len();
        if !publish_connected_client(
            &session_arc,
            &client_arc,
            new_client,
            &peer_arc,
            tools,
            false,
            &logs,
            debug_name,
            request_timeout,
            &gcx_weak,
            epoch,
        )
        .await
        {
            return false;
        }
        let metrics_arc = {
            let mut session_locked = session_arc.lock().await;
            session_locked
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .map(|s| s.metrics.clone())
        };
        if let Some(metrics_arc) = metrics_arc {
            metrics_arc.lock().await.record_reconnect();
        }

        let msg = format!(
            "Reconnected to {} successfully with {} tools",
            debug_name, tools_len
        );
        tracing::info!("{}", msg);
        add_log_entry(logs.clone(), msg).await;
        return true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use crate::integrations::sessions::IntegrationSession;

    fn make_session_arc(status: MCPConnectionStatus) -> Arc<AMutex<Box<dyn IntegrationSession>>> {
        Arc::new(AMutex::new(
            Box::new(super::super::session_mcp::SessionMCP {
                debug_name: "test".to_string(),
                config_path: "/tmp/test.yaml".to_string(),
                launched_cfg: serde_json::Value::Null,
                mcp_client: None,
                mcp_tools: Vec::new(),
                mcp_resources: Vec::new(),
                mcp_prompts: Vec::new(),
                server_info: None,
                startup_task_handles: None,
                health_task_handle: None,
                recovery_task: None,
                lifecycle_epoch: 0,
                logs: Arc::new(AMutex::new(Vec::new())),
                stderr_file_path: None,
                stderr_cursor: Arc::new(AMutex::new(0)),
                connection_status: status,
                last_successful_connection: None,
                metrics: super::super::mcp_metrics::new_shared_metrics(),
                #[cfg(target_os = "linux")]
                observation: None,
                auth_manager: None,
                auth_status: super::super::session_mcp::MCPAuthStatus::NotApplicable,
                oauth_refresh_task_handle: None,
                resource_index_task: None,
                oauth_probe: None,
                sampling_session_approved: false,
                active_progress: Vec::new(),
                last_logged_status: None,
            }) as Box<dyn IntegrationSession>,
        ))
    }

    #[tokio::test]
    async fn test_unchanged_needs_auth_session_skips_relaunch() {
        let session_arc = make_session_arc(MCPConnectionStatus::NeedsAuth);
        let config = serde_json::json!({"url": "https://mcp.example.com"});
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = config.clone();

        assert!(should_skip_mcp_session_setup(session, &config));
        assert!(!should_skip_mcp_session_setup(
            session,
            &serde_json::json!({"url": "https://other.example.com"})
        ));
    }

    #[tokio::test]
    async fn test_unchanged_config_with_startup_task_in_flight_skips_relaunch() {
        let session_arc = make_session_arc(MCPConnectionStatus::Connecting);
        let config = serde_json::json!({"url": "https://mcp.example.com"});
        let startup_task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = startup_task.abort_handle();
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = config.clone();
        session.startup_task_handles = Some((
            Arc::new(AMutex::new(Some(startup_task))),
            abort_handle.clone(),
        ));

        assert!(should_skip_mcp_session_setup(session, &config));
        abort_handle.abort();
    }

    #[tokio::test]
    async fn test_changed_config_with_startup_task_in_flight_does_not_skip_relaunch() {
        let session_arc = make_session_arc(MCPConnectionStatus::Connecting);
        let old_config = serde_json::json!({"url": "https://old.example.com"});
        let new_config = serde_json::json!({"url": "https://new.example.com"});
        let startup_task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = startup_task.abort_handle();
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = old_config;
        session.startup_task_handles = Some((
            Arc::new(AMutex::new(Some(startup_task))),
            abort_handle.clone(),
        ));

        assert!(!should_skip_mcp_session_setup(session, &new_config));
        abort_handle.abort();
    }

    #[tokio::test]
    async fn test_finished_startup_task_does_not_skip_relaunch() {
        let session_arc = make_session_arc(MCPConnectionStatus::Failed {
            message: "startup failed".to_string(),
        });
        let config = serde_json::json!({"url": "https://mcp.example.com"});
        let startup_task = tokio::spawn(async {});
        let abort_handle = startup_task.abort_handle();
        while !abort_handle.is_finished() {
            tokio::task::yield_now().await;
        }
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = config.clone();
        session.startup_task_handles =
            Some((Arc::new(AMutex::new(Some(startup_task))), abort_handle));

        assert!(!should_skip_mcp_session_setup(session, &config));
    }

    #[tokio::test]
    async fn test_config_change_clears_stale_auth_state() {
        let session_arc = make_session_arc(MCPConnectionStatus::NeedsAuth);
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = serde_json::json!({"url": "https://oauth.example.com"});
        session.auth_status = MCPAuthStatus::NeedsReauth;

        prepare_mcp_session_for_start(
            session,
            serde_json::json!({"command": "npx", "args": ["something"]}),
            true,
        );

        assert!(
            matches!(session.auth_status, MCPAuthStatus::NotApplicable),
            "stale NeedsReauth must not survive a config change, got {:?}",
            session.auth_status
        );
        assert!(session.auth_manager.is_none());
    }

    #[tokio::test]
    async fn test_same_config_retry_keeps_auth_state() {
        let session_arc = make_session_arc(MCPConnectionStatus::Connected);
        let config = serde_json::json!({"url": "https://oauth.example.com"});
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = config.clone();
        session.auth_status = MCPAuthStatus::Authenticated;

        prepare_mcp_session_for_start(session, config, false);

        assert!(matches!(session.auth_status, MCPAuthStatus::Authenticated));
    }

    #[tokio::test]
    async fn test_same_config_retry_preserves_oauth_probe() {
        let session_arc = make_session_arc(MCPConnectionStatus::Failed {
            message: "temporary failure".to_string(),
        });
        let config = serde_json::json!({"url": "https://mcp.example.com"});
        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = config.clone();
        session.oauth_probe = Some(super::super::mcp_auth::MCPAuthProbeResult {
            http_status: Some(401),
            needs_auth: true,
            oauth_available: true,
            suggested_scopes: vec!["mcp.read".to_string()],
            www_authenticate: None,
            discovery_error: None,
        });

        prepare_mcp_session_for_start(session, config.clone(), false);
        assert!(session.oauth_probe.is_some());

        prepare_mcp_session_for_start(
            session,
            serde_json::json!({"url": "https://other.example.com"}),
            true,
        );
        assert!(session.oauth_probe.is_none());
    }

    #[tokio::test]
    async fn test_oauth_token_changes_and_explicit_reconnect_force_relaunch() {
        use super::super::integr_mcp_http::SettingsMCPHttp;
        use super::super::mcp_auth::MCPOAuthTokens;

        let session_arc = make_session_arc(MCPConnectionStatus::NeedsAuth);
        let mut settings = SettingsMCPHttp::default();
        settings.mcp_url = "https://mcp.example.com".to_string();
        settings.auth.auth_type = AuthType::Oauth2Pkce;
        let without_tokens = serde_json::to_value(&settings).unwrap();

        let mut session_locked = session_arc.lock().await;
        let session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        session.launched_cfg = without_tokens.clone();
        assert!(should_skip_mcp_session_setup(session, &without_tokens));

        settings.auth.oauth_tokens = Some(MCPOAuthTokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: 9999999999000,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec!["mcp.read".to_string()],
            bound_url: None,
        });
        let with_tokens = serde_json::to_value(&settings).unwrap();
        assert!(!should_skip_mcp_session_setup(session, &with_tokens));

        session.launched_cfg = with_tokens.clone();
        assert!(!should_skip_mcp_session_setup(session, &without_tokens));

        session.launched_cfg = serde_json::Value::Null;
        assert!(!should_skip_mcp_session_setup(session, &with_tokens));
    }

    #[tokio::test]
    async fn persisted_operator_config_is_not_a_native_reconnect_grant() {
        #[derive(Clone)]
        struct CountInitializer(Arc<AtomicU32>);
        #[async_trait::async_trait]
        impl MCPTransportInitializer for CountInitializer {
            async fn init_mcp_transport(
                &self,
                _logs: Arc<AMutex<Vec<String>>>,
                _debug_name: String,
                _init_timeout: u64,
                _request_timeout: u64,
                _session: Arc<AMutex<Box<dyn IntegrationSession>>>,
                _handler: McpClientHandler,
            ) -> Option<McpRunningService> {
                self.0.fetch_add(1, Ordering::SeqCst);
                None
            }
        }
        let cfg = serde_json::json!({
            "operator_tools": ["read"],
            "available": {"on_your_laptop": true, "when_isolated": false}
        });
        let attempts = Arc::new(AtomicU32::new(0));
        let gcx = crate::global_context::tests::make_test_gcx().await;
        mcp_session_setup(
            gcx.clone(),
            "/scratch/mcp_operator_fixture.yaml".into(),
            cfg,
            CountInitializer(attempts.clone()),
            1,
            1,
            1,
            1,
            false,
        )
        .await;
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert!(gcx.integration_sessions.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stale_snapshot_setup_does_not_resurrect_deleted_native_config() {
        #[derive(Clone)]
        struct CountInitializer(Arc<AtomicU32>);
        #[async_trait::async_trait]
        impl MCPTransportInitializer for CountInitializer {
            async fn init_mcp_transport(
                &self,
                _logs: Arc<AMutex<Vec<String>>>,
                _debug_name: String,
                _init_timeout: u64,
                _request_timeout: u64,
                _session: Arc<AMutex<Box<dyn IntegrationSession>>>,
                _handler: McpClientHandler,
            ) -> Option<McpRunningService> {
                self.0.fetch_add(1, Ordering::SeqCst);
                None
            }
        }
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let config = dir
            .path()
            .join("integrations.d")
            .join("mcp_stdio_gone.yaml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "command: never\n").unwrap();
        let config_path = config.to_string_lossy().into_owned();
        let snapshot = serde_json::json!({"command": "never"});
        std::fs::remove_file(&config).unwrap();
        remove_mcp_session_for_config(gcx.clone(), &config_path).await;
        let attempts = Arc::new(AtomicU32::new(0));
        mcp_session_setup(
            gcx.clone(),
            config_path.clone(),
            snapshot.clone(),
            CountInitializer(attempts.clone()),
            1,
            1,
            1,
            1,
            false,
        )
        .await;
        assert!(gcx.integration_sessions.lock().await.is_empty());
        assert_eq!(attempts.load(Ordering::SeqCst), 0);

        gcx.integration_sessions.lock().await.insert(
            config_path.clone(),
            make_session_arc(MCPConnectionStatus::Connected),
        );
        mcp_session_setup(
            gcx.clone(),
            config_path.clone(),
            snapshot,
            CountInitializer(attempts.clone()),
            1,
            1,
            1,
            1,
            false,
        )
        .await;
        assert!(gcx.integration_sessions.lock().await.is_empty());
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
    }

    #[derive(Clone)]
    struct OfflineInitializer {
        attempts: Arc<AtomicU32>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
        stop_after: u32,
    }

    #[derive(Clone)]
    struct RecoveringInitializer {
        attempts: Arc<AtomicU32>,
        offline_attempts: u32,
        fixtures: Arc<AMutex<Vec<tokio::task::JoinHandle<()>>>>,
    }

    #[async_trait::async_trait]
    impl MCPTransportInitializer for RecoveringInitializer {
        async fn init_mcp_transport(
            &self,
            _logs: Arc<AMutex<Vec<String>>>,
            _name: String,
            _init_timeout: u64,
            _request_timeout: u64,
            _session: Arc<AMutex<Box<dyn IntegrationSession>>>,
            handler: McpClientHandler,
        ) -> Option<McpRunningService> {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            if self.attempts.fetch_add(1, Ordering::SeqCst) < self.offline_attempts {
                return None;
            }
            let (client, server) = tokio::io::duplex(8192);
            self.fixtures.lock().await.push(tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(server);
                let mut lines = BufReader::new(read).lines();
                while let Some(line) = lines.next_line().await.unwrap() {
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let result = match request["method"].as_str() {
                        Some("initialize") => serde_json::json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools":{}, "resources":{}, "prompts":{}},
                            "serverInfo": {"name":"fixture", "version":"1"}
                        }),
                        Some("tools/list") => serde_json::json!({"tools":[{
                            "name":"read", "inputSchema":{"type":"object"}
                        }]}),
                        Some("resources/list") => serde_json::json!({"resources":[{
                            "uri":"file:///fixture.txt", "name":"fixture.txt"
                        }]}),
                        Some("prompts/list") => serde_json::json!({"prompts":[{
                            "name":"summarize"
                        }]}),
                        _ => continue,
                    };
                    let response =
                        serde_json::json!({"jsonrpc":"2.0", "id":request["id"], "result":result});
                    if write
                        .write_all(format!("{response}\n").as_bytes())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }));
            timeout(Duration::from_secs(2), rmcp::serve_client(handler, client))
                .await
                .ok()?
                .ok()
        }
    }

    #[tokio::test]
    async fn initial_offline_recovery_runs_connected_tail_and_removal_stops_supervisor() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = "/tmp/mcp_initial_offline_fixture.yaml";
        let initializer = RecoveringInitializer {
            attempts: Arc::new(AtomicU32::new(0)),
            offline_attempts: 1,
            fixtures: Arc::new(AMutex::new(Vec::new())),
        };
        mcp_session_setup(
            gcx.clone(),
            path.into(),
            serde_json::json!({}),
            initializer.clone(),
            1,
            1,
            30,
            1,
            true,
        )
        .await;
        timeout(Duration::from_secs(5), async {
            loop {
                let session = gcx
                    .integration_sessions
                    .lock()
                    .await
                    .get(path)
                    .cloned()
                    .unwrap();
                let mut locked = session.lock().await;
                let session = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
                if session.connection_status == MCPConnectionStatus::Connected {
                    assert_eq!(session.mcp_tools[0].name, "read");
                    assert_eq!(
                        session.server_info.as_ref().unwrap().server_info.name,
                        "fixture"
                    );
                    assert_eq!(session.mcp_resources[0].uri, "file:///fixture.txt");
                    assert_eq!(session.mcp_prompts[0].name, "summarize");
                    assert!(session.recovery_task.is_some());
                    assert!(session
                        .startup_task_handles
                        .as_ref()
                        .unwrap()
                        .1
                        .is_finished());
                    break;
                }
                drop(locked);
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(initializer.attempts.load(Ordering::SeqCst), 2);
        remove_mcp_session_for_config(gcx, path).await;
        for fixture in initializer.fixtures.lock().await.drain(..) {
            timeout(Duration::from_secs(2), fixture)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn slow_retry_connects_after_exhausted_burst() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let session = make_session_arc(MCPConnectionStatus::Disconnected);
        let client = Arc::new(AMutex::new(None));
        let logs = Arc::new(AMutex::new(Vec::new()));
        let initializer = RecoveringInitializer {
            attempts: Arc::new(AtomicU32::new(0)),
            offline_attempts: 3,
            fixtures: Arc::new(AMutex::new(Vec::new())),
        };
        assert!(
            reconnect_with_backoff(
                session.clone(),
                &initializer,
                client.clone(),
                logs.clone(),
                "slow-fixture",
                1,
                1,
                2,
                &[0],
                Arc::downgrade(&gcx),
                0,
            )
            .await
        );
        assert_eq!(initializer.attempts.load(Ordering::SeqCst), 4);
        assert_eq!(
            session
                .lock()
                .await
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .unwrap()
                .connection_status,
            MCPConnectionStatus::Connected
        );
        cancel_mcp_client("slow-fixture", client, logs).await;
        for fixture in initializer.fixtures.lock().await.drain(..) {
            timeout(Duration::from_secs(2), fixture)
                .await
                .unwrap()
                .unwrap();
        }
        tool_catalog_cache_remove("/tmp/test.yaml");
    }

    #[tokio::test]
    async fn removal_serializes_with_changed_settings_setup() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = "/tmp/mcp_remove_vs_setup_fixture.yaml";
        let initializer = RecoveringInitializer {
            attempts: Arc::new(AtomicU32::new(0)),
            offline_attempts: 0,
            fixtures: Arc::new(AMutex::new(Vec::new())),
        };
        let setup = |version: u32| {
            mcp_session_setup(
                gcx.clone(),
                path.into(),
                serde_json::json!({ "version": version }),
                initializer.clone(),
                2,
                2,
                30,
                1,
                true,
            )
        };
        setup(1).await;
        let old = gcx
            .integration_sessions
            .lock()
            .await
            .get(path)
            .cloned()
            .unwrap();
        let old_join = old
            .lock()
            .await
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap()
            .startup_task_handles
            .clone()
            .unwrap()
            .0;
        timeout(
            Duration::from_secs(5),
            super::super::session_mcp::mcp_session_wait_startup_bounded(
                old.clone(),
                Duration::from_secs(5),
            ),
        )
        .await
        .unwrap();
        let barrier = old_join.lock().await;
        let remover = tokio::spawn(remove_mcp_session_for_config(gcx.clone(), path));
        timeout(Duration::from_secs(5), async {
            while gcx.integration_sessions.lock().await.contains_key(path)
                || old
                    .lock()
                    .await
                    .as_any_mut()
                    .downcast_mut::<SessionMCP>()
                    .unwrap()
                    .connection_status
                    != MCPConnectionStatus::Disconnected
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let racing_setup = tokio::spawn({
            let gcx = gcx.clone();
            let initializer = initializer.clone();
            async move {
                mcp_session_setup(
                    gcx,
                    path.into(),
                    serde_json::json!({"version": 2}),
                    initializer,
                    2,
                    2,
                    30,
                    1,
                    true,
                )
                .await
            }
        });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(!racing_setup.is_finished());
        assert!(!gcx.integration_sessions.lock().await.contains_key(path));
        drop(barrier);
        timeout(Duration::from_secs(5), remover)
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(5), racing_setup)
            .await
            .unwrap()
            .unwrap();
        {
            let mut locked = old.lock().await;
            let old = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
            assert!(old.mcp_client.is_none());
            assert!(old.health_task_handle.is_none());
            assert!(old.recovery_task.is_none());
            assert!(old.mcp_tools.is_empty());
        }
        let new = gcx
            .integration_sessions
            .lock()
            .await
            .get(path)
            .cloned()
            .unwrap();
        assert!(!Arc::ptr_eq(&old, &new));
        timeout(Duration::from_secs(5), async {
            loop {
                {
                    let mut locked = new.lock().await;
                    let new = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
                    if new.connection_status == MCPConnectionStatus::Connected {
                        assert_eq!(new.launched_cfg, serde_json::json!({"version": 2}));
                        assert_eq!(new.mcp_tools[0].name, "read");
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(tool_catalog_cache_get(path).unwrap()[0].name, "read");
        assert_eq!(initializer.attempts.load(Ordering::SeqCst), 2);
        remove_mcp_session_for_config(gcx.clone(), path).await;
        assert!(!gcx.integration_sessions.lock().await.contains_key(path));
        for fixture in initializer.fixtures.lock().await.drain(..) {
            timeout(Duration::from_secs(2), fixture)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn settings_change_and_removal_abort_initial_retry() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = "/tmp/mcp_cancel_retry_fixture.yaml";
        let initializer = OfflineInitializer {
            attempts: Arc::new(AtomicU32::new(0)),
            shutdown: gcx.shutdown_flag.clone(),
            stop_after: u32::MAX,
        };
        mcp_session_setup(
            gcx.clone(),
            path.into(),
            serde_json::json!({"version":1}),
            initializer.clone(),
            1,
            1,
            30,
            1,
            true,
        )
        .await;
        let session = gcx
            .integration_sessions
            .lock()
            .await
            .get(path)
            .cloned()
            .unwrap();
        timeout(Duration::from_secs(2), async {
            loop {
                if mcp_next_retry_in_secs(path).is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let old_abort = session
            .lock()
            .await
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap()
            .recovery_task
            .as_ref()
            .unwrap()
            .abort_handle();
        mcp_session_setup(
            gcx.clone(),
            path.into(),
            serde_json::json!({"version":2}),
            initializer.clone(),
            1,
            1,
            30,
            1,
            true,
        )
        .await;
        timeout(Duration::from_secs(2), async {
            while !old_abort.is_finished() {
                tokio::task::yield_now().await;
            }
            while mcp_next_retry_in_secs(path).is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let new_abort = session
            .lock()
            .await
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap()
            .recovery_task
            .as_ref()
            .unwrap()
            .abort_handle();
        assert!(!new_abort.is_finished());
        assert_eq!(initializer.attempts.load(Ordering::SeqCst), 2);
        remove_mcp_session_for_config(gcx.clone(), path).await;
        timeout(Duration::from_secs(2), async {
            while !new_abort.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!gcx.integration_sessions.lock().await.contains_key(path));
        assert!(mcp_next_retry_in_secs(path).is_none());
        assert_eq!(initializer.attempts.load(Ordering::SeqCst), 2);
    }

    #[async_trait::async_trait]
    impl MCPTransportInitializer for OfflineInitializer {
        async fn init_mcp_transport(
            &self,
            _logs: Arc<AMutex<Vec<String>>>,
            _name: String,
            _init_timeout: u64,
            _request_timeout: u64,
            _session: Arc<AMutex<Box<dyn IntegrationSession>>>,
            _handler: McpClientHandler,
        ) -> Option<McpRunningService> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) + 1 == self.stop_after {
                self.shutdown.store(true, Ordering::SeqCst);
            }
            None
        }
    }

    #[tokio::test]
    async fn reconnect_continues_beyond_burst_and_delay_table() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let session = make_session_arc(MCPConnectionStatus::Disconnected);
        let attempts = Arc::new(AtomicU32::new(0));
        let initializer = OfflineInitializer {
            attempts: attempts.clone(),
            shutdown: gcx.shutdown_flag.clone(),
            stop_after: 11,
        };
        assert!(
            !reconnect_with_backoff(
                session.clone(),
                &initializer,
                Arc::new(AMutex::new(None)),
                Arc::new(AMutex::new(Vec::new())),
                "fixture",
                1,
                1,
                9,
                &[0],
                Arc::downgrade(&gcx),
                0,
            )
            .await
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 11);
        assert_eq!(
            session
                .lock()
                .await
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .unwrap()
                .connection_status,
            MCPConnectionStatus::Reconnecting {
                attempt: 11
            }
        );
    }

    #[tokio::test]
    async fn disabled_or_zero_attempt_reconnect_stops_after_initial_failure() {
        for (enabled, max_attempts) in [(false, 9), (true, 0)] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let path = format!("/tmp/mcp_no_retry_{enabled}_{max_attempts}.yaml");
            let initializer = OfflineInitializer {
                attempts: Arc::new(AtomicU32::new(0)),
                shutdown: gcx.shutdown_flag.clone(),
                stop_after: u32::MAX,
            };
            mcp_session_setup(
                gcx.clone(),
                path.clone(),
                serde_json::json!({}),
                initializer.clone(),
                1,
                1,
                1,
                max_attempts,
                enabled,
            )
            .await;
            let session = gcx
                .integration_sessions
                .lock()
                .await
                .get(&path)
                .cloned()
                .unwrap();
            timeout(
                Duration::from_secs(2),
                super::super::session_mcp::mcp_session_wait_startup(session.clone()),
            )
            .await
            .unwrap();
            assert_eq!(initializer.attempts.load(Ordering::SeqCst), 1);
            {
                let mut locked = session.lock().await;
                let mcp = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
                assert_eq!(
                    mcp.connection_status,
                    MCPConnectionStatus::Failed {
                        message: "Transport initialization failed".into(),
                    }
                );
                assert!(mcp.health_task_handle.is_none());
                assert!(mcp.mcp_client.is_none());
            }
            assert!(mcp_next_retry_in_secs(&path).is_none());
            remove_mcp_session_for_config(gcx, &path).await;
        }
    }

    #[tokio::test]
    async fn removing_session_cancels_startup_and_clears_catalog() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = "/tmp/mcp_remove_fixture.yaml";
        let session = make_session_arc(MCPConnectionStatus::Connecting);
        let (ready, wait) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let abort = task.abort_handle();
        {
            let mut locked = session.lock().await;
            let session = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
            session.config_path = path.into();
            session.startup_task_handles = Some((Arc::new(AMutex::new(Some(task))), abort.clone()));
        }
        wait.await.unwrap();
        gcx.integration_sessions
            .lock()
            .await
            .insert(path.into(), session.clone());
        tool_catalog_cache_store(path, &[]);
        assert!(tool_catalog_cache_get(path).is_some());
        remove_mcp_session_for_config(gcx.clone(), path).await;
        assert!(!gcx.integration_sessions.lock().await.contains_key(path));
        assert!(tool_catalog_cache_get(path).is_none());
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
    }

    #[tokio::test]
    async fn test_build_auth_client_no_tokens_sets_needs_auth() {
        use super::super::session_mcp::SessionMCP;
        use super::super::mcp_metrics::new_shared_metrics;
        use crate::integrations::sessions::IntegrationSession;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config_path = tmp.path().to_str().unwrap().to_string();
        let logs = Arc::new(AMutex::new(Vec::new()));

        let session_arc: Arc<AMutex<Box<dyn IntegrationSession>>> =
            Arc::new(AMutex::new(Box::new(SessionMCP {
                debug_name: "test".to_string(),
                config_path: config_path.clone(),
                launched_cfg: serde_json::Value::Null,
                mcp_client: None,
                mcp_tools: Vec::new(),
                mcp_resources: Vec::new(),
                mcp_prompts: Vec::new(),
                server_info: None,
                startup_task_handles: None,
                health_task_handle: None,
                recovery_task: None,
                lifecycle_epoch: 0,
                logs: logs.clone(),
                stderr_file_path: None,
                stderr_cursor: Arc::new(AMutex::new(0)),
                connection_status: MCPConnectionStatus::Connecting,
                last_successful_connection: None,
                metrics: new_shared_metrics(),
                #[cfg(target_os = "linux")]
                observation: None,
                auth_manager: None,
                auth_status: MCPAuthStatus::NotApplicable,
                oauth_refresh_task_handle: None,
                resource_index_task: None,
                oauth_probe: None,
                sampling_session_approved: false,
                active_progress: Vec::new(),
                last_logged_status: None,
            }) as Box<dyn IntegrationSession>));

        let result = super::build_auth_client_for_mcp(
            "http://localhost:8080",
            &HashMap::new(),
            &config_path,
            "Streamable HTTP",
            logs,
            "test_server",
            session_arc.clone(),
        )
        .await;

        assert!(result.is_none(), "Should return None when no tokens");

        let mut session_locked = session_arc.lock().await;
        let mcp_session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .unwrap();
        assert!(
            matches!(
                mcp_session.connection_status,
                MCPConnectionStatus::NeedsAuth
            ),
            "Status should be NeedsAuth when no tokens, got {:?}",
            mcp_session.connection_status
        );
        assert!(
            matches!(mcp_session.auth_status, MCPAuthStatus::NeedsLogin),
            "Auth status should be NeedsLogin when no tokens, got {:?}",
            mcp_session.auth_status
        );
    }

    #[tokio::test]
    async fn lifecycle_lock_entry_is_pruned_after_last_guard_drops() {
        let key = "/tmp/lifecycle_lock_prune_fixture/integrations.d/mcp_http_x.yaml";
        let guard = mcp_lifecycle_lock(key).await;
        assert!(lifecycle_lock_registered(key));
        let waiter = tokio::spawn(async move {
            let _second = mcp_lifecycle_lock(key).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "a live lock must keep serializing");
        drop(guard);
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(!lifecycle_lock_registered(key));
        drop(mcp_lifecycle_lock("/tmp/lifecycle_lock_prune_fixture/other.yaml").await);
        let locks = MCP_LIFECYCLE_LOCKS.lock().unwrap();
        assert!(!locks.as_ref().unwrap().contains_key(key));
    }

    #[tokio::test]
    async fn aborted_owner_aborts_spawned_serve_child() {
        struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let owner = tokio::spawn({
            let dropped = dropped.clone();
            async move {
                let child = tokio::spawn(async move {
                    let _flag = DropFlag(dropped);
                    std::future::pending::<()>().await;
                });
                let _guard = AbortOnDrop(child.abort_handle());
                let _ = timeout(Duration::from_secs(60), child).await;
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        owner.abort();
        let _ = owner.await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child serve task must be aborted with its owner");
    }
}
