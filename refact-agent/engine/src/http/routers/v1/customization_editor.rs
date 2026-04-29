use axum::extract::Path;
use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::{
    load_merged_registry, load_registry_from_dir, invalidate_all_registry_caches, ConfigScope,
};
use crate::yaml_configs::customization_types::*;
use crate::yaml_configs::project_configs_bootstrap::{
    global_configs_try_create_all, project_configs_ensure_dirs,
};
use crate::buddy::drafts::{draft_kind_str, DraftTarget, DraftValidationError};
use crate::buddy::types::DraftKind;

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

fn json_response<T: Serialize>(
    status: StatusCode,
    data: &T,
) -> Result<Response<Body>, ScratchError> {
    let body_str = serde_json::to_string(data).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization error: {}", e),
        )
    })?;
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body_str))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn draft_error_response(err: DraftValidationError) -> Result<Response<Body>, ScratchError> {
    match err {
        DraftValidationError::NotFound => json_error(StatusCode::NOT_FOUND, "draft_not_found"),
        DraftValidationError::KindMismatch { expected, actual } => json_error(
            StatusCode::CONFLICT,
            &format!(
                "draft_kind_mismatch: expected {}, got {}",
                draft_kind_str(&expected),
                draft_kind_str(&actual)
            ),
        ),
        DraftValidationError::TargetMismatch { expected, actual } => json_error(
            StatusCode::CONFLICT,
            &format!(
                "draft_target_mismatch: expected {}, got {}",
                expected, actual
            ),
        ),
        DraftValidationError::Parse(err) => json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("draft_parse_failed: {}", err),
        ),
    }
}

async fn validate_draft_for_target(
    gcx: Arc<ARwLock<GlobalContext>>,
    draft_id: &str,
    expected_kind: DraftKind,
    target_id: &str,
) -> std::result::Result<(), DraftValidationError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let Some(svc) = lock.as_ref() else {
        return Err(DraftValidationError::NotFound);
    };
    svc.draft_store
        .get_validated(draft_id, expected_kind, DraftTarget::Id(target_id))
        .map(|_| ())
}

async fn consume_draft_for_target(
    gcx: Arc<ARwLock<GlobalContext>>,
    draft_id: &str,
    expected_kind: DraftKind,
    target_id: &str,
) -> std::result::Result<ConsumedDraft, DraftValidationError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let Some(svc) = lock.as_mut() else {
        return Err(DraftValidationError::NotFound);
    };
    svc.consume_validated_draft(draft_id, expected_kind, DraftTarget::Id(target_id))
        .map(|d| ConsumedDraft { draft_id: d.id })
}

async fn write_file_atomic(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let tmp_path: PathBuf = parent.join(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4()));
    let write_result = async {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        tokio::fs::rename(&tmp_path, path).await
    }
    .await;
    if write_result.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
    write_result
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

fn normalize_kind(kind: &str) -> &str {
    if kind == "delegates" {
        "subagents"
    } else {
        kind
    }
}

fn customization_kind_to_draft_kind(kind: &str) -> Option<DraftKind> {
    match normalize_kind(kind) {
        "modes" => Some(DraftKind::Mode),
        "subagents" => Some(DraftKind::Delegate),
        _ => None,
    }
}

#[derive(Serialize)]
struct DraftMetadata {
    draft_id: String,
    kind: String,
    title: String,
    explanation: String,
    source_opportunity_id: Option<String>,
    expires_at: String,
}

#[derive(Serialize)]
struct DraftConfigResponse {
    data: serde_json::Value,
    draft_metadata: DraftMetadata,
}

#[derive(Serialize)]
pub struct ConsumedDraft {
    pub draft_id: String,
}

#[derive(Serialize)]
pub struct RegistryResponse {
    pub modes: Vec<ConfigItem>,
    pub subagents: Vec<ConfigItem>,
    pub toolbox_commands: Vec<ConfigItem>,
    pub code_lens: Vec<ConfigItem>,
    pub errors: Vec<ErrorItem>,
    pub has_project_root: bool,
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

