use axum::extract::Path;
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::load_project_registry;
use crate::yaml_configs::customization_types::*;

fn json_error(status: StatusCode, msg: &str) -> Result<Response<Body>, ScratchError> {
    let body = serde_json::json!({"error": msg});
    let body_str = serde_json::to_string(&body)
        .unwrap_or_else(|_| r#"{"error":"serialization error"}"#.to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body_str))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn json_response<T: Serialize>(status: StatusCode, data: &T) -> Result<Response<Body>, ScratchError> {
    let body_str = serde_json::to_string(data)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON serialization error: {}", e)))?;
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body_str))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn invalidate_registry_cache(gcx: Arc<ARwLock<GlobalContext>>, project_root: &std::path::Path) {
    let cache_arc = {
        let gcx_locked = gcx.read().await;
        gcx_locked.project_registry_cache.clone()
    };
    if let Ok(mut cache) = cache_arc.write() {
        cache.remove(project_root);
    };
}

#[derive(Serialize)]
pub struct RegistryResponse {
    pub modes: Vec<ConfigItem>,
    pub subagents: Vec<ConfigItem>,
    pub toolbox_commands: Vec<ConfigItem>,
    pub code_lens: Vec<ConfigItem>,
    pub errors: Vec<ErrorItem>,
}

#[derive(Serialize)]
pub struct ConfigItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub file_path: String,
    pub specific: bool,
}

#[derive(Serialize)]
pub struct ErrorItem {
    pub file_path: String,
    pub error: String,
}

pub async fn handle_v1_customization_registry(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = match dirs.first() {
        Some(dir) => dir.clone(),
        None => {
            let empty = RegistryResponse {
                modes: vec![],
                subagents: vec![],
                toolbox_commands: vec![],
                code_lens: vec![],
                errors: vec![],
            };
            return json_response(StatusCode::OK, &empty);
        }
    };

    let registry = load_project_registry(&project_root).await;
    let refact_dir = project_root.join(".refact");

    let response = RegistryResponse {
        modes: registry.modes.values().map(|m| ConfigItem {
            id: m.id.clone(),
            kind: "modes".to_string(),
            title: if m.title.is_empty() { m.id.clone() } else { m.title.clone() },
            file_path: refact_dir.join("modes").join(format!("{}.yaml", m.id)).display().to_string(),
            specific: m.specific,
        }).collect(),
        subagents: registry.subagents.values().map(|s| ConfigItem {
            id: s.id.clone(),
            kind: "subagents".to_string(),
            title: if s.title.is_empty() { s.id.clone() } else { s.title.clone() },
            file_path: refact_dir.join("subagents").join(format!("{}.yaml", s.id)).display().to_string(),
            specific: s.specific,
        }).collect(),
        toolbox_commands: registry.toolbox_commands.values().map(|t| ConfigItem {
            id: t.id.clone(),
            kind: "toolbox_commands".to_string(),
            title: t.id.clone(),
            file_path: refact_dir.join("toolbox_commands").join(format!("{}.yaml", t.id)).display().to_string(),
            specific: false,
        }).collect(),
        code_lens: registry.code_lens.values().map(|c| ConfigItem {
            id: c.id.clone(),
            kind: "code_lens".to_string(),
            title: if c.label.is_empty() { c.id.clone() } else { c.label.clone() },
            file_path: refact_dir.join("code_lens").join(format!("{}.yaml", c.id)).display().to_string(),
            specific: false,
        }).collect(),
        errors: registry.errors.iter().map(|e| ErrorItem {
            file_path: e.file_path.clone(),
            error: e.error.clone(),
        }).collect(),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Serialize)]
pub struct ConfigDetailResponse {
    pub config: serde_json::Value,
    pub file_path: String,
    pub raw_yaml: String,
}

