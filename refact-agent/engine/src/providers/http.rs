use axum::extract::{Path, Query};
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
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
use super::openrouter::OpenRouterProvider;
use super::google_gemini::GoogleGeminiProvider;
use super::claude_code::ClaudeCodeProvider;
use super::openai_codex::OpenAICodexProvider;

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

    let settings = strip_derived_fields(settings);
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
    let (model_caps, caps_error) = match get_model_caps(gcx.clone(), &address_url, false).await {
        Ok(caps) => (caps, None),
        Err(e) => {
            tracing::warn!("Failed to fetch model_caps for provider '{}': {}", params.name, e);
            (HashMap::new(), Some(format!("Failed to fetch model capabilities: {}. Model limits may be inaccurate.", e)))
        }
    };
    let models = provider.fetch_available_models(&http_client, &model_caps).await;
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
            let default_provider = create_provider(provider_name)
                .ok_or_else(|| ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Unknown provider type '{}'", provider_name),
                ))?;
            registry.add(default_provider);
        }

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
        let previous_enabled = provider.enabled_models().to_vec();
        let previous_disabled = provider.disabled_models().to_vec();

        provider.set_model_enabled(model_id, enabled);
        (gcx_locked.config_dir.clone(), previous_enabled, previous_disabled)
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

    json_response(StatusCode::OK, &json!({"success": true, "model_id": model_id, "enabled": enabled}))
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
            let default_provider = create_provider(provider_name)
                .ok_or_else(|| ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Unknown provider type '{}'", provider_name),
                ))?;
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

    let (config_dir, had_existing) = {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        if registry.get(&params.name).is_none() {
            let default_provider = create_provider(&params.name)
                .ok_or_else(|| ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Unknown provider type '{}'", params.name),
                ))?;
            registry.add(default_provider);
        }

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
const DERIVED_SETTINGS_KEYS: &[&str] = &[
    "auth_status", "oauth_connected", "claude_cli_path", "readonly",
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
    let (enabled_models, disabled_models, custom_models, selected_providers) = {
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
            provider.selected_providers().clone(),
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
    // Always persist enabled_models (even empty) so clearing all models is reflected on reload
    yaml_map.insert(
        serde_yaml::Value::String("enabled_models".to_string()),
        serde_yaml::to_value(&enabled_models)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize enabled_models: {}", e)))?,
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
            serde_yaml::to_value(&disabled_models)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize disabled_models: {}", e)))?,
        );
    } else {
        yaml_map.remove(serde_yaml::Value::String("disabled_models".to_string()));
    }
    yaml_map.insert(
        serde_yaml::Value::String("custom_models".to_string()),
        serde_yaml::to_value(&custom_models)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize custom_models: {}", e)))?,
    );
    if selected_providers.is_empty() {
        yaml_map.remove(serde_yaml::Value::String("selected_providers".to_string()));
    } else {
        yaml_map.insert(
            serde_yaml::Value::String("selected_providers".to_string()),
            serde_yaml::to_value(&selected_providers)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize selected_providers: {}", e)))?,
        );
    }

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

async fn reload_provider_from_disk(
    gcx: Arc<ARwLock<GlobalContext>>,
    provider_name: &str,
    config_dir: &std::path::Path,
) -> Result<(), ScratchError> {
    let provider_path = config_dir.join("providers.d").join(format!("{}.yaml", provider_name));
    if !provider_path.exists() {
        return Ok(());
    }

    let content = tokio::fs::read_to_string(&provider_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reload config: {}", e)))?;

    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid YAML after save: {}", e)))?;

    let gcx_locked = gcx.read().await;
    let mut registry = gcx_locked.providers.write().await;

    if let Some(existing) = registry.get_mut(provider_name) {
        existing
            .provider_settings_apply(yaml)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to apply settings: {}", e)))?;
    } else {
        let mut provider = create_provider(provider_name)
            .ok_or_else(|| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create provider".to_string()))?;
        provider
            .provider_settings_apply(yaml)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to apply settings: {}", e)))?;
        registry.add(provider);
    }

    Ok(())
}

pub async fn handle_v1_provider_oauth_start(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    _body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    match params.name.as_str() {
        "claude_code" => {
            let mode = crate::providers::claude_code_oauth::OAuthMode::Max;
            let (session_id, authorize_url) = crate::providers::claude_code_oauth::start_oauth_session(mode).await;
            json_response(StatusCode::OK, &json!({
                "session_id": session_id,
                "authorize_url": authorize_url,
            }))
        }
        "openai_codex" => {
            let fallback_port = gcx.read().await.cmdline.http_port;
            let (session_id, authorize_url, callback_port) = crate::providers::openai_codex_oauth::start_oauth_session(fallback_port).await;

            // If callback port differs from our main HTTP port, start a dedicated listener
            if callback_port != fallback_port {
                let http_client = gcx.read().await.http_client.clone();
                match crate::providers::openai_codex_oauth::start_callback_listener(callback_port, http_client).await {
                    Ok(listener_handle) => {
                        let gcx_clone = gcx.clone();
                        tokio::spawn(async move {
                            if let Some(tokens) = listener_handle.await.ok().flatten() {
                                let config_dir = gcx_clone.read().await.config_dir.clone();
                                if let Ok(tokens_value) = serde_yaml::to_value(&tokens) {
                                    if let Err(e) = save_provider_oauth_tokens(
                                        &gcx_clone, &config_dir, "openai_codex", &tokens_value,
                                    ).await {
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

            json_response(StatusCode::OK, &json!({
                "session_id": session_id,
                "authorize_url": authorize_url,
            }))
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
    pub code: String,
}

pub async fn handle_v1_provider_oauth_exchange(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let request: OAuthExchangeRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Invalid JSON: {}", e))
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

            save_provider_oauth_tokens(&gcx, &config_dir, "claude_code", &serde_yaml::to_value(&tokens)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize tokens: {}", e)))?
            ).await?;
        }
        "openai_codex" => {
            let tokens = crate::providers::openai_codex_oauth::exchange_code(
                &http_client,
                &request.session_id,
                &request.code,
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

            save_provider_oauth_tokens(&gcx, &config_dir, "openai_codex", &serde_yaml::to_value(&tokens)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize tokens: {}", e)))?
            ).await?;
        }
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("OAuth not supported for provider '{}'", params.name),
            ));
        }
    }

    json_response(StatusCode::OK, &json!({
        "success": true,
        "auth_status": "OK (OAuth login)",
    }))
}

pub async fn handle_v1_provider_oauth_logout(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let config_dir = gcx.read().await.config_dir.clone();

    match params.name.as_str() {
        "claude_code" => {
            let empty = serde_yaml::to_value(&crate::providers::claude_code_oauth::OAuthTokens::default())
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize: {}", e)))?;
            save_provider_oauth_tokens(&gcx, &config_dir, "claude_code", &empty).await?;
        }
        "openai_codex" => {
            let empty = serde_yaml::to_value(&crate::providers::openai_codex_oauth::OAuthTokens::default())
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize: {}", e)))?;
            save_provider_oauth_tokens(&gcx, &config_dir, "openai_codex", &empty).await?;
        }
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("OAuth not supported for provider '{}'", params.name),
            ));
        }
    }

    json_response(StatusCode::OK, &json!({
        "success": true,
        "auth_status": "No credentials found",
    }))
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

fn html_response(title: &str, heading: &str, heading_color: &str, message: &str) -> Result<Response<Body>, ScratchError> {
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><title>{title}</title></head>
<body style="font-family: system-ui; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #1a1a2e; color: #e0e0e0;">
<div style="text-align: center;">
<h1 style="color: {heading_color};">{heading}</h1>
<p>{message}</p>
</div>
</body></html>"#
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html")
        .body(Body::from(html))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Response build failed: {}", e)))
}

pub async fn handle_v1_provider_oauth_callback(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(params): Path<ProviderPathParams>,
    Query(query): Query<OAuthCallbackParams>,
) -> Result<Response<Body>, ScratchError> {
    if params.name != "openai_codex" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("OAuth callback not supported for provider '{}'", params.name),
        ));
    }

    if let Some(err) = &query.error {
        let desc = query.error_description.as_deref().unwrap_or("Unknown error");
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

    let tokens = match crate::providers::openai_codex_oauth::exchange_code(
        &http_client,
        &session_id,
        &code,
    ).await {
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
        &gcx, &config_dir, "openai_codex",
        &serde_yaml::to_value(&tokens)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize tokens: {}", e)))?,
    ).await {
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
        let desc = query.error_description.as_deref().unwrap_or("Unknown error");
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

    let tokens = match crate::providers::openai_codex_oauth::exchange_code(
        &http_client,
        &session_id,
        &code,
    ).await {
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
        &gcx, &config_dir, "openai_codex",
        &serde_yaml::to_value(&tokens)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize tokens: {}", e)))?,
    ).await {
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
        Err(e) => json_response(
            StatusCode::OK,
            &json!({"error": e}),
        ),
    }
}

/// GET /v1/openai-codex/usage
pub async fn handle_v1_openai_codex_usage(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = registry
            .get("openai_codex")
            .map(|p| p.clone_box())
            .or_else(|| create_provider("openai_codex"))
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    "OpenAI Codex provider is not available".to_string(),
                )
            })?;
        (provider, gcx_locked.http_client.clone())
    };

    let Some(codex) = provider.as_any().downcast_ref::<OpenAICodexProvider>() else {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve OpenAI Codex provider type".to_string(),
        ));
    };

    match codex.fetch_usage(&http_client).await {
        Ok(usage) => json_response(StatusCode::OK, &json!({"data": usage})),
        Err(e) => json_response(
            StatusCode::OK,
            &json!({"error": e}),
        ),
    }
}

