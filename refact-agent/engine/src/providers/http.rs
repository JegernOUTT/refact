use axum::extract::{Path, Query};
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::buddy::drafts::{draft_kind_str, DraftTarget, DraftValidationError};
use crate::buddy::types::DraftKind;
use crate::caps::model_caps::get_model_caps;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

fn json_response(
    status: StatusCode,
    body: &impl Serialize,
) -> Result<Response<Body>, ScratchError> {
    let json = serde_json::to_string(body).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization failed: {}", e),
        )
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

async fn invalidate_caps(gcx: Arc<ARwLock<GlobalContext>>) {
    let mut gcx_locked = gcx.write().await;
    gcx_locked.caps = None;
    gcx_locked.caps_last_attempted_ts = 0;
}
use crate::providers::config::ProviderDefaults;
use crate::providers::registry::{
    create_provider, delete_provider_config, save_provider_config, PROVIDER_NAMES,
};
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelSource, ProviderModel, ProviderRuntime,
};
use super::openrouter::OpenRouterProvider;
use super::google_gemini::GoogleGeminiProvider;
use super::claude_code::ClaudeCodeProvider;
use super::openai_codex::{OpenAICodexProvider, UsageRequestError};

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[derive(Serialize)]
struct ProviderListItem {
    name: &'static str,
    display_name: &'static str,
    enabled: bool,
    readonly: bool,
    has_credentials: bool,
    status: &'static str,
    model_count: usize,
}

#[derive(Serialize)]
struct ProviderListResponse {
    providers: Vec<ProviderListItem>,
}

pub async fn handle_v1_providers_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;

    let mut providers = Vec::new();
    for name in PROVIDER_NAMES {
        if let Some(provider) = registry.get(name) {
            if provider.is_hidden_from_list() {
                continue;
            }
            let (enabled, readonly) = match provider.build_runtime() {
                Ok(runtime) => (runtime.enabled, provider.is_readonly()),
                Err(_) => (false, provider.is_readonly()),
            };
            let has_creds = provider.has_credentials();
            let model_count = provider.selected_model_count();
            let status = if has_creds && model_count > 0 && enabled {
                "active"
            } else if has_creds {
                "configured"
            } else {
                "not_configured"
            };
            providers.push(ProviderListItem {
                name: provider.name(),
                display_name: provider.display_name(),
                enabled,
                readonly,
                has_credentials: has_creds,
                status,
                model_count,
            });
        } else if let Some(default_provider) = create_provider(name) {
            if default_provider.is_hidden_from_list() {
                continue;
            }
            providers.push(ProviderListItem {
                name: default_provider.name(),
                display_name: default_provider.display_name(),
                enabled: false,
                readonly: default_provider.is_readonly(),
                has_credentials: false,
                status: "not_configured",
                model_count: 0,
            });
        }
    }

    let response = ProviderListResponse { providers };
    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct ProviderPathParams {
    name: String,
}

#[derive(Deserialize)]
pub struct ProviderModelPathParams {
    name: String,
    model_id: String,
}

#[derive(Serialize)]
pub struct OpenRouterModelEndpointsResponse {
    pub provider_variants: Vec<crate::providers::traits::ProviderVariant>,
    pub available_providers: Vec<String>,
}

#[derive(Serialize)]
struct ProviderDetailResponse {
    name: String,
    display_name: String,
    enabled: bool,
    readonly: bool,
    has_credentials: bool,
    selected_models_count: usize,
    status: &'static str,
    settings: serde_json::Value,
    runtime: Option<ProviderRuntime>,
}

pub async fn handle_v1_provider_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;

    let provider: Box<dyn crate::providers::traits::ProviderTrait> =
        if let Some(p) = registry.get(&params.name) {
            p.clone_box()
        } else if let Some(p) = create_provider(&params.name) {
            p
        } else {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", params.name),
            ));
        };

    let runtime = provider.build_runtime().ok();
    let has_creds = provider.has_credentials();
    let selected_count = provider.selected_model_count();
    let enabled = runtime.as_ref().map(|r| r.enabled).unwrap_or(false);
    let status = if has_creds && selected_count > 0 && enabled {
        "active"
    } else if has_creds {
        "configured"
    } else {
        "not_configured"
    };
    let response = ProviderDetailResponse {
        name: provider.name().to_string(),
        display_name: provider.display_name().to_string(),
        enabled,
        readonly: provider.is_readonly(),
        has_credentials: has_creds,
        selected_models_count: selected_count,
        status,
        settings: provider.provider_settings_as_json(),
        runtime: runtime.map(|r| r.redacted()),
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Serialize)]
struct ProviderSchemaResponse {
    name: String,
    schema: String,
}

pub async fn handle_v1_provider_schema(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let schema = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;

        if let Some(provider) = registry.get(&params.name) {
            provider.provider_schema().to_string()
        } else if let Some(default_provider) = create_provider(&params.name) {
            default_provider.provider_schema().to_string()
        } else {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", params.name),
            ));
        }
    };

    let response = ProviderSchemaResponse {
        name: params.name,
        schema,
    };

    json_response(StatusCode::OK, &response)
}

pub async fn handle_v1_provider_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let settings: serde_yaml::Value =
        if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            serde_yaml::to_value(json_val).map_err(|e| {
                ScratchError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("Failed to convert JSON to YAML: {}", e),
                )
            })?
        } else {
            serde_yaml::from_slice(&body_bytes).map_err(|e| {
                ScratchError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("Invalid JSON/YAML: {}", e),
                )
            })?
        };

    let config_dir = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        if let Some(provider) = registry.get(&params.name) {
            if provider.is_readonly() {
                return Err(ScratchError::new(
                    StatusCode::FORBIDDEN,
                    format!("Provider '{}' is readonly", params.name),
                ));
            }
        }
        gcx_locked.config_dir.clone()
    };

    if create_provider(&params.name).is_none() {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("Unknown provider type '{}'", params.name),
        ));
    }

    let settings = strip_derived_fields(settings);
    let merged_settings =
        merge_provider_settings_preserving_secrets(&config_dir, &params.name, settings).await?;

    save_provider_config(&config_dir, &params.name, merged_settings)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let provider_path = config_dir
            .join("providers.d")
            .join(format!("{}.yaml", params.name));
        let content = tokio::fs::read_to_string(&provider_path)
            .await
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to reload config: {}", e),
                )
            })?;

        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Invalid YAML after save: {}", e),
            )
        })?;

        let mut provider = create_provider(&params.name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create provider".to_string(),
            )
        })?;

        provider.provider_settings_apply(yaml).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to apply settings: {}", e),
            )
        })?;

        registry.add(provider);
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true}))
}

pub async fn handle_v1_provider_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    if create_provider(&params.name).is_none() {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("Unknown provider type '{}'", params.name),
        ));
    }

    let config_dir = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        if let Some(provider) = registry.get(&params.name) {
            if provider.is_readonly() {
                return Err(ScratchError::new(
                    StatusCode::FORBIDDEN,
                    format!("Provider '{}' is readonly", params.name),
                ));
            }
        }
        gcx_locked.config_dir.clone()
    };

    delete_provider_config(&config_dir, &params.name)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(default_provider) = create_provider(&params.name) {
            registry.add(default_provider);
        }
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true}))
}

#[derive(Serialize)]
struct ProviderModelsResponse {
    models: Vec<crate::providers::traits::ProviderModel>,
}

pub async fn handle_v1_provider_models(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;

    let provider: Box<dyn crate::providers::traits::ProviderTrait> =
        if let Some(p) = registry.get(&params.name) {
            p.clone_box()
        } else if let Some(p) = create_provider(&params.name) {
            p
        } else {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", params.name),
            ));
        };

    let runtime = provider
        .build_runtime()
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let mut models = runtime.chat_models;
    models.extend(runtime.completion_models);
    if let Some(emb) = runtime.embedding_model {
        models.push(emb);
    }

    let response = ProviderModelsResponse { models };

    json_response(StatusCode::OK, &response)
}

pub async fn handle_v1_defaults_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let config_dir = gcx.read().await.config_dir.clone();
    let defaults = ProviderDefaults::load(&config_dir)
        .await
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    json_response(StatusCode::OK, &defaults)
}

#[derive(Deserialize)]
struct DefaultsUpdateRequest {
    #[serde(default)]
    draft_id: Option<String>,
    #[serde(flatten)]
    defaults: ProviderDefaults,
}

