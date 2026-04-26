use std::sync::Arc;
use axum::Extension;
use axum::extract::Query;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::mcp::mcp_naming;
use crate::integrations::mcp::session_mcp::{SessionMCP, MCPConnectionStatus};
use crate::integrations::mcp::mcp_metrics::MCPServerMetrics;
use crate::integrations::running_integrations::load_integrations;

#[derive(Deserialize)]
pub struct McpServerInfoQuery {
    pub config_path: String,
}

#[derive(Deserialize)]
pub struct McpServerReconnectRequest {
    pub config_path: String,
}

#[derive(Serialize)]
struct McpToolInfo {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<serde_json::Value>,
    internal_name: String,
}

#[derive(Serialize)]
struct McpResourceInfo {
    uri: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

#[derive(Serialize)]
struct McpPromptInfo {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize)]
struct McpServerInfoResponse {
    config_path: String,
    status: serde_json::Value,
    auth_status: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol_version: Option<String>,
    tools: Vec<McpToolInfo>,
    resources: Vec<McpResourceInfo>,
    prompts: Vec<McpPromptInfo>,
    capabilities: serde_json::Value,
    logs_tail: Vec<String>,
    metrics: MCPServerMetrics,
}

pub async fn handle_v1_mcp_server_info(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(params): Query<McpServerInfoQuery>,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let session_key = params.config_path.clone();

    let session = gcx
        .read()
        .await
        .integration_sessions
        .get(&session_key)
        .cloned()
        .ok_or(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("no session for {}", session_key),
        ))?;

    let (
        config_path_clone,
        connection_status,
        auth_status,
        server_info,
        tools_raw,
        resources_raw,
        prompts_raw,
        logs_arc,
        metrics_arc,
    ) = {
        let mut session_locked = session.lock().await;
        let mcp_session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .ok_or(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session is not an MCP session".to_string(),
            ))?;
        (
            mcp_session.config_path.clone(),
            mcp_session.connection_status.clone(),
            mcp_session.auth_status.clone(),
            mcp_session.server_info.clone(),
            mcp_session.mcp_tools.clone(),
            mcp_session.mcp_resources.clone(),
            mcp_session.mcp_prompts.clone(),
            mcp_session.logs.clone(),
            mcp_session.metrics.clone(),
        )
    };

    let status = serde_json::to_value(&connection_status).unwrap_or(serde_json::Value::Null);
    let auth_status_json = serde_json::to_value(&auth_status).unwrap_or(serde_json::Value::Null);

    let (server_name, server_version, protocol_version, capabilities_json) =
        if let Some(ref info) = server_info {
            (
                Some(info.server_info.name.clone()),
                Some(info.server_info.version.clone()),
                Some(info.protocol_version.to_string()),
                serde_json::json!({
                    "tools": info.capabilities.tools.is_some(),
                    "resources": info.capabilities.resources.is_some(),
                    "prompts": info.capabilities.prompts.is_some(),
                    "sampling": true,
                }),
            )
        } else {
            (
                None,
                None,
                None,
                serde_json::json!({
                    "tools": false,
                    "resources": false,
                    "prompts": false,
                    "sampling": true,
                }),
            )
        };

    let yaml_name = std::path::Path::new(&config_path_clone)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let shortened_yaml_name = mcp_naming::shorten_config_name(yaml_name);

    let tools: Vec<McpToolInfo> = tools_raw
        .iter()
        .map(|tool| {
            let input_schema = {
                let mut map = tool.input_schema.as_ref().clone();
                if !map.contains_key("type") {
                    map.insert("type".to_string(), serde_json::json!("object"));
                }
                serde_json::Value::Object(map)
            };

            let internal_name = format!("{}_{}", shortened_yaml_name, tool.name)
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>();

            let annotations = tool
                .annotations
                .as_ref()
                .and_then(|a| serde_json::to_value(a).ok());

            McpToolInfo {
                name: tool.name.to_string(),
                description: tool.description.as_deref().unwrap_or_default().to_string(),
                input_schema,
                annotations,
                internal_name,
            }
        })
        .collect();

    let resources: Vec<McpResourceInfo> = resources_raw
        .iter()
        .map(|resource| McpResourceInfo {
            uri: resource.uri.to_string(),
            name: resource.name.to_string(),
            description: resource.description.as_deref().map(|s| s.to_string()),
            mime_type: resource.mime_type.clone().map(|s| s.to_string()),
        })
        .collect();

    let prompts: Vec<McpPromptInfo> = prompts_raw
        .iter()
        .map(|prompt| McpPromptInfo {
            name: prompt.name.to_string(),
            description: prompt.description.as_deref().map(|s| s.to_string()),
        })
        .collect();

    let logs_tail = logs_arc.try_lock().map(|l| l.clone()).unwrap_or_default();

    let metrics = if let Ok(mut m) = metrics_arc.try_lock() {
        m.snapshot()
    } else {
        crate::integrations::mcp::mcp_metrics::MCPServerMetrics::default()
    };

    let response = McpServerInfoResponse {
        config_path: session_key,
        status,
        auth_status: auth_status_json,
        server_name,
        server_version,
        protocol_version,
        tools,
        resources,
        prompts,
        capabilities: capabilities_json,
        logs_tail,
        metrics,
    };

    let payload = serde_json::to_string_pretty(&response).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize: {}", e),
        )
    })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

