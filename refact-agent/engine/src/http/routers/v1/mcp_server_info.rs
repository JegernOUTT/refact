use axum::extract::Query;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
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
    active_progress: Vec<crate::integrations::mcp::session_mcp::MCPProgressInfo>,
}

pub async fn handle_v1_mcp_server_info(
    State(app): State<AppState>,
    Query(params): Query<McpServerInfoQuery>,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let session_key = params.config_path.clone();

    let session = {
        let integration_sessions = gcx.integration_sessions.clone();
        let integration_sessions = integration_sessions.lock().await;
        integration_sessions.get(&session_key).cloned()
    }
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
        active_progress,
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
            {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                mcp_session.active_progress.retain(|p| {
                    now_ms - p.updated_at_ms
                        < crate::integrations::mcp::session_mcp::PROGRESS_STALE_MS
                });
                mcp_session.active_progress.clone()
            },
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

    let server_prefix =
        crate::integrations::mcp::integr_mcp_common::tool_name_server_prefix(&config_path_clone);

    let internal_names = {
        let candidates: Vec<(String, String)> = tools_raw
            .iter()
            .map(|tool| {
                (
                    mcp_naming::model_tool_name(&server_prefix, tool.name.as_ref()),
                    tool.name.to_string(),
                )
            })
            .collect();
        mcp_naming::disambiguate_model_tool_names(candidates)
    };
    let tools: Vec<McpToolInfo> = tools_raw
        .iter()
        .zip(internal_names)
        .map(|(tool, internal_name)| {
            let input_schema = {
                let mut map = tool.input_schema.as_ref().clone();
                if !map.contains_key("type") {
                    map.insert("type".to_string(), serde_json::json!("object"));
                }
                serde_json::Value::Object(map)
            };

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
        active_progress,
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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<McpServerReconnectRequest>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let session_key = post.config_path.clone();

    let session = {
        let integration_sessions = gcx.integration_sessions.clone();
        let integration_sessions = integration_sessions.lock().await;
        integration_sessions.get(&session_key).cloned()
    }
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

        let busy = match &mcp_session.connection_status {
            MCPConnectionStatus::Reconnecting {
                ..
            } => true,
            MCPConnectionStatus::Connecting => {
                crate::integrations::mcp::integr_mcp_common::startup_task_in_flight(mcp_session)
            }
            _ => false,
        };
        if busy {
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