fn defaults_draft_validation_error(err: DraftValidationError) -> ScratchError {
    match err {
        DraftValidationError::NotFound => {
            ScratchError::new(StatusCode::NOT_FOUND, "draft_not_found".to_string())
        }
        DraftValidationError::KindMismatch { expected, actual } => ScratchError::new(
            StatusCode::CONFLICT,
            format!(
                "draft_kind_mismatch: expected {}, got {}",
                draft_kind_str(&expected),
                draft_kind_str(&actual)
            ),
        ),
        DraftValidationError::TargetMismatch { expected, actual } => ScratchError::new(
            StatusCode::CONFLICT,
            format!(
                "draft_target_mismatch: expected {}, got {}",
                expected, actual
            ),
        ),
        DraftValidationError::Parse(err) => ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("draft_parse_failed: {}", err),
        ),
    }
}

fn validate_defaults_draft_shape(content: &str) -> Result<(), ScratchError> {
    let value: Value = serde_json::from_str(content).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("draft_parse_failed: {}", e),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "draft_parse_failed: defaults draft must be a JSON object".to_string(),
        )
    })?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "chat"
                | "chat_light"
                | "chat_thinking"
                | "chat_buddy"
                | "completion_model"
                | "embedding_model"
        ) {
            return Err(ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("draft_parse_failed: unsupported defaults key {}", key),
            ));
        }
    }
    serde_json::from_value::<ProviderDefaults>(value).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("draft_parse_failed: {}", e),
        )
    })?;
    Ok(())
}

async fn validate_defaults_draft(
    gcx: Arc<ARwLock<GlobalContext>>,
    draft_id: &str,
) -> Result<(), ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let svc = lock.as_ref().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".to_string(),
        )
    })?;
    let content = svc
        .draft_store
        .get_validated(draft_id, DraftKind::DefaultsModel, DraftTarget::Any)
        .map_err(defaults_draft_validation_error)?
        .yaml_or_json
        .clone();
    drop(lock);
    validate_defaults_draft_shape(&content)
}

async fn consume_defaults_draft(
    gcx: Arc<ARwLock<GlobalContext>>,
    draft_id: &str,
) -> Result<(), ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".to_string(),
        )
    })?;
    svc.consume_validated_draft(draft_id, DraftKind::DefaultsModel, DraftTarget::Any)
        .map(|_| ())
        .map_err(defaults_draft_validation_error)
}

pub async fn handle_v1_defaults_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: DefaultsUpdateRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    if let Some(draft_id) = req.draft_id.as_deref() {
        validate_defaults_draft(gcx.clone(), draft_id).await?;
    }

    let config_dir = gcx.read().await.config_dir.clone();
    req.defaults
        .save(&config_dir)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if let Some(draft_id) = req.draft_id.as_deref() {
        consume_defaults_draft(gcx.clone(), draft_id).await?;
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true}))
}

// /v1/models endpoint - returns models grouped by type for frontend compatibility
#[derive(Deserialize)]
pub struct ModelsQueryParams {
    #[serde(rename = "provider-name")]
    provider_name: String,
}

#[derive(Serialize)]
struct SimplifiedModel {
    name: String,
    enabled: bool,
    removable: bool,
    user_configured: bool,
}

#[derive(Serialize)]
struct ModelsResponse {
    chat_models: Vec<SimplifiedModel>,
    completion_models: Vec<SimplifiedModel>,
    embedding_model: Option<SimplifiedModel>,
}

impl From<&ProviderModel> for SimplifiedModel {
    fn from(model: &ProviderModel) -> Self {
        SimplifiedModel {
            name: model.id.clone(),
            enabled: model.enabled,
            removable: model.removable,
            user_configured: model.user_configured,
        }
    }
}

pub async fn handle_v1_models(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(params): Query<ModelsQueryParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;

    let provider: Box<dyn crate::providers::traits::ProviderTrait> =
        if let Some(p) = registry.get(&params.provider_name) {
            p.clone_box()
        } else if let Some(p) = create_provider(&params.provider_name) {
            p
        } else {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", params.provider_name),
            ));
        };

    let runtime = provider
        .build_runtime()
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = ModelsResponse {
        chat_models: runtime
            .chat_models
            .iter()
            .map(SimplifiedModel::from)
            .collect(),
        completion_models: runtime
            .completion_models
            .iter()
            .map(SimplifiedModel::from)
            .collect(),
        embedding_model: runtime.embedding_model.as_ref().map(SimplifiedModel::from),
    };

    json_response(StatusCode::OK, &response)
}

// ============================================================================
// Available Models Management Endpoints
// ============================================================================

#[derive(Serialize)]
pub struct AvailableModelsResponse {
    models: Vec<AvailableModel>,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
pub struct OpenRouterHealthResponse {
    ok: bool,
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<crate::providers::openrouter::OpenRouterHealthInfo>,
}

#[derive(Serialize)]
pub struct GoogleGeminiHealthResponse {
    ok: bool,
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<crate::providers::google_gemini::GoogleGeminiHealthInfo>,
}

/// GET /v1/providers/{name}/available-models
/// Fetches all available models for a provider from model_caps or API
pub async fn handle_v1_provider_available_models(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let http_client = gcx_locked.http_client.clone();

        let provider: Box<dyn crate::providers::traits::ProviderTrait> =
            if let Some(p) = registry.get(&params.name) {
                p.clone_box()
            } else if let Some(p) = create_provider(&params.name) {
                p
            } else {
                return Err(ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Provider '{}' not found", params.name),
                ));
            };

        (provider, http_client)
    };

    let source = provider.model_source();
    let (model_caps, caps_error) = match get_model_caps(gcx.clone(), false).await {
        Ok(caps) => (caps, None),
        Err(e) => {
            tracing::warn!(
                "Failed to fetch model_caps for provider '{}': {}",
                params.name,
                e
            );
            (
                HashMap::new(),
                Some(format!(
                    "Failed to fetch model capabilities: {}. Model limits may be inaccurate.",
                    e
                )),
            )
        }
    };
    let models = provider
        .fetch_available_models(&http_client, &model_caps)
        .await;
    let error = caps_error;

    let source_str = match source {
        ModelSource::ModelCaps => "model_caps",
        ModelSource::Api => "api",
        ModelSource::Local => "local",
        ModelSource::Manual => "manual",
    };

    let response = AvailableModelsResponse {
        models,
        source: source_str.to_string(),
        error,
    };

    json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
pub struct ModelToggleRequest {
    pub model_id: String,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct ModelProviderRequest {
    pub model_id: String,
    #[serde(default)]
    pub selected_provider: Option<String>,
}

/// POST /v1/providers/{name}/models/toggle
/// Enable or disable a model for a provider
/// Body: { "model_id": "claude-3-5-sonnet", "enabled": true }
pub async fn handle_v1_provider_model_toggle(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: ModelToggleRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    // Validate model_id
    if request.model_id.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Model ID cannot be empty".to_string(),
        ));
    }
    if request.model_id.len() > 256 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Model ID too long (max 256 characters)".to_string(),
        ));
    }

    update_model_enabled_state(gcx, &params.name, &request.model_id, request.enabled).await
}

/// POST /v1/providers/{name}/models/provider
/// Set preferred upstream provider for a model (OpenRouter)
/// Body: { "model_id": "openai/gpt-4.1", "selected_provider": "openai" }
pub async fn handle_v1_provider_model_provider_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: ModelProviderRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    if request.model_id.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Model ID cannot be empty".to_string(),
        ));
    }

    if let Some(ref provider) = request.selected_provider {
        if provider.len() > 128 {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "Selected provider is too long (max 128 characters)".to_string(),
            ));
        }
    }

    update_model_selected_provider_state(
        gcx,
        &params.name,
        &request.model_id,
        request.selected_provider,
    )
    .await
}

/// GET /v1/providers/openrouter/models/:model_id/endpoints
pub async fn handle_v1_openrouter_model_endpoints(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderModelPathParams>,
) -> Result<Response<Body>, ScratchError> {
    if params.name != "openrouter" {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            "Provider does not support endpoints lookup".to_string(),
        ));
    }

    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry
            .get(&params.name)
            .map(|p| p.clone_box())
            .or_else(|| create_provider(&params.name))
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Provider '{}' not found", params.name),
                )
            })?;
        (provider, gcx_locked.http_client.clone())
    };

    let Some(openrouter) = provider.as_any().downcast_ref::<OpenRouterProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve OpenRouter provider type".to_string(),
        ));
    };

    let (provider_variants, available_providers) = openrouter
        .fetch_model_endpoints(&http_client, &params.model_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::BAD_GATEWAY, e))?;

    json_response(
        StatusCode::OK,
        &OpenRouterModelEndpointsResponse {
            provider_variants,
            available_providers,
        },
    )
}

