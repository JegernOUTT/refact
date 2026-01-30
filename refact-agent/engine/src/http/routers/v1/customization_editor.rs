use axum::extract::Path;
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::{load_merged_registry, load_registry_from_dir, invalidate_all_registry_caches, ConfigScope};
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

async fn invalidate_registry_cache(gcx: Arc<ARwLock<GlobalContext>>, scope: ConfigScope) {
    match scope {
        ConfigScope::Global => {
            invalidate_all_registry_caches(gcx).await;
        }
        ConfigScope::Local => {
            invalidate_all_registry_caches(gcx).await;
        }
    }
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
    pub scope: String,
    pub global_path: String,
    pub local_path: String,
    pub global_exists: bool,
    pub local_exists: bool,
}

#[derive(Serialize)]
pub struct ErrorItem {
    pub file_path: String,
    pub error: String,
}

pub async fn handle_v1_customization_registry(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;
    let _global_registry = load_registry_from_dir(&config_dir).await;

    let local_refact_dir = project_root.as_ref().map(|p| p.join(".refact"));

    let make_config_item = |id: &str, kind: &str, title: &str, specific: bool| -> ConfigItem {
        let global_path = config_dir.join(kind).join(format!("{}.yaml", id));
        let local_path = local_refact_dir.as_ref().map(|d| d.join(kind).join(format!("{}.yaml", id)));
        let global_exists = global_path.exists();
        let local_exists = local_path.as_ref().map(|p| p.exists()).unwrap_or(false);
        let effective_scope = if local_exists { "local" } else { "global" };
        let effective_path = if local_exists {
            local_path.as_ref().unwrap().display().to_string()
        } else {
            global_path.display().to_string()
        };
        ConfigItem {
            id: id.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            file_path: effective_path,
            specific,
            scope: effective_scope.to_string(),
            global_path: global_path.display().to_string(),
            local_path: local_path.map(|p| p.display().to_string()).unwrap_or_default(),
            global_exists,
            local_exists,
        }
    };

    let response = RegistryResponse {
        modes: registry.modes.values().map(|m| {
            make_config_item(&m.id, "modes", if m.title.is_empty() { &m.id } else { &m.title }, m.specific)
        }).collect(),
        subagents: registry.subagents.values().map(|s| {
            make_config_item(&s.id, "subagents", if s.title.is_empty() { &s.id } else { &s.title }, s.specific)
        }).collect(),
        toolbox_commands: registry.toolbox_commands.values().map(|t| {
            make_config_item(&t.id, "toolbox_commands", &t.id, false)
        }).collect(),
        code_lens: registry.code_lens.values().map(|c| {
            make_config_item(&c.id, "code_lens", if c.label.is_empty() { &c.id } else { &c.label }, false)
        }).collect(),
        errors: registry.errors.iter().map(|e| ErrorItem {
            file_path: e.file_path.clone(),
            error: e.error.clone(),
        }).collect(),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct GetConfigQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Serialize)]
pub struct ConfigDetailResponse {
    pub config: serde_json::Value,
    pub file_path: String,
    pub raw_yaml: String,
    pub scope: String,
}

pub async fn handle_v1_customization_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((kind, id)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<GetConfigQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_kind(&kind) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Err(e) = validate_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let global_path = config_dir.join(&kind).join(format!("{}.yaml", id));
    let local_path = project_root.as_ref().map(|p| p.join(".refact").join(&kind).join(format!("{}.yaml", id)));

    let (file_path, scope) = match query.scope.as_deref() {
        Some("global") => (global_path, ConfigScope::Global),
        Some("local") => {
            match local_path {
                Some(p) => (p, ConfigScope::Local),
                None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
            }
        }
        _ => {
            if local_path.as_ref().map(|p| p.exists()).unwrap_or(false) {
                (local_path.unwrap(), ConfigScope::Local)
            } else {
                (global_path, ConfigScope::Global)
            }
        }
    };

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
        scope: match scope { ConfigScope::Global => "global", ConfigScope::Local => "local" }.to_string(),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct SaveConfigRequest {
    pub config: serde_json::Value,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Serialize)]