pub async fn handle_v1_mcp_server_reconnect(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<McpServerReconnectRequest>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let session_key = post.config_path.clone();

    let session = gcx
        .read()
        .await
        .integration_sessions
        .get(&session_key)
        .cloned()
        .ok_or(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("no session for {}", session_key),
        ))?;

    let (client, logs) = {
        let mut session_locked = session.lock().await;
        let mcp_session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .ok_or(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session is not an MCP session".to_string(),
            ))?;

        let reconnecting = matches!(
            &mcp_session.connection_status,
            MCPConnectionStatus::Reconnecting { .. } | MCPConnectionStatus::Connecting
        );
        if reconnecting {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "MCP server is already connecting or reconnecting".to_string(),
            ));
        }

        mcp_session.connection_status = MCPConnectionStatus::Disconnected;
        mcp_session.launched_cfg = serde_json::Value::Null;
        (mcp_session.mcp_client.clone(), mcp_session.logs.clone())
    };

    if let Some(client_arc) = client {
        crate::integrations::mcp::session_mcp::cancel_mcp_client(&session_key, client_arc, logs)
            .await;
    }

    {
        let mut session_locked = session.lock().await;
        let mcp_session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .ok_or(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session is not an MCP session".to_string(),
            ))?;
        mcp_session.mcp_tools = vec![];
        mcp_session.mcp_resources = vec![];
        mcp_session.mcp_prompts = vec![];
        mcp_session.server_info = None;
        mcp_session.connection_status = MCPConnectionStatus::Connecting;
    }

    let config_filename = std::path::Path::new(&session_key)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = load_integrations(
        gcx.clone(),
        &[format!("**/integrations.d/{}", config_filename)],
    )
    .await;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"reconnect_triggered": true}).to_string(),
        ))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp::session_mcp::{MCPAuthStatus};

    #[test]
    fn test_mcp_server_info_response_serializes() {
        let response = McpServerInfoResponse {
            config_path: "mcp_stdio_test.yaml".to_string(),
            status: serde_json::json!({"status": "connected"}),
            auth_status: serde_json::json!("not_applicable"),
            server_name: Some("TestServer".to_string()),
            server_version: Some("1.0.0".to_string()),
            protocol_version: Some("2024-11-05".to_string()),
            tools: vec![McpToolInfo {
                name: "my_tool".to_string(),
                description: "does things".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                annotations: None,
                internal_name: "mcp_test_my_tool".to_string(),
            }],
            resources: vec![],
            prompts: vec![],
            capabilities: serde_json::json!({"tools": true, "resources": false, "prompts": false, "sampling": true}),
            logs_tail: vec!["[12:00:00] Connected".to_string()],
            metrics: MCPServerMetrics::default(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("TestServer"));
        assert!(json.contains("mcp_test_my_tool"));
        assert!(json.contains("Connected"));
        assert!(json.contains("\"metrics\""));
        assert!(json.contains("\"sampling\":true"));
    }

    #[test]
    fn test_mcp_server_info_response_no_server_info() {
        let response = McpServerInfoResponse {
            config_path: "mcp_stdio_test.yaml".to_string(),
            status: serde_json::json!({"status": "disconnected"}),
            auth_status: serde_json::json!("not_applicable"),
            server_name: None,
            server_version: None,
            protocol_version: None,
            tools: vec![],
            resources: vec![],
            prompts: vec![],
            capabilities: serde_json::json!({"tools": false, "resources": false, "prompts": false, "sampling": true}),
            logs_tail: vec![],
            metrics: MCPServerMetrics::default(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert!(
            json.get("server_name").is_none(),
            "server_name should be omitted when None"
        );
        assert!(
            json.get("server_version").is_none(),
            "server_version should be omitted when None"
        );
        assert!(
            json.get("protocol_version").is_none(),
            "protocol_version should be omitted when None"
        );
        assert!(
            json.get("metrics").is_some(),
            "metrics should always be present"
        );
        assert_eq!(json["capabilities"]["sampling"], serde_json::json!(true));
    }

    #[test]
    fn test_mcp_metrics_in_response() {
        use crate::integrations::mcp::mcp_metrics::MCPMetricsCollector;
        use std::time::Instant;
        let mut collector = MCPMetricsCollector::new();
        let start = Instant::now();
        collector.record_call_success("test_tool", start);
        collector.record_call_failure("bad_tool", start);
        let metrics = collector.snapshot();
        assert_eq!(metrics.total_tool_calls, 2);
        assert_eq!(metrics.successful_calls, 1);
        assert_eq!(metrics.failed_calls, 1);
        assert!(metrics.tool_stats.contains_key("test_tool"));
        assert!(metrics.tool_stats.contains_key("bad_tool"));
    }

    #[test]
    fn test_shorten_mcp_yaml_name() {
        assert_eq!(
            mcp_naming::shorten_config_name("mcp_stdio_github"),
            "mcp_github"
        );
        assert_eq!(
            mcp_naming::shorten_config_name("mcp_sse_myserver"),
            "mcp_myserver"
        );
        assert_eq!(
            mcp_naming::shorten_config_name("mcp_http_myserver"),
            "mcp_myserver"
        );
        assert_eq!(
            mcp_naming::shorten_config_name("other_integration"),
            "other_integration"
        );
    }

    #[test]
    fn test_mcp_http_prefix_stripped_in_internal_name() {
        let yaml_name = "mcp_http_myserver";
        let shortened = mcp_naming::shorten_config_name(yaml_name);
        assert_eq!(shortened, "mcp_myserver");
    }

    #[test]
    fn test_reconnect_sets_status_to_connecting() {
        use crate::integrations::mcp::mcp_metrics::new_shared_metrics;
        use tokio::sync::Mutex as AMutex;
        let mut session = SessionMCP {
            debug_name: "test".to_string(),
            config_path: "/tmp/mcp_stdio_test.yaml".to_string(),
            launched_cfg: serde_json::json!({"command": "npx", "args": ["something"]}),
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
        };
        session.connection_status = MCPConnectionStatus::Connecting;
        assert!(matches!(
            session.connection_status,
            MCPConnectionStatus::Connecting
        ));
    }

    #[test]
    fn test_reconnect_resets_launched_cfg_to_null() {
        use crate::integrations::mcp::mcp_metrics::new_shared_metrics;
        use tokio::sync::Mutex as AMutex;
        let mut session = SessionMCP {
            debug_name: "test".to_string(),
            config_path: "/tmp/mcp_stdio_test.yaml".to_string(),
            launched_cfg: serde_json::json!({"command": "npx", "args": ["something"]}),
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
            connection_status: MCPConnectionStatus::Connected,
            last_successful_connection: None,
            metrics: new_shared_metrics(),
            auth_manager: None,
            auth_status: MCPAuthStatus::NotApplicable,
            oauth_refresh_task_handle: None,
        };
        assert_ne!(session.launched_cfg, serde_json::Value::Null);
        session.launched_cfg = serde_json::Value::Null;
        assert_eq!(session.launched_cfg, serde_json::Value::Null);
    }
}