    let _ = global_configs_try_create_all(&config_dir).await;
    if let Some(ref root) = project_root {
        let _ = project_configs_ensure_dirs(root).await;
    }

    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;
    let _global_registry = load_registry_from_dir(&config_dir).await;
    let local_refact_dir = project_root.as_ref().map(|p| p.join(".refact"));

    let make_config_item = |id: &str, kind: &str, title: &str, specific: bool| -> ConfigItem {
        let global_path = config_dir.join(kind).join(format!("{}.yaml", id));
        let local_path = local_refact_dir
            .as_ref()
            .map(|d| d.join(kind).join(format!("{}.yaml", id)));
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
            local_path: local_path
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            global_exists,
            local_exists,
        }
    };

    let mut modes: Vec<_> = Vec::new();
    let mut seen_mode_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for m in registry.modes.values() {
        if seen_mode_ids.insert(m.id.clone()) {
            modes.push(make_config_item(
                &m.id,
                "modes",
                if m.title.is_empty() { &m.id } else { &m.title },
                m.specific,
            ));
        }
    }

    for m in &registry.mode_overrides {
        if seen_mode_ids.insert(m.id.clone()) {
            modes.push(make_config_item(
                &m.id,
                "modes",
                if m.title.is_empty() { &m.id } else { &m.title },
                m.specific,
            ));
        }
    }

    modes.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));

    let mut subagents: Vec<_> = registry
        .subagents
        .values()
        .map(|s| {
            make_config_item(
                &s.id,
                "subagents",
                if s.title.is_empty() { &s.id } else { &s.title },
                s.specific,
            )
        })
        .collect();
    subagents.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));

    let mut toolbox_commands: Vec<_> = registry
        .toolbox_commands
        .values()
        .map(|t| make_config_item(&t.id, "toolbox_commands", &t.id, false))
        .collect();
    toolbox_commands.sort_by(|a, b| a.id.cmp(&b.id));

    let mut code_lens: Vec<_> = registry
        .code_lens
        .values()
        .map(|c| {
            make_config_item(
                &c.id,
                "code_lens",
                if c.label.is_empty() { &c.id } else { &c.label },
                false,
            )
        })
        .collect();
    code_lens.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));

    let response = RegistryResponse {
        modes,
        subagents,
        toolbox_commands,
        code_lens,
        errors: registry
            .errors
            .iter()
            .map(|e| ErrorItem {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
        has_project_root: project_root.is_some(),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct GetConfigQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
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
    let storage_kind = normalize_kind(&kind).to_string();

    if let Some(ref draft_id) = query.draft_id {
        let Some(expected_kind) = customization_kind_to_draft_kind(&kind) else {
            return json_error(StatusCode::BAD_REQUEST, "drafts_not_supported_for_kind");
        };
        let draft_data = {
            let buddy_arc = gcx.read().await.buddy.clone();
            let lock = buddy_arc.lock().await;
            match lock.as_ref() {
                Some(svc) => match svc.draft_store.get_validated(
                    draft_id,
                    expected_kind,
                    DraftTarget::Id(&id),
                ) {
                    Ok(draft) => Ok((
                        draft.id.clone(),
                        draft.kind,
                        draft.title.clone(),
                        draft.explanation.clone(),
                        draft.expires_at,
                        draft.yaml_or_json.clone(),
                    )),
                    Err(err) => Err(err),
                },
                None => Err(DraftValidationError::NotFound),
            }
        };
        return match draft_data {
            Err(err) => draft_error_response(err),
            Ok((did, dkind, title, explanation, expires_at, yaml_or_json)) => {
                let data: serde_json::Value = match serde_yaml::from_str(&yaml_or_json) {
                    Ok(v) => v,
                    Err(e) => {
                        return json_error(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            &format!("draft_parse_failed: {}", e),
                        )
                    }
                };
                json_response(
                    StatusCode::OK,
                    &DraftConfigResponse {
                        data,
                        draft_metadata: DraftMetadata {
                            draft_id: did,
                            kind: draft_kind_str(&dkind).to_string(),
                            title,
                            explanation,
                            source_opportunity_id: None,
                            expires_at: expires_at.to_rfc3339(),
                        },
                    },
                )
            }
        };
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let global_path = config_dir.join(&storage_kind).join(format!("{}.yaml", id));
    let local_path = project_root.as_ref().map(|p| {
        p.join(".refact")
            .join(&storage_kind)
            .join(format!("{}.yaml", id))
    });

    let (file_path, scope) = match query.scope.as_deref() {
        Some("global") => (global_path, ConfigScope::Global),
        Some("local") => match local_path {
            Some(p) => (p, ConfigScope::Local),
            None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
        },
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
        scope: match scope {
            ConfigScope::Global => "global",
            ConfigScope::Local => "local",
        }
        .to_string(),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct SaveConfigRequest {
    pub config: serde_json::Value,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
}

#[derive(Serialize)]
pub struct SaveConfigResponse {
    pub ok: bool,
    pub file_path: String,
    pub scope: String,
    pub errors: Vec<ErrorItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_draft: Option<ConsumedDraft>,
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
    let storage_kind = normalize_kind(&kind).to_string();

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let request: SaveConfigRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    if let Err(e) = validate_config(&storage_kind, &request.config, &id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Some(ref draft_id) = request.draft_id {
        let Some(expected_kind) = customization_kind_to_draft_kind(&kind) else {
            return json_error(StatusCode::BAD_REQUEST, "drafts_not_supported_for_kind");
        };
        if let Err(err) = validate_draft_for_target(gcx.clone(), draft_id, expected_kind, &id).await
        {
            return draft_error_response(err);
        }
    }

    let global_path = config_dir.join(&storage_kind).join(format!("{}.yaml", id));
    let local_path = project_root.as_ref().map(|p| {
        p.join(".refact")
            .join(&storage_kind)
            .join(format!("{}.yaml", id))
    });

    let (file_path, scope) = match request.scope.as_deref() {
        Some("global") => (global_path, ConfigScope::Global),
        Some("local") => match local_path {
            Some(p) => (p, ConfigScope::Local),
            None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
        },
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
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("yaml serialize error: {}", e),
            )
        }
    };

    if let Err(e) = write_file_atomic(&file_path, &yaml_str).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write error: {}", e),
        );
    }

    invalidate_registry_cache(gcx.clone(), scope).await;
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let consumed_draft = if registry.errors.is_empty() {
        if let Some(ref draft_id) = request.draft_id {
            let expected_kind = customization_kind_to_draft_kind(&kind).ok_or_else(|| {
                ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "drafts_not_supported_for_kind".to_string(),
                )
            })?;
            match consume_draft_for_target(gcx.clone(), draft_id, expected_kind, &id).await {
                Ok(consumed) => Some(consumed),
                Err(err) => return draft_error_response(err),
            }
        } else {
            None
        }
    } else {
        None
    };

    let response = SaveConfigResponse {
        ok: registry.errors.is_empty(),
        file_path: file_path.display().to_string(),
        scope: match scope {
            ConfigScope::Global => "global",
            ConfigScope::Local => "local",
        }
        .to_string(),
        errors: registry
            .errors
            .iter()
            .map(|e| ErrorItem {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
        consumed_draft,
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
    let storage_kind = normalize_kind(&kind).to_string();

    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let (file_path, scope) = match request.scope.as_deref() {
        Some("global") => (
            config_dir
                .join(&storage_kind)
                .join(format!("{}.yaml", request.id)),
            ConfigScope::Global,
        ),
        Some("local") => match &project_root {
            Some(p) => (
                p.join(".refact")
                    .join(&storage_kind)
                    .join(format!("{}.yaml", request.id)),
                ConfigScope::Local,
            ),
            None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
        },
        _ => match &project_root {
            Some(p) => (
                p.join(".refact")
                    .join(&storage_kind)
                    .join(format!("{}.yaml", request.id)),
                ConfigScope::Local,
            ),
            None => (
                config_dir
                    .join(&storage_kind)
                    .join(format!("{}.yaml", request.id)),
                ConfigScope::Global,
            ),
        },
    };

    if let Err(e) = validate_config(&storage_kind, &request.config, &request.id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    if let Some(parent) = file_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let yaml_str = match serde_yaml::to_string(&request.config) {
        Ok(s) => s,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("yaml serialize error: {}", e),
            )
        }
    };

    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file_path)
        .await
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return json_error(StatusCode::CONFLICT, "config already exists");
        }
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("write error: {}", e),
            )
        }
    };
    if let Err(e) = file.write_all(yaml_str.as_bytes()).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write error: {}", e),
        );
    }

    invalidate_registry_cache(gcx.clone(), scope).await;
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let response = SaveConfigResponse {
        ok: registry.errors.is_empty(),
        file_path: file_path.display().to_string(),
        scope: match scope {
            ConfigScope::Global => "global",
            ConfigScope::Local => "local",
        }
        .to_string(),
        errors: registry
            .errors
            .iter()
            .map(|e| ErrorItem {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
        consumed_draft: None,
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
    let storage_kind = normalize_kind(&kind).to_string();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let (file_path, scope) = match query.scope.as_deref() {
        Some("global") => (
            config_dir.join(&storage_kind).join(format!("{}.yaml", id)),
            ConfigScope::Global,
        ),
        Some("local") => match &project_root {
            Some(p) => (
                p.join(".refact")
                    .join(&storage_kind)
                    .join(format!("{}.yaml", id)),
                ConfigScope::Local,
            ),
            None => return json_error(StatusCode::BAD_REQUEST, "no project root for local scope"),
        },
        _ => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "scope parameter required for delete",
            )
        }
    };

    if !file_path.exists() {
        return json_error(StatusCode::NOT_FOUND, "config not found");
    }

    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("delete error: {}", e),
        );
    }

    invalidate_registry_cache(gcx.clone(), scope).await;
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let response = DeleteConfigResponse {
        ok: true,
        scope: match scope {
            ConfigScope::Global => "global",
            ConfigScope::Local => "local",
        }
        .to_string(),
        errors: registry
            .errors
            .iter()
            .map(|e| ErrorItem {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
    };

    json_response(StatusCode::OK, &response)
}

fn validate_kind(kind: &str) -> std::result::Result<&str, String> {
    match kind {
        "modes" | "subagents" | "delegates" | "toolbox_commands" | "code_lens" => Ok(kind),
        "hooks" => Err("hooks are managed by ext hooks routes".to_string()),
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
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(
            "id must contain only lowercase letters, digits, underscore, or hyphen".to_string(),
        );
    }
    Ok(())
}

const MAX_SUPPORTED_SCHEMA_VERSION: u32 = 100;

fn validate_config(
    kind: &str,
    config: &serde_json::Value,
    expected_id: &str,
) -> std::result::Result<(), String> {
    let kind = normalize_kind(kind);
    let config_id = config.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if config_id != expected_id {
        return Err(format!(
            "config id '{}' does not match expected '{}'",
            config_id, expected_id
        ));
    }

    let schema_version = config
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing or invalid schema_version field".to_string())?;
    if schema_version == 0 || schema_version > MAX_SUPPORTED_SCHEMA_VERSION as u64 {
        return Err(format!(
            "schema_version {} is not supported (must be 1..={})",
            schema_version, MAX_SUPPORTED_SCHEMA_VERSION
        ));
    }

    let json_str = serde_json::to_string(config).map_err(|e| e.to_string())?;
    match kind {
        "modes" => {
            serde_json::from_str::<ModeConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        "subagents" => {
            serde_json::from_str::<SubagentConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        "toolbox_commands" => {
            serde_json::from_str::<ToolboxCommandConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        "code_lens" => {
            serde_json::from_str::<CodeLensConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        _ => {
            return Err(format!("unknown kind: {}", kind));
        }
    }
    Ok(())
}