/// GET /v1/openrouter/account-info
pub async fn handle_v1_openrouter_account_info(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry
            .get("openrouter")
            .map(|p| p.clone_box())
            .or_else(|| create_provider("openrouter"))
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    "OpenRouter provider is not available".to_string(),
                )
            })?;
        (provider, gcx_locked.http_client.clone())
    };

    let Some(openrouter) = provider.as_any().downcast_ref::<OpenRouterProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve OpenRouter provider type".to_string(),
        ));
    };

    let account_info = openrouter
        .fetch_account_info(&http_client)
        .await
        .map_err(|e| ScratchError::new(StatusCode::BAD_GATEWAY, e))?;

    json_response(StatusCode::OK, &json!({"data": account_info}))
}

/// GET /v1/openrouter/health
pub async fn handle_v1_openrouter_health(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry
            .get("openrouter")
            .map(|p| p.clone_box())
            .or_else(|| create_provider("openrouter"))
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    "OpenRouter provider is not available".to_string(),
                )
            })?;
        (provider, gcx_locked.http_client.clone())
    };

    let Some(openrouter) = provider.as_any().downcast_ref::<OpenRouterProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve OpenRouter provider type".to_string(),
        ));
    };

    match openrouter.check_api_key_health(&http_client).await {
        Ok(info) => json_response(
            StatusCode::OK,
            &OpenRouterHealthResponse {
                ok: true,
                message: None,
                data: Some(info),
            },
        ),
        Err(e) => json_response(
            StatusCode::OK,
            &OpenRouterHealthResponse {
                ok: false,
                message: Some(e),
                data: None,
            },
        ),
    }
}

/// GET /v1/google-gemini/health
pub async fn handle_v1_google_gemini_health(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry
            .get("google_gemini")
            .map(|p| p.clone_box())
            .or_else(|| create_provider("google_gemini"))
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    "Google Gemini provider is not available".to_string(),
                )
            })?;
        (provider, gcx_locked.http_client.clone())
    };

    let Some(gemini) = provider.as_any().downcast_ref::<GoogleGeminiProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve Google Gemini provider type".to_string(),
        ));
    };

    match gemini.check_api_key_health(&http_client).await {
        Ok(info) => json_response(
            StatusCode::OK,
            &GoogleGeminiHealthResponse {
                ok: true,
                message: None,
                data: Some(info),
            },
        ),
        Err(e) => json_response(
            StatusCode::OK,
            &GoogleGeminiHealthResponse {
                ok: false,
                message: Some(e),
                data: None,
            },
        ),
    }
}

async fn update_model_enabled_state(
    gcx: Arc<ARwLock<GlobalContext>>,
    provider_name: &str,
    model_id: &str,
    enabled: bool,
) -> Result<Response<Body>, ScratchError> {
    // Capture previous state for rollback
    let (config_dir, previous_enabled_models, previous_disabled_models) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        // Auto-create default provider if not yet configured (e.g. first model toggle on Ollama)
        if registry.get(provider_name).is_none() {
            let default_provider = create_provider(provider_name).ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Unknown provider type '{}'", provider_name),
                )
            })?;
            registry.add(default_provider);
        }

        let provider = registry.get_mut(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", provider_name),
            )
        })?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", provider_name),
            ));
        }

        // Capture previous state for rollback
        let previous_enabled = provider.enabled_models().to_vec();
        let previous_disabled = provider.disabled_models().to_vec();

        provider.set_model_enabled(model_id, enabled);
        (
            gcx_locked.config_dir.clone(),
            previous_enabled,
            previous_disabled,
        )
    };

    // Try to save updated config
    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        // Rollback in-memory state on persistence failure
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(provider_name) {
            for model in provider.enabled_models().to_vec() {
                provider.set_model_enabled(&model, false);
            }
            for model in provider.disabled_models().to_vec() {
                provider.set_model_enabled(&model, true);
            }
            for model in &previous_enabled_models {
                provider.set_model_enabled(model, true);
            }
            for model in &previous_disabled_models {
                provider.set_model_enabled(model, false);
            }
        }
        return Err(e);
    }

    reload_provider_from_disk(gcx.clone(), provider_name, &config_dir).await?;

    invalidate_caps(gcx).await;

    json_response(
        StatusCode::OK,
        &json!({"success": true, "model_id": model_id, "enabled": enabled}),
    )
}

async fn update_model_selected_provider_state(
    gcx: Arc<ARwLock<GlobalContext>>,
    provider_name: &str,
    model_id: &str,
    selected_provider: Option<String>,
) -> Result<Response<Body>, ScratchError> {
    let (config_dir, previous_selected_provider) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        if registry.get(provider_name).is_none() {
            let default_provider = create_provider(provider_name).ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Unknown provider type '{}'", provider_name),
                )
            })?;
            registry.add(default_provider);
        }

        let provider = registry.get_mut(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", provider_name),
            )
        })?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", provider_name),
            ));
        }

        let prev = provider.selected_providers().get(model_id).cloned();
        provider.set_selected_provider(model_id, selected_provider.clone());
        (gcx_locked.config_dir.clone(), prev)
    };

    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(provider_name) {
            provider.set_selected_provider(model_id, previous_selected_provider);
        }
        return Err(e);
    }

    invalidate_caps(gcx).await;

    json_response(
        StatusCode::OK,
        &json!({
            "success": true,
            "model_id": model_id,
            "selected_provider": selected_provider
        }),
    )
}

#[derive(Deserialize)]
pub struct AddCustomModelRequest {
    id: String,
    #[serde(flatten)]
    config: CustomModelConfig,
}

/// POST /v1/providers/{name}/custom-models
/// Add a custom model to a provider
pub async fn handle_v1_provider_add_custom_model(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: AddCustomModelRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    // Validate model_id
    if request.id.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Model ID cannot be empty".to_string(),
        ));
    }

    let (config_dir, previous_config) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        if registry.get(&params.name).is_none() {
            let default_provider = create_provider(&params.name).ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Unknown provider type '{}'", params.name),
                )
            })?;
            registry.add(default_provider);
        }

        let provider = registry.get_mut(&params.name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", params.name),
            )
        })?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", params.name),
            ));
        }

        let previous_config = provider.custom_models().get(&request.id).cloned();
        provider.add_custom_model(request.id.clone(), request.config.clone());
        (gcx_locked.config_dir.clone(), previous_config)
    };

    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, &params.name).await {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(&params.name) {
            if let Some(previous) = previous_config {
                provider.add_custom_model(request.id.clone(), previous);
            } else {
                provider.remove_custom_model(&request.id);
            }
        }
        return Err(e);
    }

    invalidate_caps(gcx).await;

    json_response(
        StatusCode::OK,
        &json!({"success": true, "model_id": request.id}),
    )
}

#[derive(Deserialize)]
pub struct RemoveCustomModelRequest {
    pub model_id: String,
}

/// POST /v1/providers/{name}/custom-models/remove
/// Remove a custom model from a provider (preferred over DELETE with body)
/// Body: { "model_id": "my-custom-model" }
pub async fn handle_v1_provider_remove_custom_model_post(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    handle_v1_provider_remove_custom_model_impl(gcx, &params.name, body_bytes).await
}

/// DELETE /v1/providers/{name}/custom-models
/// Remove a custom model from a provider (kept for backward compatibility)
/// Note: Some proxies may strip DELETE request bodies. Prefer POST /custom-models/remove.
/// Body: { "model_id": "my-custom-model" }
pub async fn handle_v1_provider_remove_custom_model(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    handle_v1_provider_remove_custom_model_impl(gcx, &params.name, body_bytes).await
}

async fn handle_v1_provider_remove_custom_model_impl(
    gcx: Arc<ARwLock<GlobalContext>>,
    provider_name: &str,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: RemoveCustomModelRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    if request.model_id.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Model ID cannot be empty".to_string(),
        ));
    }

    let (config_dir, previous_custom_models, previous_enabled_models, previous_disabled_models) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let provider = registry.get_mut(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", provider_name),
            )
        })?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", provider_name),
            ));
        }

        let previous = provider.custom_models().clone();
        let previous_enabled = provider.enabled_models().to_vec();
        let previous_disabled = provider.disabled_models().to_vec();

        if !provider.remove_custom_model(&request.model_id) {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Custom model '{}' not found", request.model_id),
            ));
        }
        provider.set_model_enabled(&request.model_id, false);

        (
            gcx_locked.config_dir.clone(),
            previous,
            previous_enabled,
            previous_disabled,
        )
    };

    // Try to save updated config, rollback on failure
    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        // Rollback in-memory state
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(provider_name) {
            if let Some(config) = previous_custom_models.get(&request.model_id) {
                provider.add_custom_model(request.model_id.clone(), config.clone());
            }
            for model in provider.enabled_models().to_vec() {
                provider.set_model_enabled(&model, false);
            }
            for model in provider.disabled_models().to_vec() {
                provider.set_model_enabled(&model, true);
            }
            for model in &previous_enabled_models {
                provider.set_model_enabled(model, true);
            }
            for model in &previous_disabled_models {
                provider.set_model_enabled(model, false);
            }
        }
        return Err(e);
    }

    invalidate_caps(gcx).await;

    json_response(
        StatusCode::OK,
        &json!({"success": true, "model_id": request.model_id}),
    )
}

