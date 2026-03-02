use std::sync::Arc;
use axum::Extension;
use axum::extract::Query;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::mcp::session_mcp::{SessionMCP, MCPConnectionStatus};

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

    let mut session_locked = session.lock().await;
    let mcp_session = session_locked
        .as_any_mut()
        .downcast_mut::<SessionMCP>()
        .ok_or(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "session is not an MCP session".to_string(),
        ))?;

    let status = serde_json::to_value(&mcp_session.connection_status).unwrap_or(serde_json::Value::Null);

    let (server_name, server_version, protocol_version, capabilities_json) =
        if let Some(ref info) = mcp_session.server_info {
            (
                Some(info.server_info.name.clone()),
                Some(info.server_info.version.clone()),
                Some(info.protocol_version.to_string()),
                serde_json::json!({
                    "tools": info.capabilities.tools.is_some(),
                    "resources": info.capabilities.resources.is_some(),
                    "prompts": info.capabilities.prompts.is_some(),
                    "sampling": false,
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
                    "sampling": false,
                }),
            )
        };

    let tools: Vec<McpToolInfo> = mcp_session.mcp_tools.iter().map(|tool| {
        let input_schema = {
            let mut map = tool.input_schema.as_ref().clone();
            if !map.contains_key("type") {
                map.insert("type".to_string(), serde_json::json!("object"));
            }
            serde_json::Value::Object(map)
        };

        let yaml_name = std::path::Path::new(&mcp_session.config_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown");
        let shortened_yaml_name = if let Some(stripped) = yaml_name.strip_prefix("mcp_stdio_") {
            format!("mcp_{}", stripped)
        } else if let Some(stripped) = yaml_name.strip_prefix("mcp_sse_") {
            format!("mcp_{}", stripped)
        } else {
            yaml_name.to_string()
        };
        let internal_name = format!("{}_{}", shortened_yaml_name, tool.name)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();

        let annotations = tool.annotations.as_ref().and_then(|a| serde_json::to_value(a).ok());

        McpToolInfo {
            name: tool.name.to_string(),
            description: tool.description.as_deref().unwrap_or_default().to_string(),
            input_schema,
            annotations,
            internal_name,
        }
    }).collect();

    let resources: Vec<McpResourceInfo> = mcp_session.mcp_resources.iter().map(|resource| {
        McpResourceInfo {
            uri: resource.uri.to_string(),
            name: resource.name.to_string(),
            description: resource.description.as_deref().map(|s| s.to_string()),
            mime_type: resource.mime_type.clone().map(|s| s.to_string()),
        }
    }).collect();

    let prompts: Vec<McpPromptInfo> = mcp_session.mcp_prompts.iter().map(|prompt| {
        McpPromptInfo {
            name: prompt.name.to_string(),
            description: prompt.description.as_deref().map(|s| s.to_string()),
        }
    }).collect();

    let logs_tail = mcp_session.logs.try_lock()
        .map(|l| l.clone())
        .unwrap_or_default();

    let response = McpServerInfoResponse {
        config_path: session_key,
        status,
        server_name,
        server_version,
        protocol_version,
        tools,
        resources,
        prompts,
        capabilities: capabilities_json,
        logs_tail,
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
        crate::integrations::mcp::session_mcp::cancel_mcp_client(
            &session_key,
            client_arc,
            logs,
        )
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
        mcp_session.mcp_client = None;
        mcp_session.mcp_tools = vec![];
        mcp_session.mcp_resources = vec![];
        mcp_session.mcp_prompts = vec![];
        mcp_session.server_info = None;
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::json!({"reconnect_triggered": true}).to_string()))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_info_response_serializes() {
        let response = McpServerInfoResponse {
            config_path: "mcp_stdio_test.yaml".to_string(),
            status: serde_json::json!({"status": "connected"}),
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
            capabilities: serde_json::json!({"tools": true, "resources": false, "prompts": false, "sampling": false}),
            logs_tail: vec!["[12:00:00] Connected".to_string()],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("TestServer"));
        assert!(json.contains("mcp_test_my_tool"));
        assert!(json.contains("Connected"));
    }

    #[test]
    fn test_mcp_server_info_response_no_server_info() {
        let response = McpServerInfoResponse {
            config_path: "mcp_stdio_test.yaml".to_string(),
            status: serde_json::json!({"status": "disconnected"}),
            server_name: None,
            server_version: None,
            protocol_version: None,
            tools: vec![],
            resources: vec![],
            prompts: vec![],
            capabilities: serde_json::json!({"tools": false, "resources": false, "prompts": false, "sampling": false}),
            logs_tail: vec![],
        };

        let json = serde_json::to_value(&response).unwrap();
        assert!(json.get("server_name").is_none(), "server_name should be omitted when None");
        assert!(json.get("server_version").is_none(), "server_version should be omitted when None");
        assert!(json.get("protocol_version").is_none(), "protocol_version should be omitted when None");
    }
}
