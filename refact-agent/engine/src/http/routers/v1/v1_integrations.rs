use std::path::PathBuf;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Deserialize;
use regex::Regex;
use axum::extract::Path;
use axum::extract::Query;
use rust_embed::RustEmbed;
use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::integrations::mcp::management::delete_integration_config;
use crate::integrations::mcp::session_mcp::SessionMCP;

pub async fn handle_v1_integrations(
    State(app): State<AppState>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let integrations =
        crate::integrations::setting_up_integrations::integrations_all(gcx.clone(), true).await;
    let payload = serde_json::to_string_pretty(&integrations).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize payload: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

pub async fn handle_v1_integrations_filtered(
    State(app): State<AppState>,
    Path(integr_name): Path<String>,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let integrations_result: crate::integrations::setting_up_integrations::IntegrationResult =
        crate::integrations::setting_up_integrations::integrations_all(gcx.clone(), true).await;
    let mut filtered_integrations = Vec::new();

    for integration in &integrations_result.integrations {
        let pattern = integration.integr_name.replace("_TEMPLATE", "_.*");
        match Regex::new(&pattern) {
            Ok(re) => {
                if re.is_match(&integr_name) {
                    let mut integration_copy = integration.clone();
                    integration_copy.integr_name = integr_name.clone();
                    if let Some(pos) = integration
                        .integr_config_path
                        .rfind(&integration.integr_name)
                    {
                        let (start, end) = integration.integr_config_path.split_at(pos);
                        integration_copy.integr_config_path = format!(
                            "{}{}{}",
                            start,
                            integr_name,
                            &end[integration.integr_name.len()..]
                        );
                    }
                    if integration.integr_name.find("_TEMPLATE").is_some() {
                        let config_path_exists =
                            integrations_result
                                .integrations
                                .iter()
                                .any(|existing_integration| {
                                    existing_integration.integr_config_path
                                        == integration_copy.integr_config_path
                                });
                        if config_path_exists {
                            continue;
                        }
                    }
                    filtered_integrations.push(integration_copy);
                }
            }
            Err(e) => {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid regex pattern: {}", e),
                ));
            }
        }
    }

    let payload = serde_json::to_string_pretty(
        &crate::integrations::setting_up_integrations::IntegrationResult {
            integrations: filtered_integrations,
            error_log: integrations_result.error_log,
        },
    )
    .map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize payload: {}", e),
        )
    })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

#[derive(Deserialize)]
struct IntegrationGetPost {
    pub integr_config_path: String,
}

pub async fn handle_v1_integration_get(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<IntegrationGetPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let the_get = crate::integrations::setting_up_integrations::integration_config_get(
        gcx.clone(),
        post.integr_config_path,
    )
    .await
    .map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load integrations: {}", e),
        )
    })?;

    let payload = serde_json::to_string_pretty(&the_get).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize payload: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

#[derive(Deserialize)]
struct IntegrationSavePost {
    pub integr_config_path: String,
    pub integr_values: serde_json::Value,
}

pub async fn handle_v1_integration_save(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<IntegrationSavePost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    crate::integrations::setting_up_integrations::integration_config_save(
        gcx.clone(),
        &post.integr_config_path,
        &post.integr_values,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(format!("")))
        .unwrap())
}

#[derive(RustEmbed)]
#[folder = "assets/integrations/"]
struct IntegrationAsset;

pub async fn handle_v1_integration_icon(
    Path(icon_name): Path<String>,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let sanitized_icon_name = icon_name
        .split('/')
        .last()
        .map(|x| x.replace("_TEMPLATE", ""))
        .ok_or(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "invalid file name".to_string(),
        ))?;
    if let Some(icon_bytes) = IntegrationAsset::get(&sanitized_icon_name).map(|file| file.data) {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .header("Content-Disposition", "inline")
            .body(Body::from(icon_bytes))
            .unwrap());
    }
    Err(ScratchError::new(
        StatusCode::NOT_FOUND,
        format!("icon {} not found", sanitized_icon_name),
    ))
}