pub struct SaveConfigResponse {
    pub ok: bool,
    pub file_path: String,
    pub scope: String,
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

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let request: SaveConfigRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    if let Err(e) = validate_config(&kind, &request.config, &id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let global_path = config_dir.join(&kind).join(format!("{}.yaml", id));
    let local_path = project_root.as_ref().map(|p| p.join(".refact").join(&kind).join(format!("{}.yaml", id)));

    let (file_path, scope) = match request.scope.as_deref() {
        Some("global") => (global_path, ConfigScope::Global),
        Some("local") => {
            match local_path {
                Some(p) => (p, ConfigScope::Local),
                None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
            }
        }
        _ => {
            if local_path.as_ref().map(|p| p.exists()).unwrap_or(false) {
                (local_path.unwrap(), ConfigScope::Local)
            } else if global_path.exists() {
                (global_path, ConfigScope::Global)
            } else {
                match local_path {
                    Some(p) => (p, ConfigScope::Local),
                    None => (global_path, ConfigScope::Global),
                }
            }
        }
    };

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

    invalidate_registry_cache(gcx.clone(), scope).await;
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let response = SaveConfigResponse {
        ok: registry.errors.is_empty(),
        file_path: file_path.display().to_string(),
        scope: match scope { ConfigScope::Global => "global", ConfigScope::Local => "local" }.to_string(),
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
    #[serde(default)]
    pub scope: Option<String>,
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

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let (file_path, scope) = match request.scope.as_deref() {
        Some("global") => (config_dir.join(&kind).join(format!("{}.yaml", request.id)), ConfigScope::Global),
        Some("local") => {
            match &project_root {
                Some(p) => (p.join(".refact").join(&kind).join(format!("{}.yaml", request.id)), ConfigScope::Local),
                None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
            }
        }
        _ => {
            match &project_root {
                Some(p) => (p.join(".refact").join(&kind).join(format!("{}.yaml", request.id)), ConfigScope::Local),
                None => (config_dir.join(&kind).join(format!("{}.yaml", request.id)), ConfigScope::Global),
            }
        }
    };

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

    invalidate_registry_cache(gcx.clone(), scope).await;
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let response = SaveConfigResponse {
        ok: registry.errors.is_empty(),
        file_path: file_path.display().to_string(),
        scope: match scope { ConfigScope::Global => "global", ConfigScope::Local => "local" }.to_string(),
        errors: registry.errors.iter().map(|e| ErrorItem {
            file_path: e.file_path.clone(),
            error: e.error.clone(),
        }).collect(),
    };

    json_response(StatusCode::CREATED, &response)
}

#[derive(Deserialize)]
pub struct DeleteConfigQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Serialize)]
pub struct DeleteConfigResponse {
    pub ok: bool,
    pub scope: String,
    pub errors: Vec<ErrorItem>,
}

pub async fn handle_v1_customization_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((kind, id)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<DeleteConfigQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_kind(&kind) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Err(e) = validate_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let (file_path, scope) = match query.scope.as_deref() {
        Some("global") => (config_dir.join(&kind).join(format!("{}.yaml", id)), ConfigScope::Global),
        Some("local") => {
            match &project_root {
                Some(p) => (p.join(".refact").join(&kind).join(format!("{}.yaml", id)), ConfigScope::Local),
                None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
            }
        }
        _ => return json_error(StatusCode::BAD_REQUEST, "scope parameter required for delete"),
    };

    if !file_path.exists() {
        return json_error(StatusCode::NOT_FOUND, "config not found");
    }

    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("delete error: {}", e));
    }

    invalidate_registry_cache(gcx.clone(), scope).await;
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let response = DeleteConfigResponse {
        ok: true,
        scope: match scope { ConfigScope::Global => "global", ConfigScope::Local => "local" }.to_string(),
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