async fn save_provider_oauth_tokens(
    gcx: &Arc<ARwLock<GlobalContext>>,
    config_dir: &std::path::Path,
    provider_name: &str,
    tokens_value: &serde_yaml::Value,
) -> Result<(), ScratchError> {
    let providers_dir = config_dir.join("providers.d");
    let config_path = providers_dir.join(format!("{}.yaml", provider_name));

    tokio::fs::create_dir_all(&providers_dir).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create providers.d: {}", e)))?;

    let mut yaml_map: serde_yaml::Mapping = if config_path.exists() {
        let content = tokio::fs::read_to_string(&config_path).await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read config: {}", e)))?;
        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| ScratchError::new(
                StatusCode::CONFLICT,
                format!("Config file is invalid YAML and cannot be safely patched: {}. Please fix the file manually.", e),
            ))?;
        value.as_mapping().cloned().ok_or_else(|| ScratchError::new(
            StatusCode::CONFLICT,
            "Config file root is not a YAML mapping. Cannot safely patch.".to_string(),
        ))?
    } else {
        serde_yaml::Mapping::new()
    };

    yaml_map.insert(
        serde_yaml::Value::String("oauth_tokens".to_string()),
        tokens_value.clone(),
    );

    // Backward/compat + UX: expose API key (if present) at the top-level as well.
    // Our OpenAI Codex provider expects an API key (OPENAI_API_KEY) for api.openai.com.
    if let Some(api_key) = tokens_value
        .get("openai_api_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        yaml_map.insert(
            serde_yaml::Value::String("OPENAI_API_KEY".to_string()),
            serde_yaml::Value::String(api_key.to_string()),
        );
    }

    let content = serde_yaml::to_string(&yaml_map)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize config: {}", e)))?;

    use std::sync::atomic::{AtomicU64, Ordering};
    static OAUTH_COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = OAUTH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = config_path.with_extension(format!("yaml.tmp.{}.{}", std::process::id(), unique_id));

    tokio::fs::write(&temp_path, &content).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write temp config: {}", e)))?;
    tokio::fs::rename(&temp_path, &config_path).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to rename config: {}", e)))?;

    {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let full_content = tokio::fs::read_to_string(&config_path).await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reload config: {}", e)))?;
        let yaml: serde_yaml::Value = serde_yaml::from_str(&full_content)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid YAML after save: {}", e)))?;

        let mut provider = create_provider(provider_name)
            .ok_or_else(|| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create provider '{}'", provider_name)))?;
        provider.provider_settings_apply(yaml)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to apply settings: {}", e)))?;
        registry.add(provider);
    }

    invalidate_caps(gcx.clone()).await;
    Ok(())
}