// Define a structure to match query parameters
#[derive(Deserialize)]
pub struct HTTPIntegrationDeleteQueryParams {
    integration_path: PathBuf,
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn v1_integrations_delete_removes_live_session_and_cached_tools() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(dir.path().to_path_buf());
        let config_dir = dir.path().join(".refact/integrations.d");
        std::fs::create_dir_all(&config_dir).unwrap();
        let path = config_dir.join("mcp_http_delete.yaml");
        std::fs::write(&path, "mcp_url: http://localhost:1\n").unwrap();
        let key = path.to_string_lossy().into_owned();
        let session = SessionMCP::detached(&key);
        gcx.integration_sessions
            .lock()
            .await
            .insert(key.clone(), Arc::new(Mutex::new(Box::new(session))));
        let tool = serde_json::from_value(
            serde_json::json!({"name":"read", "inputSchema":{"type":"object"}}),
        )
        .unwrap();
        crate::integrations::mcp::integr_mcp_common::tool_catalog_cache_store(&key, &[tool]);
        assert!(gcx.integration_sessions.lock().await.contains_key(&key));
        let before =
            crate::http::routers::v1::privacy::handle_v1_privacy_policy_get(State(app.clone()))
                .await
                .unwrap()
                .0;
        let server = crate::integrations::mcp::mcp_interactions::server_name_from_config_path(&key);
        assert!(before
            .destinations
            .iter()
            .any(|destination| destination.id.0 == server));
        let response = handle_v1_integration_delete(
            State(app.clone()),
            Query(HTTPIntegrationDeleteQueryParams {
                integration_path: path.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!path.exists());
        assert!(!gcx.integration_sessions.lock().await.contains_key(&key));
        assert!(
            crate::integrations::mcp::integr_mcp_common::tool_catalog_cache_get(&key).is_none()
        );
        let after = crate::http::routers::v1::privacy::handle_v1_privacy_policy_get(State(app))
            .await
            .unwrap()
            .0;
        assert!(!after
            .destinations
            .iter()
            .any(|destination| destination.id.0 == server));
    }
}

pub async fn handle_v1_integration_delete(
    State(app): State<AppState>,
    Query(params): Query<HTTPIntegrationDeleteQueryParams>,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let integration_path = params.integration_path;
    log::info!("Deleting integration path: {:?}", integration_path);
    delete_integration_config(app.gcx, &integration_path).await?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .unwrap())
}

#[derive(Deserialize)]
pub struct IntegrationsMcpLogsRequest {
    pub config_path: String,
}

pub async fn handle_v1_integrations_mcp_logs(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<IntegrationsMcpLogsRequest>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let session_key = post.config_path;
    let session = {
        let integration_sessions = gcx.integration_sessions.clone();
        let integration_sessions = integration_sessions.lock().await;
        integration_sessions.get(&session_key).cloned()
    }
    .ok_or(ScratchError::new(
        StatusCode::NOT_FOUND,
        format!("session {} not found", session_key),
    ))?;

    let (logs_arc, stderr_file_path, stderr_cursor) = {
        let mut session_locked = session.lock().await;
        let session_downcasted = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .ok_or(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Session is not a MCP session".to_string(),
            ))?;
        (
            session_downcasted.logs.clone(),
            session_downcasted.stderr_file_path.clone(),
            session_downcasted.stderr_cursor.clone(),
        )
    };

    if let Some(stderr_path) = &stderr_file_path {
        if let Err(e) = crate::integrations::mcp::session_mcp::update_logs_from_stderr(
            stderr_path,
            stderr_cursor,
            logs_arc.clone(),
        )
        .await
        {
            tracing::warn!("Failed to read stderr file: {}", e);
        }
    }

    return Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "logs": logs_arc.lock().await.clone(),
            })
            .to_string(),
        ))
        .unwrap());
}
