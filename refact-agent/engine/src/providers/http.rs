use axum::extract::{Path, Query};
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::caps::model_caps::get_model_caps;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;


fn json_response(status: StatusCode, body: &impl Serialize) -> Result<Response<Body>, ScratchError> {
    let json = serde_json::to_string(body)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON serialization failed: {}", e)))?;
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Response build failed: {}", e)))
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
use crate::providers::traits::{AvailableModel, CustomModelConfig, ModelSource, ProviderModel, ProviderRuntime};

#[derive(Serialize)]
struct ProviderListItem {
    name: &'static str,
    display_name: &'static str,
    enabled: bool,
    readonly: bool,
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
            providers.push(ProviderListItem {
                name: provider.name(),
                display_name: provider.display_name(),
                enabled,
                readonly,
                model_count: provider.enabled_models().len(),
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

#[derive(Serialize)]
struct ProviderDetailResponse {
    name: String,
    display_name: String,
    enabled: bool,
    readonly: bool,
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
    let response = ProviderDetailResponse {
        name: provider.name().to_string(),
        display_name: provider.display_name().to_string(),
        enabled: runtime.as_ref().map(|r| r.enabled).unwrap_or(false),
        readonly: provider.is_readonly(),
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
    let settings: serde_yaml::Value = if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
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

    // Merge with existing config to preserve secrets
    let merged_settings = merge_provider_settings_preserving_secrets(&config_dir, &params.name, settings).await?;

    save_provider_config(&config_dir, &params.name, merged_settings)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let provider_path = config_dir.join("providers.d").join(format!("{}.yaml", params.name));
        let content = tokio::fs::read_to_string(&provider_path).await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reload config: {}", e)))?;

        let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid YAML after save: {}", e)))?;

        let mut provider = create_provider(&params.name)
            .ok_or_else(|| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create provider".to_string()))?;

        provider.provider_settings_apply(yaml)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to apply settings: {}", e)))?;

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

    let runtime = provider.build_runtime().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

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

pub async fn handle_v1_defaults_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let defaults: ProviderDefaults = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    let config_dir = gcx.read().await.config_dir.clone();
    defaults
        .save(&config_dir)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

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

    let runtime = provider.build_runtime().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    let response = ModelsResponse {
        chat_models: runtime.chat_models.iter().map(SimplifiedModel::from).collect(),
        completion_models: runtime.completion_models.iter().map(SimplifiedModel::from).collect(),
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

/// GET /v1/providers/{name}/available-models
/// Fetches all available models for a provider from model_caps or API
pub async fn handle_v1_provider_available_models(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, address_url, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let address_url = gcx_locked.cmdline.address_url.clone();
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

        (provider, address_url, http_client)
    };

    let source = provider.model_source();
    let (models, error) = match get_model_caps(gcx.clone(), &address_url, false).await {
        Ok(model_caps) => {
            let models = provider.fetch_available_models(&http_client, &model_caps).await;
            (models, None)
        }
        Err(e) => {
            tracing::warn!("Failed to fetch model_caps for provider '{}': {}", params.name, e);
            (
                provider.get_custom_models_only(),
                Some(format!("Failed to fetch model capabilities: {}. Only custom models shown.", e)),
            )
        }
    };

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

async fn update_model_enabled_state(
    gcx: Arc<ARwLock<GlobalContext>>,
    provider_name: &str,
    model_id: &str,
    enabled: bool,
) -> Result<Response<Body>, ScratchError> {
    // Capture previous state for rollback
    let (config_dir, previous_enabled_models) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let provider = registry.get_mut(provider_name)
            .ok_or_else(|| ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", provider_name),
            ))?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", provider_name),
            ));
        }

        // Capture previous state for rollback
        let previous = provider.enabled_models().to_vec();

        provider.set_model_enabled(model_id, enabled);
        (gcx_locked.config_dir.clone(), previous)
    };

    // Try to save updated config
    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        // Rollback in-memory state on persistence failure
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(provider_name) {
            // Restore previous enabled_models
            for model in &previous_enabled_models {
                provider.set_model_enabled(model, true);
            }
            // Disable any that weren't in previous
            let previous_set: std::collections::HashSet<_> = previous_enabled_models.iter().collect();
            for model in provider.enabled_models().to_vec() {
                if !previous_set.contains(&model) {
                    provider.set_model_enabled(&model, false);
                }
            }
        }
        return Err(e);
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true, "model_id": model_id, "enabled": enabled}))
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

    let (config_dir, had_existing) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let provider = registry.get_mut(&params.name)
            .ok_or_else(|| ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", params.name),
            ))?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", params.name),
            ));
        }

        let had_existing = provider.custom_models().contains_key(&request.id);
        provider.add_custom_model(request.id.clone(), request.config.clone());
        (gcx_locked.config_dir.clone(), had_existing)
    };

    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, &params.name).await {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(&params.name) {
            if !had_existing {
                provider.remove_custom_model(&request.id);
            }
        }
        return Err(e);
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true, "model_id": request.id}))
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

    // Capture previous state for rollback
    let (config_dir, previous_custom_models) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let provider = registry.get_mut(provider_name)
            .ok_or_else(|| ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", provider_name),
            ))?;

        if provider.is_readonly() {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                format!("Provider '{}' is readonly", provider_name),
            ));
        }

        // Capture previous state for rollback
        let previous = provider.custom_models().clone();

        if !provider.remove_custom_model(&request.model_id) {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Custom model '{}' not found", request.model_id),
            ));
        }

        (gcx_locked.config_dir.clone(), previous)
    };

    // Try to save updated config, rollback on failure
    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        // Rollback in-memory state
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;
        if let Some(provider) = registry.get_mut(provider_name) {
            // Restore the removed model
            if let Some(config) = previous_custom_models.get(&request.model_id) {
                provider.add_custom_model(request.model_id.clone(), config.clone());
            }
        }
        return Err(e);
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true, "model_id": request.model_id}))
}