/// Merge new settings with existing config, preserving secret fields when value is "***"
const DERIVED_SETTINGS_KEYS: &[&str] = &[
    "auth_status",
    "auth_source",
    "oauth_connected",
    "cli_refresh_managed",
    "api_key_ready",
    "claude_cli_path",
    "readonly",
];

fn strip_derived_fields(value: serde_yaml::Value) -> serde_yaml::Value {
    if let serde_yaml::Value::Mapping(mut map) = value {
        for key in DERIVED_SETTINGS_KEYS {
            map.remove(serde_yaml::Value::String(key.to_string()));
        }
        serde_yaml::Value::Mapping(map)
    } else {
        value
    }
}

async fn merge_provider_settings_preserving_secrets(
    config_dir: &std::path::Path,
    provider_name: &str,
    new_settings: serde_yaml::Value,
) -> Result<serde_yaml::Value, ScratchError> {
    let config_path = config_dir
        .join("providers.d")
        .join(format!("{}.yaml", provider_name));

    // If no existing config, just return new settings (but strip "***" values)
    if !config_path.exists() {
        return Ok(strip_masked_secrets(new_settings));
    }

    let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read config: {}", e),
        )
    })?;

    let existing: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
        ScratchError::new(
            StatusCode::CONFLICT,
            format!(
                "Existing config is invalid YAML: {}. Fix manually or delete the file.",
                e
            ),
        )
    })?;

    Ok(merge_yaml_preserving_secrets(existing, new_settings))
}

/// Recursively merge YAML, preserving existing values when new value is "***"
fn merge_yaml_preserving_secrets(
    existing: serde_yaml::Value,
    new: serde_yaml::Value,
) -> serde_yaml::Value {
    use serde_yaml::Value;

    match (existing, new) {
        (Value::Mapping(mut existing_map), Value::Mapping(new_map)) => {
            for (key, new_value) in new_map {
                let merged_value = if let Some(existing_value) = existing_map.remove(&key) {
                    merge_yaml_preserving_secrets(existing_value, new_value)
                } else {
                    strip_masked_secrets(new_value)
                };
                existing_map.insert(key, merged_value);
            }
            Value::Mapping(existing_map)
        }
        (existing, Value::String(s)) if s == "***" => {
            // Keep existing value when new is "***"
            existing
        }
        (_, new) => strip_masked_secrets(new),
    }
}

/// Remove "***" values from a YAML tree (for new configs without existing values)
fn strip_masked_secrets(value: serde_yaml::Value) -> serde_yaml::Value {
    use serde_yaml::Value;

    match value {
        Value::String(s) if s == "***" => Value::String(String::new()),
        Value::Mapping(map) => {
            let filtered: serde_yaml::Mapping = map
                .into_iter()
                .map(|(k, v)| (k, strip_masked_secrets(v)))
                .collect();
            Value::Mapping(filtered)
        }
        Value::Sequence(seq) => {
            Value::Sequence(seq.into_iter().map(strip_masked_secrets).collect())
        }
        other => other,
    }
}

/// Helper function to patch provider config - only updates enabled_models/disabled_models and custom_models
/// while preserving secrets and other fields.
///
/// SAFETY: This function will NOT write if the existing config is invalid YAML,
/// to prevent destroying secrets and other settings.
async fn patch_provider_model_config(
    gcx: Arc<ARwLock<GlobalContext>>,
    config_dir: &std::path::Path,
    provider_name: &str,
) -> Result<(), ScratchError> {
    let (enabled_models, disabled_models, custom_models, selected_providers) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry.get(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", provider_name),
            )
        })?;
        (
            provider.enabled_models().to_vec(),
            provider.disabled_models().to_vec(),
            provider.custom_models().clone(),
            provider.selected_providers().clone(),
        )
    };

    let providers_dir = config_dir.join("providers.d");
    let config_path = providers_dir.join(format!("{}.yaml", provider_name));

    // Load existing YAML - DO NOT use unwrap_or_default() to avoid destroying config on parse error
    let mut yaml_map: serde_yaml::Mapping = if config_path.exists() {
        let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read config: {}", e),
            )
        })?;

        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| ScratchError::new(
                StatusCode::CONFLICT,
                format!("Config file is invalid YAML and cannot be safely patched: {}. Please fix the file manually.", e)
            ))?;

        value.as_mapping().cloned().ok_or_else(|| {
            ScratchError::new(
                StatusCode::CONFLICT,
                "Config file root is not a YAML mapping. Cannot safely patch.".to_string(),
            )
        })?
    } else {
        serde_yaml::Mapping::new()
    };

    // Update only the model-related fields, preserving everything else (including secrets)
    // Always persist enabled_models (even empty) so clearing all models is reflected on reload
    yaml_map.insert(
        serde_yaml::Value::String("enabled_models".to_string()),
        serde_yaml::to_value(&enabled_models).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to serialize enabled_models: {}", e),
            )
        })?,
    );
    if !enabled_models.is_empty() {
        yaml_map.insert(
            serde_yaml::Value::String("enabled".to_string()),
            serde_yaml::Value::Bool(true),
        );
    }
    // Always persist disabled_models for denylist providers
    if !disabled_models.is_empty() {
        yaml_map.insert(
            serde_yaml::Value::String("disabled_models".to_string()),
            serde_yaml::to_value(&disabled_models).map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize disabled_models: {}", e),
                )
            })?,
        );
    } else {
        yaml_map.remove(serde_yaml::Value::String("disabled_models".to_string()));
    }
    yaml_map.insert(
        serde_yaml::Value::String("custom_models".to_string()),
        serde_yaml::to_value(&custom_models).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to serialize custom_models: {}", e),
            )
        })?,
    );
    if selected_providers.is_empty() {
        yaml_map.remove(serde_yaml::Value::String("selected_providers".to_string()));
    } else {
        yaml_map.insert(
            serde_yaml::Value::String("selected_providers".to_string()),
            serde_yaml::to_value(&selected_providers).map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize selected_providers: {}", e),
                )
            })?,
        );
    }

    // Ensure directory exists
    tokio::fs::create_dir_all(&providers_dir)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create providers.d: {}", e),
            )
        })?;

    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path =
        config_path.with_extension(format!("yaml.tmp.{}.{}", std::process::id(), unique_id));
    let content = serde_yaml::to_string(&yaml_map).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize config: {}", e),
        )
    })?;

    tokio::fs::write(&temp_path, &content).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write temp config: {}", e),
        )
    })?;

    tokio::fs::rename(&temp_path, &config_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to rename config: {}", e),
            )
        })?;

    Ok(())
}

async fn reload_provider_from_disk(
    gcx: Arc<ARwLock<GlobalContext>>,
    provider_name: &str,
    config_dir: &std::path::Path,
) -> Result<(), ScratchError> {
    let provider_path = config_dir
        .join("providers.d")
        .join(format!("{}.yaml", provider_name));
    if !provider_path.exists() {
        return Ok(());
    }

    let content = tokio::fs::read_to_string(&provider_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to reload config: {}", e),
            )
        })?;

    let yaml: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid YAML after save: {}", e),
        )
    })?;

    let gcx_locked = gcx.read().await;
    let mut registry = gcx_locked.providers.write().await;

    if let Some(existing) = registry.get_mut(provider_name) {
        existing.provider_settings_apply(yaml).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to apply settings: {}", e),
            )
        })?;
    } else {
        let mut provider = create_provider(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create provider".to_string(),
            )
        })?;
        provider.provider_settings_apply(yaml).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to apply settings: {}", e),
            )
        })?;
        registry.add(provider);
    }

    Ok(())
}

#[derive(Deserialize, Default)]
struct GitHubCopilotOAuthStartRequest {
    #[serde(default, alias = "enterpriseUrl")]
    enterprise_url: Option<String>,
    #[serde(default, alias = "deploymentType")]
    deployment_type: Option<String>,
}