pub async fn handle_v1_customization_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_kind(&kind) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Err(e) = validate_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = match dirs.first() {
        Some(dir) => dir.clone(),
        None => return json_error(StatusCode::BAD_REQUEST, "no project root"),
    };

    let file_path = get_config_path(&project_root, &kind, &id);
    if !file_path.exists() {
        return json_error(StatusCode::NOT_FOUND, "config not found");
    }

    let raw_yaml = match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => content,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let config: serde_json::Value = match serde_yaml::from_str(&raw_yaml) {
        Ok(v) => v,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("yaml parse error: {}", e)),
    };

    let response = ConfigDetailResponse {
        config,
        file_path: file_path.display().to_string(),
        raw_yaml,
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct SaveConfigRequest {
    pub config: serde_json::Value,
}

#[derive(Serialize)]
pub struct SaveConfigResponse {
    pub ok: bool,
    pub file_path: String,
    pub errors: Vec<ErrorItem>,
}

pub async fn handle_v1_customization_save(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((kind, id)): Path<(String, String)>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_kind(&kind) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Err(e) = validate_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = match dirs.first() {
        Some(dir) => dir.clone(),
        None => return json_error(StatusCode::BAD_REQUEST, "no project root"),
    };

    let request: SaveConfigRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    if let Err(e) = validate_config(&kind, &request.config, &id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let file_path = get_config_path(&project_root, &kind, &id);
    if let Some(parent) = file_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let yaml_str = match serde_yaml::to_string(&request.config) {
        Ok(s) => s,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("yaml serialize error: {}", e)),
    };

    if let Err(e) = tokio::fs::write(&file_path, &yaml_str).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("write error: {}", e));
    }

    invalidate_registry_cache(gcx.clone(), &project_root).await;
    let registry = load_project_registry(&project_root).await;

    let response = SaveConfigResponse {
        ok: registry.errors.is_empty(),
        file_path: file_path.display().to_string(),
        errors: registry.errors.iter().map(|e| ErrorItem {
            file_path: e.file_path.clone(),
            error: e.error.clone(),
        }).collect(),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct CreateConfigRequest {
    pub id: String,
    pub config: serde_json::Value,
}

pub async fn handle_v1_customization_create(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(kind): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_kind(&kind) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let request: CreateConfigRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    if let Err(e) = validate_id(&request.id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = match dirs.first() {
        Some(dir) => dir.clone(),
        None => return json_error(StatusCode::BAD_REQUEST, "no project root"),
    };

    let file_path = get_config_path(&project_root, &kind, &request.id);
    if file_path.exists() {
        return json_error(StatusCode::CONFLICT, "config already exists");
    }

    if let Err(e) = validate_config(&kind, &request.config, &request.id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    if let Some(parent) = file_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let yaml_str = match serde_yaml::to_string(&request.config) {
        Ok(s) => s,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("yaml serialize error: {}", e)),
    };

    if let Err(e) = tokio::fs::write(&file_path, &yaml_str).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("write error: {}", e));
    }

    invalidate_registry_cache(gcx.clone(), &project_root).await;
    let registry = load_project_registry(&project_root).await;

    let response = SaveConfigResponse {
        ok: registry.errors.is_empty(),
        file_path: file_path.display().to_string(),
        errors: registry.errors.iter().map(|e| ErrorItem {
            file_path: e.file_path.clone(),
            error: e.error.clone(),
        }).collect(),
    };

    json_response(StatusCode::CREATED, &response)
}

#[derive(Serialize)]
pub struct DeleteConfigResponse {
    pub ok: bool,
    pub errors: Vec<ErrorItem>,
}

pub async fn handle_v1_customization_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_kind(&kind) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Err(e) = validate_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = match dirs.first() {
        Some(dir) => dir.clone(),
        None => return json_error(StatusCode::BAD_REQUEST, "no project root"),
    };

    let file_path = get_config_path(&project_root, &kind, &id);
    if !file_path.exists() {
        return json_error(StatusCode::NOT_FOUND, "config not found");
    }

    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("delete error: {}", e));
    }

    invalidate_registry_cache(gcx.clone(), &project_root).await;
    let registry = load_project_registry(&project_root).await;

    let response = DeleteConfigResponse {
        ok: true,
        errors: registry.errors.iter().map(|e| ErrorItem {
            file_path: e.file_path.clone(),
            error: e.error.clone(),
        }).collect(),
    };

    json_response(StatusCode::OK, &response)
}

fn validate_kind(kind: &str) -> std::result::Result<&str, String> {
    match kind {
        "modes" | "subagents" | "toolbox_commands" | "code_lens" => Ok(kind),
        _ => Err(format!("invalid kind: {}", kind)),
    }
}

fn validate_id(id: &str) -> std::result::Result<(), String> {
    if id.is_empty() {
        return Err("id cannot be empty".to_string());
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("id contains invalid characters".to_string());
    }
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err("id must contain only lowercase letters, digits, underscore, or hyphen".to_string());
    }
    Ok(())
}

fn get_config_path(project_root: &std::path::Path, kind: &str, id: &str) -> PathBuf {
    project_root.join(".refact").join(kind).join(format!("{}.yaml", id))
}

fn validate_config(kind: &str, config: &serde_json::Value, expected_id: &str) -> std::result::Result<(), String> {
    let config_id = config.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if config_id != expected_id {
        return Err(format!("config id '{}' does not match expected '{}'", config_id, expected_id));
    }
    let yaml_str = serde_json::to_string(config).map_err(|e| e.to_string())?;
    match kind {
        "modes" => {
            serde_json::from_str::<ModeConfig>(&yaml_str).map_err(|e| e.to_string())?;
        }
        "subagents" => {
            serde_json::from_str::<SubagentConfig>(&yaml_str).map_err(|e| e.to_string())?;
        }
        "toolbox_commands" => {
            serde_json::from_str::<ToolboxCommandConfig>(&yaml_str).map_err(|e| e.to_string())?;
        }
        "code_lens" => {
            serde_json::from_str::<CodeLensConfig>(&yaml_str).map_err(|e| e.to_string())?;
        }
        _ => {
            return Err(format!("unknown kind: {}", kind));
        }
    }
    Ok(())
}
