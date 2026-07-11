use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Deserialize;

use crate::custom_error::ScratchError;
use crate::integrations::mcp::mcp_interactions::{
    MCPInteractionResponse, list_mcp_interactions, respond_mcp_interaction,
};

fn json_response(
    status: StatusCode,
    body: &impl serde::Serialize,
) -> Result<Response<Body>, ScratchError> {
    let json = serde_json::to_string(body).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON: {}", e))
    })?;
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response build failed: {}", e),
            )
        })
}

pub async fn handle_v1_mcp_interactions_list() -> Result<Response<Body>, ScratchError> {
    let interactions = list_mcp_interactions().await;
    json_response(
        StatusCode::OK,
        &serde_json::json!({ "interactions": interactions }),
    )
}

#[derive(Deserialize)]
pub struct McpInteractionRespondRequest {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
}

pub async fn handle_v1_mcp_interactions_respond(
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: McpInteractionRespondRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;
    if !matches!(req.action.as_str(), "accept" | "decline" | "cancel") {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "action must be one of: accept, decline, cancel".to_string(),
        ));
    }
    respond_mcp_interaction(
        &req.id,
        MCPInteractionResponse {
            action: req.action,
            content: req.content,
        },
    )
    .await
    .map_err(|e| {
        let status = if e.contains("no longer awaited") {
            StatusCode::GONE
        } else {
            StatusCode::NOT_FOUND
        };
        ScratchError::new(status, e)
    })?;
    json_response(StatusCode::OK, &serde_json::json!({"success": true}))
}