pub async fn handle_v1_provider_oauth_start(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    match params.name.as_str() {
        "claude_code" => {
            let mode = crate::providers::claude_code_oauth::OAuthMode::Max;
            let (session_id, authorize_url) =
                crate::providers::claude_code_oauth::start_oauth_session(mode).await;
            json_response(
                StatusCode::OK,
                &json!({
                    "session_id": session_id,
                    "authorize_url": authorize_url,
                }),
            )
        }
        "openai_codex" => {
            let fallback_port = gcx.read().await.cmdline.http_port;
            let (session_id, authorize_url, callback_port) =
                crate::providers::openai_codex_oauth::start_oauth_session(fallback_port).await;

            // If callback port differs from our main HTTP port, start a dedicated listener
            if callback_port != fallback_port {
                let http_client = gcx.read().await.http_client.clone();
                match crate::providers::openai_codex_oauth::start_callback_listener(
                    callback_port,
                    http_client,
                )
                .await
                {
                    Ok(listener_handle) => {
                        let gcx_clone = gcx.clone();
                        tokio::spawn(async move {
                            if let Some(tokens) = listener_handle.await.ok().flatten() {
                                let config_dir = gcx_clone.read().await.config_dir.clone();
                                if let Ok(tokens_value) = serde_yaml::to_value(&tokens) {
                                    if let Err(e) = save_provider_oauth_tokens(
                                        &gcx_clone,
                                        &config_dir,
                                        "openai_codex",
                                        &tokens_value,
                                    )
                                    .await
                                    {
                                        tracing::warn!("OpenAI Codex: failed to save OAuth tokens from callback listener: {:?}", e);
                                    } else {
                                        tracing::info!("OpenAI Codex: OAuth tokens saved successfully from callback listener");
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("OpenAI Codex: failed to start callback listener: {}", e);
                    }
                }
            }

            json_response(
                StatusCode::OK,
                &json!({
                    "session_id": session_id,
                    "authorize_url": authorize_url,
                }),
            )
        }
        "github_copilot" => {
            let request = if body_bytes.is_empty() {
                GitHubCopilotOAuthStartRequest::default()
            } else {
                serde_json::from_slice::<GitHubCopilotOAuthStartRequest>(&body_bytes).map_err(
                    |e| {
                        ScratchError::new(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            format!("Invalid JSON: {e}"),
                        )
                    },
                )?
            };
            if request
                .deployment_type
                .as_deref()
                .is_some_and(|value| value == "enterprise")
                && request
                    .enterprise_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "GitHub Enterprise login requires enterprise_url".to_string(),
                ));
            }
            let http_client = gcx.read().await.http_client.clone();
            let start = crate::providers::github_copilot_oauth::start_oauth_session(
                &http_client,
                request.enterprise_url.as_deref(),
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
            json_response(StatusCode::OK, &start)
        }
        _ => Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("OAuth not supported for provider '{}'", params.name),
        )),
    }
}

#[derive(Deserialize)]
pub struct OAuthExchangeRequest {
    pub session_id: String,
    #[serde(default)]
    pub code: String,
}

pub async fn handle_v1_provider_oauth_exchange(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: OAuthExchangeRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    let http_client = gcx.read().await.http_client.clone();
    let config_dir = gcx.read().await.config_dir.clone();

    match params.name.as_str() {
        "claude_code" => {
            let tokens = crate::providers::claude_code_oauth::exchange_code(
                &http_client,
                &request.session_id,
                &request.code,
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

            save_provider_oauth_tokens(
                &gcx,
                &config_dir,
                "claude_code",
                &serde_yaml::to_value(&tokens).map_err(|e| {
                    ScratchError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to serialize tokens: {}", e),
                    )
                })?,
            )
            .await?;
        }
        "openai_codex" => {
            let tokens = crate::providers::openai_codex_oauth::exchange_code(
                &http_client,
                &request.session_id,
                &request.code,
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

            save_provider_oauth_tokens(
                &gcx,
                &config_dir,
                "openai_codex",
                &serde_yaml::to_value(&tokens).map_err(|e| {
                    ScratchError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to serialize tokens: {}", e),
                    )
                })?,
            )
            .await?;
        }
        "github_copilot" => {
            let outcome = crate::providers::github_copilot_oauth::poll_oauth_session(
                &http_client,
                &request.session_id,
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
            match outcome {
                crate::providers::github_copilot_oauth::DevicePollOutcome::Success(tokens) => {
                    save_provider_oauth_tokens(
                        &gcx,
                        &config_dir,
                        "github_copilot",
                        &serde_yaml::to_value(&tokens).map_err(|e| {
                            ScratchError::new(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Failed to serialize tokens: {}", e),
                            )
                        })?,
                    )
                    .await?;
                }
                crate::providers::github_copilot_oauth::DevicePollOutcome::AuthorizationPending {
                    poll_interval,
                } => {
                    return json_response(
                        StatusCode::OK,
                        &json!({
                            "success": false,
                            "status": "authorization_pending",
                            "poll_interval": poll_interval,
                            "auth_status": "Waiting for GitHub device authorization",
                        }),
                    );
                }
                crate::providers::github_copilot_oauth::DevicePollOutcome::SlowDown {
                    poll_interval,
                } => {
                    return json_response(
                        StatusCode::OK,
                        &json!({
                            "success": false,
                            "status": "slow_down",
                            "poll_interval": poll_interval,
                            "auth_status": "GitHub requested a slower polling interval",
                        }),
                    );
                }
                crate::providers::github_copilot_oauth::DevicePollOutcome::ExpiredToken { message }
                | crate::providers::github_copilot_oauth::DevicePollOutcome::AccessDenied {
                    message,
                }
                | crate::providers::github_copilot_oauth::DevicePollOutcome::Error { message } => {
                    return Err(ScratchError::new(StatusCode::BAD_REQUEST, message));
                }
            }
        }
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("OAuth not supported for provider '{}'", params.name),
            ));
        }
    }

    json_response(
        StatusCode::OK,
        &json!({
            "success": true,
            "auth_status": "OK (OAuth login)",
        }),
    )
}

pub async fn handle_v1_provider_oauth_logout(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let config_dir = gcx.read().await.config_dir.clone();

    match params.name.as_str() {
        "claude_code" => {
            let empty =
                serde_yaml::to_value(&crate::providers::claude_code_oauth::OAuthTokens::default())
                    .map_err(|e| {
                        ScratchError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Failed to serialize: {}", e),
                        )
                    })?;
            save_provider_oauth_tokens(&gcx, &config_dir, "claude_code", &empty).await?;
        }
        "openai_codex" => {
            let empty =
                serde_yaml::to_value(&crate::providers::openai_codex_oauth::OAuthTokens::default())
                    .map_err(|e| {
                        ScratchError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Failed to serialize: {}", e),
                        )
                    })?;
            save_provider_oauth_tokens(&gcx, &config_dir, "openai_codex", &empty).await?;
        }
        "github_copilot" => {
            let empty = serde_yaml::to_value(
                &crate::providers::github_copilot_oauth::OAuthTokens::default(),
            )
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize: {}", e),
                )
            })?;
            save_provider_oauth_tokens(&gcx, &config_dir, "github_copilot", &empty).await?;
        }
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("OAuth not supported for provider '{}'", params.name),
            ));
        }
    }

    json_response(
        StatusCode::OK,
        &json!({
            "success": true,
            "auth_status": "No credentials found",
        }),
    )
}

#[derive(Deserialize)]
pub struct OAuthCallbackParams {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

fn html_response(
    title: &str,
    heading: &str,
    heading_color: &str,
    message: &str,
) -> Result<Response<Body>, ScratchError> {
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><title>{title}</title></head>
<body style="font-family: system-ui; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #1a1a2e; color: #e0e0e0;">
<div style="text-align: center;">
<h1 style="color: {heading_color};">{heading}</h1>
<p>{message}</p>
</div>
</body></html>"#,
        title = html_escape(title),
        heading = html_escape(heading),
        heading_color = heading_color,
        message = html_escape(message),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'",
        )
        .body(Body::from(html))
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response build failed: {}", e),
            )
        })
}

pub async fn handle_v1_provider_oauth_callback(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    Query(query): Query<OAuthCallbackParams>,
) -> Result<Response<Body>, ScratchError> {
    if params.name != "openai_codex" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "OAuth callback not supported for provider '{}'",
                params.name
            ),
        ));
    }

    if let Some(err) = &query.error {
        let desc = query
            .error_description
            .as_deref()
            .unwrap_or("Unknown error");
        tracing::warn!("OpenAI OAuth error: {} — {}", err, desc);
        return html_response(
            "Authentication Failed",
            "✗ Authentication Failed",
            "#ef4444",
            &format!("{}: {}", err, desc),
        );
    }

    let code = match &query.code {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "No authorization code received. Please try again.",
            );
        }
    };

    let session_id = match &query.state {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "Missing state parameter. Please start the OAuth flow again.",
            );
        }
    };

    let http_client = gcx.read().await.http_client.clone();
    let config_dir = gcx.read().await.config_dir.clone();

    let tokens =
        match crate::providers::openai_codex_oauth::exchange_code(&http_client, &session_id, &code)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("OpenAI OAuth exchange failed: {}", e);
                return html_response(
                    "Authentication Failed",
                    "✗ Authentication Failed",
                    "#ef4444",
                    &format!("Token exchange failed: {}", e),
                );
            }
        };

    if let Err(e) = save_provider_oauth_tokens(
        &gcx,
        &config_dir,
        "openai_codex",
        &serde_yaml::to_value(&tokens).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to serialize tokens: {}", e),
            )
        })?,
    )
    .await
    {
        tracing::warn!("Failed to save OAuth tokens: {:?}", e);
        return html_response(
            "Authentication Failed",
            "✗ Authentication Failed",
            "#ef4444",
            "Tokens received but failed to save. Please try again.",
        );
    }

    html_response(
        "Authentication Successful",
        "✓ Authentication Successful",
        "#4ade80",
        "You can close this window and return to the application.",
    )
}

