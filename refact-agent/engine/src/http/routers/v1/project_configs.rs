use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::load_merged_registry;
use crate::yaml_configs::project_configs_bootstrap::{
    global_configs_try_create_all, project_configs_ensure_dirs,
};

#[derive(Deserialize)]
pub struct RescanRequest {
    #[serde(default)]
    pub project_root: Option<String>,
}

#[derive(Serialize)]
pub struct RescanResponse {
    pub ok: bool,
    pub modes_loaded: usize,
    pub subagents_loaded: usize,
    pub toolbox_commands_loaded: usize,
    pub code_lens_loaded: usize,
    pub errors: Vec<RegistryErrorResponse>,
}

#[derive(Serialize)]
pub struct RegistryErrorResponse {
    pub file_path: String,
    pub error: String,
}

fn json_error_response(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({"error": message});
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn validate_project_root(
    gcx: Arc<ARwLock<GlobalContext>>,
    requested: Option<String>,
) -> Result<std::path::PathBuf, Response<Body>> {
    let dirs = get_project_dirs(gcx).await;
    match requested {
        Some(path) => {
            let requested_path = std::path::PathBuf::from(&path);
            let canonical_requested = requested_path
                .canonicalize()
                .map(|path| dunce::simplified(&path).to_path_buf())
                .unwrap_or_else(|_| requested_path.clone());
            let matched = dirs.iter().any(|d| {
                d.canonicalize()
                    .map(|path| dunce::simplified(&path).to_path_buf())
                    .unwrap_or_else(|_| d.clone())
                    == canonical_requested
            });
            if matched {
                Ok(requested_path)
            } else {
                Err(json_error_response(
                    StatusCode::BAD_REQUEST,
                    "Invalid project root: not a workspace directory",
                ))
            }
        }
        None => dirs.into_iter().next().ok_or_else(|| {
            json_error_response(StatusCode::BAD_REQUEST, "No project root available")
        }),
    }
}

pub async fn handle_v1_project_configs_rescan(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: RescanRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let project_root = match validate_project_root(gcx.clone(), request.project_root).await {
        Ok(path) => path,
        Err(response) => return Ok(response),
    };

    let config_dir = gcx.read().await.config_dir.clone();
    let registry = load_merged_registry(&config_dir, Some(&project_root)).await;

    let response = RescanResponse {
        ok: registry.errors.is_empty(),
        modes_loaded: registry.modes.len(),
        subagents_loaded: registry.subagents.len(),
        toolbox_commands_loaded: registry.toolbox_commands.len(),
        code_lens_loaded: registry.code_lens.len(),
        errors: registry
            .errors
            .iter()
            .map(|e| RegistryErrorResponse {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

#[derive(Serialize)]
pub struct ProjectConfigsResponse {
    pub modes: Vec<ModeInfo>,
    pub subagents: Vec<SubagentInfo>,
    pub toolbox_commands: Vec<ToolboxInfo>,
    pub code_lens: Vec<CodeLensInfo>,
    pub errors: Vec<RegistryErrorResponse>,
}

#[derive(Serialize)]
pub struct ModeInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub specific: bool,
}

#[derive(Serialize)]
pub struct SubagentInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub expose_as_tool: bool,
    pub has_code: bool,
}

#[derive(Serialize)]
pub struct ToolboxInfo {
    pub id: String,
    pub description: String,
}

#[derive(Serialize)]
pub struct CodeLensInfo {
    pub id: String,
    pub label: String,
}

pub async fn handle_v1_project_configs_bootstrap(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: RescanRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let project_root = match validate_project_root(gcx.clone(), request.project_root).await {
        Ok(path) => path,
        Err(response) => return Ok(response),
    };

    let config_dir = gcx.read().await.config_dir.clone();
    let _ = global_configs_try_create_all(&config_dir).await;
    match project_configs_ensure_dirs(&project_root).await {
        Ok(_) => {
            let registry = load_merged_registry(&config_dir, Some(&project_root)).await;
            let response = RescanResponse {
                ok: registry.errors.is_empty(),
                modes_loaded: registry.modes.len(),
                subagents_loaded: registry.subagents.len(),
                toolbox_commands_loaded: registry.toolbox_commands.len(),
                code_lens_loaded: registry.code_lens.len(),
                errors: registry
                    .errors
                    .iter()
                    .map(|e| RegistryErrorResponse {
                        file_path: e.file_path.clone(),
                        error: e.error.clone(),
                    })
                    .collect(),
            };
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&response).unwrap()))
                .unwrap())
        }
        Err(e) => Ok(json_error_response(StatusCode::INTERNAL_SERVER_ERROR, &e)),
    }
}

pub async fn handle_v1_project_configs_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let dirs = get_project_dirs(gcx.clone()).await;

    let project_root = match dirs.first() {
        Some(dir) => dir.clone(),
        None => {
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"modes":[],"subagents":[],"toolbox_commands":[],"code_lens":[],"errors":[]}"#))
                .unwrap());
        }
    };

    let config_dir = gcx.read().await.config_dir.clone();
    let _ = global_configs_try_create_all(&config_dir).await;
    let _ = project_configs_ensure_dirs(&project_root).await;
    let registry = load_merged_registry(&config_dir, Some(&project_root)).await;

    let response = ProjectConfigsResponse {
        modes: registry
            .modes
            .values()
            .filter(|m| !m.specific)
            .map(|m| ModeInfo {
                id: m.id.clone(),
                title: m.title.clone(),
                description: m.description.clone(),
                specific: m.specific,
            })
            .collect(),
        subagents: registry
            .subagents
            .values()
            .filter(|s| s.expose_as_tool)
            .map(|s| SubagentInfo {
                id: s.id.clone(),
                title: s.title.clone(),
                description: s.description.clone(),
                expose_as_tool: s.expose_as_tool,
                has_code: s.has_code,
            })
            .collect(),
        toolbox_commands: registry
            .toolbox_commands
            .values()
            .map(|t| ToolboxInfo {
                id: t.id.clone(),
                description: t.description.clone(),
            })
            .collect(),
        code_lens: registry
            .code_lens
            .values()
            .map(|c| CodeLensInfo {
                id: c.id.clone(),
                label: c.label.clone(),
            })
            .collect(),
        errors: registry
            .errors
            .iter()
            .map(|e| RegistryErrorResponse {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}