/// Merge new settings with existing config, preserving secret fields when value is "***"
async fn merge_provider_settings_preserving_secrets(
    config_dir: &std::path::Path,
    provider_name: &str,
    new_settings: serde_yaml::Value,
) -> Result<serde_yaml::Value, ScratchError> {
    let config_path = config_dir.join("providers.d").join(format!("{}.yaml", provider_name));

    // If no existing config, just return new settings (but strip "***" values)
    if !config_path.exists() {
        return Ok(strip_masked_secrets(new_settings));
    }

    let content = tokio::fs::read_to_string(&config_path).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read config: {}", e)))?;

    let existing: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| ScratchError::new(
            StatusCode::CONFLICT,
            format!("Existing config is invalid YAML: {}. Fix manually or delete the file.", e)
        ))?;

    Ok(merge_yaml_preserving_secrets(existing, new_settings))
}

/// Recursively merge YAML, preserving existing values when new value is "***"
fn merge_yaml_preserving_secrets(existing: serde_yaml::Value, new: serde_yaml::Value) -> serde_yaml::Value {
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
            let filtered: serde_yaml::Mapping = map.into_iter()
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
    let (enabled_models, disabled_models, custom_models) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry.get(provider_name)
            .ok_or_else(|| ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", provider_name),
            ))?;
        (
            provider.enabled_models().to_vec(),
            provider.disabled_models().to_vec(),
            provider.custom_models().clone(),
        )
    };

    let providers_dir = config_dir.join("providers.d");
    let config_path = providers_dir.join(format!("{}.yaml", provider_name));

    // Load existing YAML - DO NOT use unwrap_or_default() to avoid destroying config on parse error
    let mut yaml_map: serde_yaml::Mapping = if config_path.exists() {
        let content = tokio::fs::read_to_string(&config_path).await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read config: {}", e)))?;

        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| ScratchError::new(
                StatusCode::CONFLICT,
                format!("Config file is invalid YAML and cannot be safely patched: {}. Please fix the file manually.", e)
            ))?;

        value.as_mapping().cloned().ok_or_else(|| ScratchError::new(
            StatusCode::CONFLICT,
            "Config file root is not a YAML mapping. Cannot safely patch.".to_string()
        ))?
    } else {
        serde_yaml::Mapping::new()
    };

    // Update only the model-related fields, preserving everything else (including secrets)
    // Write enabled_models only if non-empty (allowlist providers)
    if !enabled_models.is_empty() {
        yaml_map.insert(
            serde_yaml::Value::String("enabled_models".to_string()),
            serde_yaml::to_value(&enabled_models)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize enabled_models: {}", e)))?,
        );
    }
    // Write disabled_models only if non-empty (denylist providers like Refact)
    if !disabled_models.is_empty() {
        yaml_map.insert(
            serde_yaml::Value::String("disabled_models".to_string()),
            serde_yaml::to_value(&disabled_models)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize disabled_models: {}", e)))?,
        );
    }
    yaml_map.insert(
        serde_yaml::Value::String("custom_models".to_string()),
        serde_yaml::to_value(&custom_models)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize custom_models: {}", e)))?,
    );

    // Ensure directory exists
    tokio::fs::create_dir_all(&providers_dir).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create providers.d: {}", e)))?;

    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = config_path.with_extension(format!("yaml.tmp.{}.{}", std::process::id(), unique_id));
    let content = serde_yaml::to_string(&yaml_map)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize config: {}", e)))?;

    tokio::fs::write(&temp_path, &content).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write temp config: {}", e)))?;

    tokio::fs::rename(&temp_path, &config_path).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to rename config: {}", e)))?;

    Ok(())
}