pub async fn handle_openai_codex_auth_callback(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<OAuthCallbackParams>,
) -> Result<Response<Body>, ScratchError> {
    if let Some(err) = &query.error {
        let desc = query
            .error_description
            .as_deref()
            .unwrap_or("Unknown error");
        tracing::warn!("OpenAI OAuth error: {} — {}", err, desc);
        return html_response(
            "Authentication Failed",
            "✗ Authentication Failed",
            "#ef4444",
            &format!("{}: {}", err, desc),
        );
    }

    let code = match &query.code {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "No authorization code received. Please try again.",
            );
        }
    };

    let session_id = match &query.state {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "Missing state parameter. Please start the OAuth flow again.",
            );
        }
    };

    let http_client = gcx.read().await.http_client.clone();
    let config_dir = gcx.read().await.config_dir.clone();

    let tokens =
        match crate::providers::openai_codex_oauth::exchange_code(&http_client, &session_id, &code)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("OpenAI OAuth exchange failed: {}", e);
                return html_response(
                    "Authentication Failed",
                    "✗ Authentication Failed",
                    "#ef4444",
                    &format!("Token exchange failed: {}", e),
                );
            }
        };

    if let Err(e) = save_provider_oauth_tokens(
        &gcx,
        &config_dir,
        "openai_codex",
        &serde_yaml::to_value(&tokens).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to serialize tokens: {}", e),
            )
        })?,
    )
    .await
    {
        tracing::warn!("Failed to save OAuth tokens: {:?}", e);
        return html_response(
            "Authentication Failed",
            "✗ Authentication Failed",
            "#ef4444",
            "Tokens received but failed to save. Please try again.",
        );
    }

    html_response(
        "Authentication Successful",
        "✓ Authentication Successful",
        "#4ade80",
        "You can close this window and return to the application.",
    )
}

/// GET /v1/claude-code/usage
pub async fn handle_v1_claude_code_usage(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry
            .get("claude_code")
            .map(|p| p.clone_box())
            .or_else(|| create_provider("claude_code"))
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    "Claude Code provider is not available".to_string(),
                )
            })?;
        (provider, gcx_locked.http_client.clone())
    };

    let Some(claude_code) = provider.as_any().downcast_ref::<ClaudeCodeProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve Claude Code provider type".to_string(),
        ));
    };

    match claude_code.fetch_usage(&http_client).await {
        Ok(usage) => json_response(StatusCode::OK, &json!({"data": usage})),
        Err(e) => json_response(StatusCode::OK, &json!({"error": e})),
    }
}

/// GET /v1/openai-codex/usage
pub async fn handle_v1_openai_codex_usage(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let result = fetch_openai_codex_usage_with_refresh(gcx).await;

    match result {
        Ok(usage) => json_response(StatusCode::OK, &json!({"data": usage})),
        Err(e) => json_response(StatusCode::OK, &json!({"error": e})),
    }
}

async fn current_openai_codex_provider(
    gcx: &Arc<ARwLock<GlobalContext>>,
) -> Result<(OpenAICodexProvider, reqwest::Client, std::path::PathBuf), String> {
    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;
    let provider = registry
        .get("openai_codex")
        .map(|p| p.clone_box())
        .or_else(|| create_provider("openai_codex"))
        .ok_or_else(|| "OpenAI Codex provider is not available".to_string())?;
    let Some(codex) = provider.as_any().downcast_ref::<OpenAICodexProvider>() else {
        return Err("Failed to resolve OpenAI Codex provider type".to_string());
    };
    Ok((
        codex.clone(),
        gcx_locked.http_client.clone(),
        gcx_locked.config_dir.clone(),
    ))
}

async fn force_refresh_openai_codex_usage_for_retry(
    gcx: Arc<ARwLock<GlobalContext>>,
    http_client: &reqwest::Client,
    rejected_access_token: &str,
    rejected_status: Option<reqwest::StatusCode>,
) -> Result<Option<OpenAICodexProvider>, String> {
    let _guard = OpenAICodexProvider::lock_refresh_guard().await?;
    let (mut provider, _, config_dir) = current_openai_codex_provider(&gcx).await?;

    if provider
        .access_token_changed_since_rejection(rejected_access_token)
        .is_some()
    {
        return Ok(Some(provider));
    }

    if let Some(status) = rejected_status {
        if !OpenAICodexProvider::should_force_refresh_for_status(
            status,
            &provider.oauth_tokens.refresh_token,
            false,
        ) {
            return Ok(None);
        }
    } else if provider.oauth_tokens.refresh_token.is_empty() {
        return Ok(None);
    }

    let previous_tokens = provider.oauth_tokens.clone();
    let previous_session_id = provider.session_id.clone();
    let refresh_result = provider
        .force_refresh_after_auth_rejection(http_client, &config_dir)
        .await;

    if !provider.auth_state_matches(&previous_tokens, &previous_session_id) {
        if sync_openai_codex_auth_state(
            gcx.clone(),
            &provider,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .map_err(|e| e.message)?
        {
            invalidate_caps(gcx.clone()).await;
        }
    }

    refresh_result.map(|access_token| access_token.map(|_| provider))
}

async fn fetch_openai_codex_usage_with_refresh(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<crate::providers::openai_codex::OpenAICodexUsage, String> {
    let (mut request_provider, http_client, _) = current_openai_codex_provider(&gcx).await?;
    if !request_provider.oauth_tokens.has_refresh_token() {
        return request_provider.fetch_usage(&http_client).await;
    }

    let mut refresh_attempted = false;
    let context = match request_provider.resolve_wham_context() {
        Ok(context) => context,
        Err(_) if request_provider.oauth_tokens.has_refresh_token() => {
            refresh_attempted = true;
            let rejected_access_token = request_provider.oauth_tokens.access_token.clone();
            request_provider = force_refresh_openai_codex_usage_for_retry(
                gcx.clone(),
                &http_client,
                &rejected_access_token,
                None,
            )
            .await?
            .ok_or_else(|| {
                "OpenAI Codex usage access token is expired and refresh returned no access token. Log in again in OpenAI Codex provider settings."
                    .to_string()
            })?;
            request_provider.resolve_wham_context()?
        }
        Err(error) => return Err(error),
    };

    match request_provider
        .fetch_usage_once(
            &http_client,
            &context.access_token,
            &context.chatgpt_account_id,
        )
        .await
    {
        Ok(usage) => Ok(usage),
        Err(UsageRequestError::Status(status, _body))
            if OpenAICodexProvider::should_force_refresh_for_status(
                status,
                &request_provider.oauth_tokens.refresh_token,
                refresh_attempted,
            ) =>
        {
            let Some(retry_provider) = force_refresh_openai_codex_usage_for_retry(
                gcx,
                &http_client,
                &context.access_token,
                Some(status),
            )
            .await?
            else {
                return Err(
                    "OpenAI Codex usage API rejected the access token and refresh returned no access token. Log in again in OpenAI Codex provider settings."
                        .to_string(),
                );
            };
            let retry_context = retry_provider.resolve_wham_context()?;
            retry_provider
                .fetch_usage_once(
                    &http_client,
                    &retry_context.access_token,
                    &retry_context.chatgpt_account_id,
                )
                .await
                .map_err(|error| {
                    OpenAICodexProvider::usage_request_error_to_string(error, retry_context.source)
                })
        }
        Err(error) => Err(OpenAICodexProvider::usage_request_error_to_string(
            error,
            context.source,
        )),
    }
}

async fn sync_openai_codex_auth_state(
    gcx: Arc<ARwLock<GlobalContext>>,
    source: &OpenAICodexProvider,
    previous_tokens: &crate::providers::openai_codex_oauth::OAuthTokens,
    previous_session_id: &str,
) -> Result<bool, ScratchError> {
    if source.auth_state_matches(previous_tokens, previous_session_id) {
        return Ok(false);
    }

    let gcx_locked = gcx.read().await;
    let mut registry = gcx_locked.providers.write().await;
    let provider = registry.get_mut("openai_codex").ok_or_else(|| {
        ScratchError::new(
            StatusCode::NOT_FOUND,
            "OpenAI Codex provider is not available".to_string(),
        )
    })?;
    let Some(current) = provider.as_any_mut().downcast_mut::<OpenAICodexProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve OpenAI Codex provider type".to_string(),
        ));
    };
    Ok(current.update_auth_state_from_if_current(source, previous_tokens, previous_session_id))
}

fn ensure_openai_codex_session_id(yaml_map: &mut serde_yaml::Mapping) -> String {
    let key = serde_yaml::Value::String("session_id".to_string());
    if let Some(session_id) = yaml_map
        .get(&key)
        .and_then(|value| value.as_str())
        .filter(|session_id| !session_id.is_empty())
    {
        return session_id.to_string();
    }
    let session_id = uuid::Uuid::new_v4().to_string();
    yaml_map.insert(key, serde_yaml::Value::String(session_id.clone()));
    session_id
}

async fn save_provider_oauth_tokens(
    gcx: &Arc<ARwLock<GlobalContext>>,
    config_dir: &std::path::Path,
    provider_name: &str,
    tokens_value: &serde_yaml::Value,
) -> Result<(), ScratchError> {
    let _openai_codex_refresh_guard = if provider_name == "openai_codex" {
        Some(
            OpenAICodexProvider::lock_refresh_guard()
                .await
                .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?,
        )
    } else {
        None
    };
    let providers_dir = config_dir.join("providers.d");
    let config_path = providers_dir.join(format!("{}.yaml", provider_name));

    tokio::fs::create_dir_all(&providers_dir)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create providers.d: {}", e),
            )
        })?;

    let mut yaml_map: serde_yaml::Mapping = if config_path.exists() {
        let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read config: {}", e),
            )
        })?;
        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| ScratchError::new(
                StatusCode::CONFLICT,
                format!("Config file is invalid YAML and cannot be safely patched: {}. Please fix the file manually.", e),
            ))?;
        value.as_mapping().cloned().ok_or_else(|| {
            ScratchError::new(
                StatusCode::CONFLICT,
                "Config file root is not a YAML mapping. Cannot safely patch.".to_string(),
            )
        })?
    } else {
        serde_yaml::Mapping::new()
    };

    yaml_map.insert(
        serde_yaml::Value::String("oauth_tokens".to_string()),
        tokens_value.clone(),
    );

    if provider_name == "openai_codex" {
        if let Some(api_key) = tokens_value
            .get("openai_api_key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            yaml_map.insert(
                serde_yaml::Value::String("OPENAI_API_KEY".to_string()),
                serde_yaml::Value::String(api_key.to_string()),
            );
        } else {
            yaml_map.remove(serde_yaml::Value::String("OPENAI_API_KEY".to_string()));
        }
        ensure_openai_codex_session_id(&mut yaml_map);
    }

    let content = serde_yaml::to_string(&yaml_map).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize config: {}", e),
        )
    })?;

    use std::sync::atomic::{AtomicU64, Ordering};
    static OAUTH_COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = OAUTH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path =
        config_path.with_extension(format!("yaml.tmp.{}.{}", std::process::id(), unique_id));

    tokio::fs::write(&temp_path, &content).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write temp config: {}", e),
        )
    })?;
    tokio::fs::rename(&temp_path, &config_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to rename config: {}", e),
            )
        })?;

    {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let full_content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to reload config: {}", e),
            )
        })?;
        let yaml: serde_yaml::Value = serde_yaml::from_str(&full_content).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Invalid YAML after save: {}", e),
            )
        })?;

        let mut provider = create_provider(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create provider '{}'", provider_name),
            )
        })?;
        provider.provider_settings_apply(yaml).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to apply settings: {}", e),
            )
        })?;
        registry.add(provider);
    }

    invalidate_caps(gcx.clone()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn openai_codex_oauth_callback_html_escapes_interpolated_values() {
        let response = html_response(
            "<script>title</script>",
            "Heading & \"quoted\"",
            "#ef4444",
            "<script>alert('xss')</script> & \"quote\"",
        )
        .unwrap();
        assert_eq!(
            response
                .headers()
                .get("Content-Type")
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();

        assert!(html.contains("&lt;script&gt;title&lt;/script&gt;"));
        assert!(html.contains("Heading &amp; &quot;quoted&quot;"));
        assert!(html.contains("&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"));
        assert!(html.contains("&amp; &quot;quote&quot;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn openai_codex_session_id_is_created_and_preserved() {
        let mut yaml_map = serde_yaml::Mapping::new();
        let created = ensure_openai_codex_session_id(&mut yaml_map);
        assert!(!created.is_empty());
        assert_eq!(
            yaml_map
                .get(&serde_yaml::Value::String("session_id".to_string()))
                .and_then(|value| value.as_str()),
            Some(created.as_str())
        );

        yaml_map.insert(
            serde_yaml::Value::String("session_id".to_string()),
            serde_yaml::Value::String("existing-session".to_string()),
        );
        let preserved = ensure_openai_codex_session_id(&mut yaml_map);
        assert_eq!(preserved, "existing-session");
    }

    #[tokio::test]
    async fn openai_codex_oauth_logout_removes_top_level_api_key() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openai_codex.yaml"),
            "OPENAI_API_KEY: sk-stale\noauth_tokens:\n  openai_api_key: sk-stale\n  access_token: old\n",
        )
        .await
        .unwrap();
        let empty =
            serde_yaml::to_value(&crate::providers::openai_codex_oauth::OAuthTokens::default())
                .unwrap();

        save_provider_oauth_tokens(&gcx, &config_dir, "openai_codex", &empty)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(providers_dir.join("openai_codex.yaml"))
            .await
            .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert!(yaml.get("OPENAI_API_KEY").is_none());
        assert_eq!(
            yaml.get("oauth_tokens")
                .and_then(|tokens| tokens.get("openai_api_key"))
                .and_then(|value| value.as_str()),
            Some("")
        );
    }

    #[tokio::test]
    async fn openai_codex_oauth_save_syncs_top_level_api_key() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        let tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            openai_api_key: "sk-new".to_string(),
            access_token: "access".to_string(),
            expires_at: i64::MAX,
            ..Default::default()
        };
        let tokens_value = serde_yaml::to_value(&tokens).unwrap();

        save_provider_oauth_tokens(&gcx, &config_dir, "openai_codex", &tokens_value)
            .await
            .unwrap();

        let content =
            tokio::fs::read_to_string(config_dir.join("providers.d").join("openai_codex.yaml"))
                .await
                .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(
            yaml.get("OPENAI_API_KEY").and_then(|value| value.as_str()),
            Some("sk-new")
        );
        assert_eq!(
            yaml.get("oauth_tokens")
                .and_then(|tokens| tokens.get("openai_api_key"))
                .and_then(|value| value.as_str()),
            Some("sk-new")
        );
    }

    #[tokio::test]
    async fn github_copilot_oauth_save_writes_redactable_yaml() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        let tokens = crate::providers::github_copilot_oauth::OAuthTokens {
            access_token: "gho-secret".to_string(),
            expires_at: 0,
            enterprise_url: Some("company.ghe.com".to_string()),
            api_base: Some("https://copilot-api.company.ghe.com".to_string()),
        };
        let tokens_value = serde_yaml::to_value(&tokens).unwrap();

        save_provider_oauth_tokens(&gcx, &config_dir, "github_copilot", &tokens_value)
            .await
            .unwrap();

        let content =
            tokio::fs::read_to_string(config_dir.join("providers.d").join("github_copilot.yaml"))
                .await
                .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(
            yaml.get("oauth_tokens")
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(|value| value.as_str()),
            Some("gho-secret")
        );
        assert_eq!(
            yaml.get("oauth_tokens")
                .and_then(|tokens| tokens.get("api_base"))
                .and_then(|value| value.as_str()),
            Some("https://copilot-api.company.ghe.com")
        );

        let stored = {
            let gcx_locked = gcx.read().await;
            let registry = gcx_locked.providers.read().await;
            registry
                .get("github_copilot")
                .unwrap()
                .provider_settings_as_json()
        };
        assert_eq!(stored["oauth_tokens"]["access_token"], "***");
        assert!(!stored.to_string().contains("gho-secret"));
    }

    #[tokio::test]
    async fn github_copilot_oauth_logout_clears_token_fields() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("github_copilot.yaml"),
            "oauth_tokens:\n  access_token: gho-secret\n  expires_at: 0\n  api_base: https://api.githubcopilot.com\n",
        )
        .await
        .unwrap();
        let empty =
            serde_yaml::to_value(&crate::providers::github_copilot_oauth::OAuthTokens::default())
                .unwrap();

        save_provider_oauth_tokens(&gcx, &config_dir, "github_copilot", &empty)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(providers_dir.join("github_copilot.yaml"))
            .await
            .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(
            yaml.get("oauth_tokens")
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(|value| value.as_str()),
            Some("")
        );
        assert_eq!(
            yaml.get("oauth_tokens")
                .and_then(|tokens| tokens.get("expires_at"))
                .and_then(|value| value.as_i64()),
            Some(0)
        );
    }

    async fn assert_remove_custom_model_clears_enabled_entry(
        provider_name: &'static str,
        provider: Box<dyn crate::providers::traits::ProviderTrait>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        {
            let gcx_locked = gcx.read().await;
            let mut registry = gcx_locked.providers.write().await;
            registry.add(provider);
        }

        handle_v1_provider_remove_custom_model_impl(
            gcx.clone(),
            provider_name,
            hyper::body::Bytes::from(r#"{"model_id":"stale-custom"}"#),
        )
        .await
        .unwrap();

        let (enabled_models, has_custom_model) = {
            let gcx_locked = gcx.read().await;
            let registry = gcx_locked.providers.read().await;
            let provider = registry.get(provider_name).unwrap();
            (
                provider.enabled_models().to_vec(),
                provider.custom_models().contains_key("stale-custom"),
            )
        };
        assert!(!enabled_models.iter().any(|model| model == "stale-custom"));
        assert!(!has_custom_model);

        let content = tokio::fs::read_to_string(
            config_dir
                .join("providers.d")
                .join(format!("{provider_name}.yaml")),
        )
        .await
        .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert!(yaml
            .get("enabled_models")
            .and_then(|value| value.as_sequence())
            .map(|models| models.is_empty())
            .unwrap_or(false));
        assert!(yaml
            .get("custom_models")
            .and_then(|value| value.as_mapping())
            .map(|models| models.is_empty())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn custom_provider_remove_custom_model_clears_stale_enabled_entry() {
        let provider = crate::providers::custom::CustomProvider {
            chat_endpoint: "https://example.com/v1/chat/completions".to_string(),
            enabled: true,
            enabled_models: vec!["stale-custom".to_string()],
            custom_models: HashMap::from([(
                "stale-custom".to_string(),
                CustomModelConfig::default(),
            )]),
            ..Default::default()
        };

        assert_remove_custom_model_clears_enabled_entry("custom", Box::new(provider)).await;
    }

    #[tokio::test]
    async fn doubao_remove_custom_model_clears_stale_enabled_entry() {
        let provider = crate::providers::doubao::DoubaoProvider {
            api_key: "sk-test".to_string(),
            enabled: true,
            enabled_models: vec!["stale-custom".to_string()],
            custom_models: HashMap::from([(
                "stale-custom".to_string(),
                CustomModelConfig::default(),
            )]),
            ..Default::default()
        };

        assert_remove_custom_model_clears_enabled_entry("doubao", Box::new(provider)).await;
    }

    #[tokio::test]
    async fn openai_codex_usage_no_token_change_does_not_replace_registry_provider() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut current = OpenAICodexProvider::default();
        current.enabled_models = vec!["keep-enabled".to_string()];
        current.custom_models.insert(
            "keep-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                ..Default::default()
            },
        );
        current.oauth_tokens.access_token = "same-access".to_string();
        current.session_id = "same-session".to_string();
        let previous_tokens = current.oauth_tokens.clone();
        let previous_session_id = current.session_id.clone();
        {
            let gcx_locked = gcx.read().await;
            let mut registry = gcx_locked.providers.write().await;
            registry.add(Box::new(current.clone()));
        }
        let mut source = OpenAICodexProvider::default();
        source.enabled_models = vec!["clobber-enabled".to_string()];
        source.oauth_tokens = previous_tokens.clone();
        source.session_id = previous_session_id.clone();

        let changed = sync_openai_codex_auth_state(
            gcx.clone(),
            &source,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .unwrap();

        let stored = {
            let gcx_locked = gcx.read().await;
            let registry = gcx_locked.providers.read().await;
            registry
                .get("openai_codex")
                .unwrap()
                .as_any()
                .downcast_ref::<OpenAICodexProvider>()
                .unwrap()
                .clone()
        };
        assert!(!changed);
        assert_eq!(stored.enabled_models, vec!["keep-enabled".to_string()]);
        assert!(stored.custom_models.contains_key("keep-custom"));
    }

    #[tokio::test]
    async fn openai_codex_stale_auth_sync_does_not_overwrite_newer_registry_token() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut current = OpenAICodexProvider::default();
        current.oauth_tokens.access_token = "newer-access".to_string();
        current.oauth_tokens.refresh_token = "newer-refresh".to_string();
        current.session_id = "newer-session".to_string();
        let previous_tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            ..Default::default()
        };
        let previous_session_id = "old-session".to_string();
        {
            let gcx_locked = gcx.read().await;
            let mut registry = gcx_locked.providers.write().await;
            registry.add(Box::new(current));
        }
        let mut source = OpenAICodexProvider::default();
        source.oauth_tokens.access_token = "stale-refresh-access".to_string();
        source.oauth_tokens.refresh_token = "stale-refresh".to_string();
        source.session_id = "stale-refresh-session".to_string();

        let changed = sync_openai_codex_auth_state(
            gcx.clone(),
            &source,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .unwrap();

        let stored = {
            let gcx_locked = gcx.read().await;
            let registry = gcx_locked.providers.read().await;
            registry
                .get("openai_codex")
                .unwrap()
                .as_any()
                .downcast_ref::<OpenAICodexProvider>()
                .unwrap()
                .clone()
        };
        assert!(!changed);
        assert_eq!(stored.oauth_tokens.access_token, "newer-access");
        assert_eq!(stored.oauth_tokens.refresh_token, "newer-refresh");
        assert_eq!(stored.session_id, "newer-session");
    }

    #[tokio::test]
    async fn openai_codex_usage_refresh_rereads_registry_and_skips_stale_token() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut current = OpenAICodexProvider::default();
        current.oauth_tokens.access_token = "fresh-access".to_string();
        current.oauth_tokens.refresh_token = "refresh".to_string();
        current.oauth_tokens.expires_at = i64::MAX;
        {
            let gcx_locked = gcx.read().await;
            let mut registry = gcx_locked.providers.write().await;
            registry.add(Box::new(current));
        }
        let http_client = gcx.read().await.http_client.clone();

        let refreshed = force_refresh_openai_codex_usage_for_retry(
            gcx,
            &http_client,
            "stale-access",
            Some(reqwest::StatusCode::UNAUTHORIZED),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(refreshed.oauth_tokens.access_token, "fresh-access");
    }

    #[tokio::test]
    async fn openai_codex_token_refresh_preserves_concurrent_model_settings() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut current = OpenAICodexProvider::default();
        current.enabled_models = vec!["keep-enabled".to_string()];
        current.custom_models.insert(
            "keep-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                ..Default::default()
            },
        );
        current.oauth_tokens.access_token = "old-access".to_string();
        current.session_id = "old-session".to_string();
        let previous_tokens = current.oauth_tokens.clone();
        let previous_session_id = current.session_id.clone();
        {
            let gcx_locked = gcx.read().await;
            let mut registry = gcx_locked.providers.write().await;
            registry.add(Box::new(current));
        }
        let mut source = OpenAICodexProvider::default();
        source.enabled_models = vec!["clobber-enabled".to_string()];
        source.oauth_tokens.access_token = "new-access".to_string();
        source.oauth_tokens.refresh_token = "new-refresh".to_string();
        source.oauth_tokens.expires_at = 42;
        source.session_id = "new-session".to_string();

        let changed = sync_openai_codex_auth_state(
            gcx.clone(),
            &source,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .unwrap();

        let stored = {
            let gcx_locked = gcx.read().await;
            let registry = gcx_locked.providers.read().await;
            registry
                .get("openai_codex")
                .unwrap()
                .as_any()
                .downcast_ref::<OpenAICodexProvider>()
                .unwrap()
                .clone()
        };
        assert!(changed);
        assert_eq!(stored.oauth_tokens.access_token, "new-access");
        assert_eq!(stored.oauth_tokens.refresh_token, "new-refresh");
        assert_eq!(stored.session_id, "new-session");
        assert_eq!(stored.enabled_models, vec!["keep-enabled".to_string()]);
        assert!(stored.custom_models.contains_key("keep-custom"));
    }
}
