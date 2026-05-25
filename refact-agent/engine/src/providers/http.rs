use axum::extract::{Path, Query, State};
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::app_state::AppState;
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

async fn invalidate_caps(gcx: Arc<GlobalContext>) {
    let caps_state = gcx.caps_state.clone();
    let mut caps_state = caps_state.write().await;
    caps_state.caps = None;
    caps_state.last_attempted_ts = 0;
}
use crate::providers::config::ProviderDefaults;
use crate::providers::config_store;
use refact_providers::identity::{provider_identity_from_yaml, validate_provider_instance_id};
use crate::providers::instance::ProviderInstance;
use crate::providers::{create_provider, delete_provider_config, PROVIDER_NAMES};
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelSource, ProviderModel, ProviderRuntime, ProviderTrait,
    extra_headers_mapping_to_hash_map, parse_extra_headers_value,
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

#[derive(Clone)]
struct HttpProviderIdentity {
    instance_id: String,
    base_provider: String,
    display_name: String,
}

fn validate_instance_id_for_http(instance_id: &str) -> Result<(), ScratchError> {
    validate_provider_instance_id(instance_id)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))
}

fn yaml_string_field(
    value: &serde_yaml::Value,
    key: &str,
    allow_empty: bool,
) -> Result<Option<String>, ScratchError> {
    let Some(field) = value.get(key) else {
        return Ok(None);
    };
    let Some(text) = field.as_str() else {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider field '{key}' must be a string"),
        ));
    };
    let text = text.trim();
    if text.is_empty() {
        if allow_empty {
            return Ok(None);
        }
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider field '{key}' cannot be empty"),
        ));
    }
    Ok(Some(text.to_string()))
}

fn ensure_settings_mapping(
    settings: serde_yaml::Value,
) -> Result<serde_yaml::Mapping, ScratchError> {
    match settings {
        serde_yaml::Value::Mapping(map) => Ok(map),
        _ => Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Provider settings must be a YAML mapping".to_string(),
        )),
    }
}

fn provider_display_name(base_provider: &str) -> Result<String, ScratchError> {
    create_provider(base_provider)
        .map(|provider| provider.display_name().to_string())
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("Unknown base provider '{base_provider}'"),
            )
        })
}

fn oauth_supported_for_base(base_provider: &str) -> bool {
    matches!(
        base_provider,
        "claude_code" | "openai_codex" | "github_copilot"
    )
}

fn openrouter_base_provider_supported(base_provider: &str) -> bool {
    base_provider == "openrouter"
}

fn health_base_provider_supported(base_provider: &str) -> bool {
    matches!(base_provider, "openrouter" | "google_gemini")
}

fn usage_base_provider_supported(base_provider: &str) -> bool {
    matches!(base_provider, "claude_code" | "openai_codex")
}

fn provider_identity_from_existing_config(
    instance_id: &str,
    settings: &serde_yaml::Value,
) -> Result<HttpProviderIdentity, ScratchError> {
    let identity = provider_identity_from_yaml(instance_id, settings)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    let display_name = identity
        .display_name
        .unwrap_or(provider_display_name(&identity.base_provider)?);
    Ok(HttpProviderIdentity {
        instance_id: identity.instance_id,
        base_provider: identity.base_provider,
        display_name,
    })
}

async fn resolve_config_identity(
    config_dir: &std::path::Path,
    instance_id: &str,
) -> Result<HttpProviderIdentity, ScratchError> {
    validate_instance_id_for_http(instance_id)?;
    if let Some(existing_settings) =
        read_existing_provider_settings(config_dir, instance_id).await?
    {
        return provider_identity_from_existing_config(instance_id, &existing_settings);
    }
    if create_provider(instance_id).is_none() {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("Provider '{}' not found or not configured", instance_id),
        ));
    }
    Ok(HttpProviderIdentity {
        instance_id: instance_id.to_string(),
        base_provider: instance_id.to_string(),
        display_name: provider_display_name(instance_id)?,
    })
}

async fn resolve_provider_identity(
    gcx: &Arc<GlobalContext>,
    instance_id: &str,
) -> Result<HttpProviderIdentity, ScratchError> {
    validate_instance_id_for_http(instance_id)?;
    let (config_dir, registry_identity) = {
        let registry = gcx.providers.read().await;
        let registry_identity = registry
            .get(instance_id)
            .map(|provider| HttpProviderIdentity {
                instance_id: provider.name().to_string(),
                base_provider: provider.base_provider_name().to_string(),
                display_name: provider.display_name().to_string(),
            });
        (gcx.config_dir.clone(), registry_identity)
    };
    if let Some(identity) = registry_identity {
        return Ok(identity);
    }
    resolve_config_identity(&config_dir, instance_id).await
}

fn downcast_provider<'a, T: 'static>(
    provider: &'a dyn ProviderTrait,
    type_name: &str,
) -> Result<&'a T, ScratchError> {
    provider.as_any().downcast_ref::<T>().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to resolve {type_name} provider type"),
        )
    })
}

async fn resolve_provider_for_base(
    gcx: &Arc<GlobalContext>,
    provider_name: &str,
    expected_base: &str,
) -> Result<(Box<dyn ProviderTrait>, reqwest::Client), ScratchError> {
    let identity = resolve_provider_identity(gcx, provider_name).await?;
    if identity.base_provider != expected_base {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Provider '{}' uses base_provider '{}' and does not support this route",
                provider_name, identity.base_provider
            ),
        ));
    }
    let (provider, http_client) = {
        let registry = gcx.providers.read().await;
        (
            registry
                .get(provider_name)
                .map(|provider| provider.clone_box()),
            gcx.http_client.clone(),
        )
    };
    let provider = if let Some(provider) = provider {
        provider
    } else {
        let config_dir = gcx.config_dir.clone();
        if provider_file_exists(&config_dir, provider_name) {
            let settings = read_existing_provider_settings(&config_dir, provider_name)
                .await?
                .ok_or_else(|| {
                    ScratchError::new(
                        StatusCode::NOT_FOUND,
                        format!("Provider '{}' not found", provider_name),
                    )
                })?;
            provider_from_yaml(provider_name, settings)?
        } else {
            create_provider(expected_base).ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Provider '{}' not found", provider_name),
                )
            })?
        }
    };
    Ok((provider, http_client))
}

async fn identity_for_model_management(
    config_dir: &std::path::Path,
    provider_name: &str,
) -> Result<HttpProviderIdentity, ScratchError> {
    validate_instance_id_for_http(provider_name)?;
    if let Some(existing_settings) =
        read_existing_provider_settings(config_dir, provider_name).await?
    {
        return provider_identity_from_existing_config(provider_name, &existing_settings);
    }
    if create_provider(provider_name).is_none() {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("Provider '{}' not found or not configured", provider_name),
        ));
    }
    Ok(HttpProviderIdentity {
        instance_id: provider_name.to_string(),
        base_provider: provider_name.to_string(),
        display_name: provider_display_name(provider_name)?,
    })
}

async fn ensure_provider_for_model_management(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<(), ScratchError> {
    let (config_dir, already_registered) = {
        let registry = gcx.providers.read().await;
        (
            gcx.config_dir.clone(),
            registry.get(provider_name).is_some(),
        )
    };
    if already_registered {
        return Ok(());
    }
    if provider_file_exists(&config_dir, provider_name) {
        reload_provider_from_disk(gcx, provider_name, &config_dir).await
    } else {
        let identity = identity_for_model_management(&config_dir, provider_name).await?;
        let provider = create_provider(&identity.base_provider).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found or not configured", provider_name),
            )
        })?;
        let provider: Box<dyn ProviderTrait> = if identity.instance_id == identity.base_provider {
            provider
        } else {
            Box::new(ProviderInstance::new(
                identity.instance_id,
                identity.base_provider,
                identity.display_name,
                provider,
            ))
        };
        let mut registry = gcx.providers.write().await;
        registry.add(provider);
        Ok(())
    }
}

fn settings_with_identity(provider: &dyn ProviderTrait) -> serde_json::Value {
    let mut settings = provider.provider_settings_as_json();
    if let serde_json::Value::Object(map) = &mut settings {
        map.insert(
            "base_provider".to_string(),
            serde_json::Value::String(provider.base_provider_name().to_string()),
        );
        map.insert(
            "display_name".to_string(),
            serde_json::Value::String(provider.display_name().to_string()),
        );
    }
    settings
}

fn provider_list_item(provider: &dyn ProviderTrait) -> ProviderListItem {
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
    ProviderListItem {
        name: provider.name().to_string(),
        base_provider: provider.base_provider_name().to_string(),
        display_name: provider.display_name().to_string(),
        enabled,
        readonly,
        has_credentials: has_creds,
        status,
        model_count,
    }
}

fn provider_file_path(config_dir: &std::path::Path, instance_id: &str) -> std::path::PathBuf {
    config_store::provider_config_path(config_dir, instance_id)
}

fn provider_file_exists(config_dir: &std::path::Path, instance_id: &str) -> bool {
    provider_file_path(config_dir, instance_id).exists()
}

async fn read_existing_provider_settings(
    config_dir: &std::path::Path,
    instance_id: &str,
) -> Result<Option<serde_yaml::Value>, ScratchError> {
    let config_path = provider_file_path(config_dir, instance_id);
    if !provider_file_exists(config_dir, instance_id) {
        return Ok(None);
    }
    let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read config: {e}"),
        )
    })?;
    let value = serde_yaml::from_str(&content).map_err(|e| {
        ScratchError::new(
            StatusCode::CONFLICT,
            format!("Existing config is invalid YAML: {e}. Fix manually or delete the file."),
        )
    })?;
    Ok(Some(value))
}

fn provider_config_store_error(error: String) -> ScratchError {
    let status = if error.contains("invalid YAML") || error.contains("root is not a YAML mapping") {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    ScratchError::new(status, error)
}

fn resolve_provider_update_identity_from_existing(
    instance_id: &str,
    registry_identity: Option<HttpProviderIdentity>,
    settings: &serde_yaml::Value,
    existing_settings: Option<&serde_yaml::Value>,
) -> Result<HttpProviderIdentity, ScratchError> {
    validate_instance_id_for_http(instance_id)?;
    let request_base = yaml_string_field(settings, "base_provider", false)?;
    let request_display = yaml_string_field(settings, "display_name", true)?;
    let disk_identity = existing_settings
        .as_ref()
        .and_then(|value| provider_identity_from_yaml(instance_id, value).ok());
    let disk_display = existing_settings.as_ref().and_then(|value| {
        yaml_string_field(value, "display_name", true)
            .ok()
            .flatten()
    });

    let base_provider = if let Some(existing) = registry_identity.as_ref() {
        if let Some(request_base) = request_base.as_ref() {
            if request_base != &existing.base_provider {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "Provider instance '{}' already uses base_provider '{}'",
                        instance_id, existing.base_provider
                    ),
                ));
            }
        }
        existing.base_provider.clone()
    } else if let Some(identity) = disk_identity.as_ref() {
        if let Some(request_base) = request_base.as_ref() {
            if request_base != &identity.base_provider {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "Provider instance '{}' already uses base_provider '{}'",
                        instance_id, identity.base_provider
                    ),
                ));
            }
        }
        identity.base_provider.clone()
    } else if let Some(request_base) = request_base {
        request_base
    } else if create_provider(instance_id).is_some() {
        instance_id.to_string()
    } else {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Provider instance '{}' must include base_provider",
                instance_id
            ),
        ));
    };

    if create_provider(&base_provider).is_none() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Unknown base provider '{base_provider}'"),
        ));
    }

    let display_name = request_display
        .or_else(|| registry_identity.map(|identity| identity.display_name))
        .or(disk_display)
        .or_else(|| disk_identity.and_then(|identity| identity.display_name))
        .unwrap_or(provider_display_name(&base_provider)?);

    Ok(HttpProviderIdentity {
        instance_id: instance_id.to_string(),
        base_provider,
        display_name,
    })
}

fn settings_with_forced_identity(
    settings: serde_yaml::Value,
    identity: &HttpProviderIdentity,
) -> Result<serde_yaml::Value, ScratchError> {
    let mut map = ensure_settings_mapping(settings)?;
    map.insert(
        serde_yaml::Value::String("base_provider".to_string()),
        serde_yaml::Value::String(identity.base_provider.clone()),
    );
    map.insert(
        serde_yaml::Value::String("display_name".to_string()),
        serde_yaml::Value::String(identity.display_name.clone()),
    );
    Ok(serde_yaml::Value::Mapping(map))
}

fn provider_from_yaml(
    instance_id: &str,
    yaml: serde_yaml::Value,
) -> Result<Box<dyn ProviderTrait>, ScratchError> {
    let identity = provider_identity_from_yaml(instance_id, &yaml)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    let mut provider = create_provider(&identity.base_provider).ok_or_else(|| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Unknown base provider '{}'", identity.base_provider),
        )
    })?;
    provider.provider_settings_apply(yaml).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to apply settings: {e}"),
        )
    })?;
    if identity.wrap_instance {
        provider = match identity.display_name {
            Some(display_name) => Box::new(ProviderInstance::new(
                identity.instance_id,
                identity.base_provider,
                display_name,
                provider,
            )),
            None => Box::new(ProviderInstance::from_inner(identity.instance_id, provider)),
        };
    }
    Ok(provider)
}

#[derive(Serialize)]
struct ProviderListItem {
    name: String,
    base_provider: String,
    display_name: String,
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
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let registry = gcx.providers.read().await;

    let mut providers = Vec::new();
    let mut seen = HashSet::new();
    for (_, provider) in registry.iter() {
        seen.insert(provider.name().to_string());
        providers.push(provider_list_item(provider));
    }
    for name in PROVIDER_NAMES {
        if seen.contains(*name) {
            continue;
        }
        if let Some(default_provider) = create_provider(name) {
            if default_provider.is_hidden_from_list() {
                continue;
            }
            seen.insert((*name).to_string());
            providers.push(provider_list_item(default_provider.as_ref()));
        }
    }
    providers.sort_by(|a, b| a.name.cmp(&b.name));

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
    base_provider: String,
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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let (registry_provider, config_dir) = {
        let registry = gcx.providers.read().await;
        (
            registry
                .get(&params.name)
                .map(|provider| provider.clone_box()),
            gcx.config_dir.clone(),
        )
    };
    let provider: Box<dyn ProviderTrait> = if let Some(provider) = registry_provider {
        provider
    } else if let Some(settings) =
        read_existing_provider_settings(&config_dir, &params.name).await?
    {
        provider_from_yaml(&params.name, settings)?
    } else if let Some(provider) = create_provider(&params.name) {
        provider
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
        base_provider: provider.base_provider_name().to_string(),
        display_name: provider.display_name().to_string(),
        enabled,
        readonly: provider.is_readonly(),
        has_credentials: has_creds,
        selected_models_count: selected_count,
        status,
        settings: settings_with_identity(provider.as_ref()),
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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let schema = {
        let registry = gcx.providers.read().await;

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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
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

    let (config_dir, registry_identity) = {
        let registry = gcx.providers.read().await;
        let registry_identity = if let Some(provider) = registry.get(&params.name) {
            if provider.is_readonly() {
                return Err(ScratchError::new(
                    StatusCode::FORBIDDEN,
                    format!("Provider '{}' is readonly", params.name),
                ));
            }
            Some(HttpProviderIdentity {
                instance_id: provider.name().to_string(),
                base_provider: provider.base_provider_name().to_string(),
                display_name: provider.display_name().to_string(),
            })
        } else {
            None
        };
        (gcx.config_dir.clone(), registry_identity)
    };

    let settings = strip_derived_fields(settings);
    let identity_cell = std::sync::Arc::new(std::sync::Mutex::new(None));
    let identity_out = identity_cell.clone();
    let config_dir_for_update = config_dir.clone();
    let params_name = params.name.clone();
    config_store::update_provider_config_with(
        &config_dir_for_update,
        &params.name,
        provider_config_store_error,
        move |existing_settings| {
            let identity = resolve_provider_update_identity_from_existing(
                &params_name,
                registry_identity,
                &settings,
                existing_settings.as_ref(),
            )?;
            let settings = settings_with_forced_identity(settings, &identity)?;
            let had_existing = existing_settings.is_some();
            let existing = existing_settings
                .unwrap_or_else(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            let merged_settings = if had_existing || identity.base_provider == "custom" {
                merge_yaml_preserving_secrets_for_provider(
                    &identity.base_provider,
                    existing,
                    settings,
                )
                .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?
            } else {
                strip_masked_secrets(settings)
            };
            let merged_settings = settings_with_forced_identity(merged_settings, &identity)?;
            *identity_cell.lock().expect("identity lock poisoned") = Some(identity);
            Ok(merged_settings)
        },
    )
    .await?;
    let identity = identity_out
        .lock()
        .expect("identity lock poisoned")
        .clone()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Provider identity was not resolved".to_string(),
            )
        })?;

    reload_provider_from_disk(gcx.clone(), &identity.instance_id, &config_dir).await?;

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true}))
}

pub async fn handle_v1_provider_delete(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;

    let config_dir = {
        let registry = gcx.providers.read().await;
        if let Some(provider) = registry.get(&params.name) {
            if provider.is_readonly() {
                return Err(ScratchError::new(
                    StatusCode::FORBIDDEN,
                    format!("Provider '{}' is readonly", params.name),
                ));
            }
        } else if create_provider(&params.name).is_none() {
            if !provider_file_exists(&gcx.config_dir, &params.name) {
                return Err(ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Provider '{}' not found", params.name),
                ));
            }
        }
        gcx.config_dir.clone()
    };

    delete_provider_config(&config_dir, &params.name)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    {
        let mut registry = gcx.providers.write().await;
        registry.remove(&params.name);
    }

    invalidate_caps(gcx).await;

    json_response(StatusCode::OK, &json!({"success": true}))
}

#[derive(Serialize)]
struct ProviderModelsResponse {
    models: Vec<crate::providers::traits::ProviderModel>,
}

pub async fn handle_v1_provider_models(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let registry = gcx.providers.read().await;

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
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let config_dir = gcx.config_dir.clone();
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
    gcx: Arc<GlobalContext>,
    draft_id: &str,
) -> Result<(), ScratchError> {
    let buddy_arc = gcx.buddy.clone();
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
    gcx: Arc<GlobalContext>,
    draft_id: &str,
) -> Result<(), ScratchError> {
    let buddy_arc = gcx.buddy.clone();
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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: DefaultsUpdateRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    if let Some(draft_id) = req.draft_id.as_deref() {
        validate_defaults_draft(gcx.clone(), draft_id).await?;
    }

    let config_dir = app.paths.config_dir.clone();
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
    State(app): State<AppState>,
    Query(params): Query<ModelsQueryParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.provider_name)?;
    let registry = gcx.providers.read().await;

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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let (provider, http_client) = {
        let registry = gcx.providers.read().await;
        let http_client = gcx.http_client.clone();

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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
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

/// GET /v1/providers/:name/models/:model_id/endpoints
pub async fn handle_v1_openrouter_model_endpoints(
    State(app): State<AppState>,
    Path(params): Path<ProviderModelPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let (provider, http_client) =
        resolve_provider_for_base(&gcx, &params.name, "openrouter").await?;
    let openrouter = downcast_provider::<OpenRouterProvider>(provider.as_ref(), "OpenRouter")?;

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

async fn openrouter_account_info_response(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) =
        resolve_provider_for_base(&gcx, provider_name, "openrouter").await?;
    let openrouter = downcast_provider::<OpenRouterProvider>(provider.as_ref(), "OpenRouter")?;

    let account_info = openrouter
        .fetch_account_info(&http_client)
        .await
        .map_err(|e| ScratchError::new(StatusCode::BAD_GATEWAY, e))?;

    json_response(StatusCode::OK, &json!({"data": account_info}))
}

/// GET /v1/openrouter/account-info
pub async fn handle_v1_openrouter_account_info(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    openrouter_account_info_response(gcx, "openrouter").await
}

pub async fn handle_v1_provider_account_info(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let identity = resolve_provider_identity(&gcx, &params.name).await?;
    if !openrouter_base_provider_supported(&identity.base_provider) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not support account-info", params.name),
        ));
    }
    openrouter_account_info_response(gcx, &params.name).await
}

async fn openrouter_health_response(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) =
        resolve_provider_for_base(&gcx, provider_name, "openrouter").await?;
    let openrouter = downcast_provider::<OpenRouterProvider>(provider.as_ref(), "OpenRouter")?;

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

async fn google_gemini_health_response(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) =
        resolve_provider_for_base(&gcx, provider_name, "google_gemini").await?;
    let gemini = downcast_provider::<GoogleGeminiProvider>(provider.as_ref(), "Google Gemini")?;

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

/// GET /v1/openrouter/health
pub async fn handle_v1_openrouter_health(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    openrouter_health_response(gcx, "openrouter").await
}

/// GET /v1/google-gemini/health
pub async fn handle_v1_google_gemini_health(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    google_gemini_health_response(gcx, "google_gemini").await
}

pub async fn handle_v1_provider_health(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let identity = resolve_provider_identity(&gcx, &params.name).await?;
    if !health_base_provider_supported(&identity.base_provider) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not support health", params.name),
        ));
    }
    match identity.base_provider.as_str() {
        "openrouter" => openrouter_health_response(gcx, &params.name).await,
        "google_gemini" => google_gemini_health_response(gcx, &params.name).await,
        _ => Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not support health", params.name),
        )),
    }
}

async fn update_model_enabled_state(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
    model_id: &str,
    enabled: bool,
) -> Result<Response<Body>, ScratchError> {
    ensure_provider_for_model_management(gcx.clone(), provider_name).await?;
    // Capture previous state for rollback
    let (config_dir, previous_enabled_models, previous_disabled_models) = {
        let mut registry = gcx.providers.write().await;

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
        (gcx.config_dir.clone(), previous_enabled, previous_disabled)
    };

    // Try to save updated config
    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        // Rollback in-memory state on persistence failure
        let mut registry = gcx.providers.write().await;
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
    gcx: Arc<GlobalContext>,
    provider_name: &str,
    model_id: &str,
    selected_provider: Option<String>,
) -> Result<Response<Body>, ScratchError> {
    ensure_provider_for_model_management(gcx.clone(), provider_name).await?;
    let (config_dir, previous_selected_provider) = {
        let mut registry = gcx.providers.write().await;

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
        (gcx.config_dir.clone(), prev)
    };

    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        let mut registry = gcx.providers.write().await;
        if let Some(provider) = registry.get_mut(provider_name) {
            provider.set_selected_provider(model_id, previous_selected_provider);
        }
        return Err(e);
    }

    reload_provider_from_disk(gcx.clone(), provider_name, &config_dir).await?;

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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
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

    ensure_provider_for_model_management(gcx.clone(), &params.name).await?;
    let (config_dir, previous_config) = {
        let mut registry = gcx.providers.write().await;

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
        (gcx.config_dir.clone(), previous_config)
    };

    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, &params.name).await {
        let mut registry = gcx.providers.write().await;
        if let Some(provider) = registry.get_mut(&params.name) {
            if let Some(previous) = previous_config {
                provider.add_custom_model(request.id.clone(), previous);
            } else {
                provider.remove_custom_model(&request.id);
            }
        }
        return Err(e);
    }

    reload_provider_from_disk(gcx.clone(), &params.name, &config_dir).await?;

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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    handle_v1_provider_remove_custom_model_impl(gcx, &params.name, body_bytes).await
}

/// DELETE /v1/providers/{name}/custom-models
/// Remove a custom model from a provider (kept for backward compatibility)
/// Note: Some proxies may strip DELETE request bodies. Prefer POST /custom-models/remove.
/// Body: { "model_id": "my-custom-model" }
pub async fn handle_v1_provider_remove_custom_model(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    handle_v1_provider_remove_custom_model_impl(gcx, &params.name, body_bytes).await
}

async fn handle_v1_provider_remove_custom_model_impl(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_instance_id_for_http(provider_name)?;
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
        let mut registry = gcx.providers.write().await;

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
            gcx.config_dir.clone(),
            previous,
            previous_enabled,
            previous_disabled,
        )
    };

    // Try to save updated config, rollback on failure
    if let Err(e) = patch_provider_model_config(gcx.clone(), &config_dir, provider_name).await {
        // Rollback in-memory state
        let mut registry = gcx.providers.write().await;
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

    reload_provider_from_disk(gcx.clone(), provider_name, &config_dir).await?;

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

#[allow(dead_code)]
async fn merge_provider_settings_preserving_secrets(
    config_dir: &std::path::Path,
    instance_id: &str,
    base_provider: &str,
    new_settings: serde_yaml::Value,
) -> Result<serde_yaml::Value, ScratchError> {
    let config_path = provider_file_path(config_dir, instance_id);

    if !provider_file_exists(config_dir, instance_id) {
        if base_provider == "custom" {
            return merge_yaml_preserving_secrets_for_provider(
                base_provider,
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                new_settings,
            )
            .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e));
        }
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

    merge_yaml_preserving_secrets_for_provider(base_provider, existing, new_settings)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))
}

fn merge_yaml_preserving_secrets_for_provider(
    provider_name: &str,
    existing: serde_yaml::Value,
    new: serde_yaml::Value,
) -> Result<serde_yaml::Value, String> {
    use serde_yaml::Value;

    match (existing, new) {
        (Value::Mapping(mut existing_map), Value::Mapping(new_map)) => {
            for (key, new_value) in new_map {
                let replace_custom_extra_headers =
                    provider_name == "custom" && key.as_str() == Some("extra_headers");
                let existing_value = existing_map.remove(&key);
                let merged_value = if replace_custom_extra_headers {
                    merge_custom_extra_headers_replace(existing_value.as_ref(), &new_value)?
                } else if let Some(existing_value) = existing_value {
                    merge_yaml_preserving_secrets_for_provider(
                        provider_name,
                        existing_value,
                        new_value,
                    )?
                } else {
                    strip_masked_secrets(new_value)
                };
                existing_map.insert(key, merged_value);
            }
            Ok(Value::Mapping(existing_map))
        }
        (existing, Value::String(s)) if s == "***" => Ok(existing),
        (_, new) => Ok(strip_masked_secrets(new)),
    }
}

fn merge_custom_extra_headers_replace(
    existing: Option<&serde_yaml::Value>,
    incoming: &serde_yaml::Value,
) -> Result<serde_yaml::Value, String> {
    use serde_yaml::{Mapping, Value};

    let incoming_map = parse_extra_headers_value(incoming)?;
    let existing_headers = match existing {
        Some(value) => extra_headers_mapping_to_hash_map(None, &parse_extra_headers_value(value)?),
        None => HashMap::new(),
    };
    let merged_headers = extra_headers_mapping_to_hash_map(Some(&existing_headers), &incoming_map);
    let mut out = Mapping::new();

    for (key, value) in merged_headers {
        out.insert(Value::String(key), Value::String(value));
    }

    Ok(Value::Mapping(out))
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

fn identity_from_provider(provider: &dyn ProviderTrait) -> HttpProviderIdentity {
    HttpProviderIdentity {
        instance_id: provider.name().to_string(),
        base_provider: provider.base_provider_name().to_string(),
        display_name: provider.display_name().to_string(),
    }
}

fn ensure_yaml_identity_fields(
    yaml_map: &mut serde_yaml::Mapping,
    identity: &HttpProviderIdentity,
) {
    yaml_map.insert(
        serde_yaml::Value::String("base_provider".to_string()),
        serde_yaml::Value::String(identity.base_provider.clone()),
    );
    yaml_map.insert(
        serde_yaml::Value::String("display_name".to_string()),
        serde_yaml::Value::String(identity.display_name.clone()),
    );
}

/// Helper function to patch provider config - only updates enabled_models/disabled_models and custom_models
/// while preserving secrets and other fields.
///
/// SAFETY: This function will NOT write if the existing config is invalid YAML,
/// to prevent destroying secrets and other settings.
async fn patch_provider_model_config(
    gcx: Arc<GlobalContext>,
    config_dir: &std::path::Path,
    provider_name: &str,
) -> Result<(), ScratchError> {
    validate_instance_id_for_http(provider_name)?;
    let (identity, enabled_models, disabled_models, custom_models, selected_providers) = {
        let registry = gcx.providers.read().await;
        let provider = registry.get(provider_name).ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Provider '{}' not found", provider_name),
            )
        })?;
        (
            identity_from_provider(provider),
            provider.enabled_models().to_vec(),
            provider.disabled_models().to_vec(),
            provider.custom_models().clone(),
            provider.selected_providers().clone(),
        )
    };

    config_store::update_provider_config_with(
        config_dir,
        provider_name,
        provider_config_store_error,
        |existing| {
            let mut yaml_map = match existing {
                Some(value) => value.as_mapping().cloned().ok_or_else(|| {
                    ScratchError::new(
                        StatusCode::CONFLICT,
                        "Config file root is not a YAML mapping. Cannot safely patch.".to_string(),
                    )
                })?,
                None => serde_yaml::Mapping::new(),
            };

            ensure_yaml_identity_fields(&mut yaml_map, &identity);

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

            Ok(serde_yaml::Value::Mapping(yaml_map))
        },
    )
    .await?;

    Ok(())
}

async fn reload_provider_from_disk(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
    config_dir: &std::path::Path,
) -> Result<(), ScratchError> {
    validate_instance_id_for_http(provider_name)?;
    if !provider_file_exists(config_dir, provider_name) {
        return Ok(());
    }

    let provider_path = provider_file_path(config_dir, provider_name);
    let content = tokio::fs::read_to_string(provider_path)
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

    let mut registry = gcx.providers.write().await;

    let provider = provider_from_yaml(provider_name, yaml)?;
    registry.add(provider);

    Ok(())
}

fn ensure_oauth_identity_for_instance_sync(
    config_dir: &std::path::Path,
    provider_name: &str,
    base_provider: &str,
    yaml_map: &mut serde_yaml::Mapping,
) -> Result<HttpProviderIdentity, ScratchError> {
    let identity = match provider_identity_from_yaml(
        provider_name,
        &serde_yaml::Value::Mapping(yaml_map.clone()),
    ) {
        Ok(identity) => HttpProviderIdentity {
            display_name: identity
                .display_name
                .clone()
                .unwrap_or(provider_display_name(&identity.base_provider)?),
            instance_id: identity.instance_id,
            base_provider: identity.base_provider,
        },
        Err(_) if !provider_file_exists(config_dir, provider_name) => HttpProviderIdentity {
            instance_id: provider_name.to_string(),
            base_provider: base_provider.to_string(),
            display_name: provider_display_name(base_provider)?,
        },
        Err(e) => return Err(ScratchError::new(StatusCode::BAD_REQUEST, e)),
    };
    if identity.base_provider != base_provider {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "OAuth not supported for provider '{}' with base_provider '{}'",
                provider_name, identity.base_provider
            ),
        ));
    }
    ensure_yaml_identity_fields(yaml_map, &identity);
    Ok(identity)
}

#[derive(Deserialize, Default)]
struct GitHubCopilotOAuthStartRequest {
    #[serde(default, alias = "enterpriseUrl")]
    enterprise_url: Option<String>,
    #[serde(default, alias = "deploymentType")]
    deployment_type: Option<String>,
}

async fn oauth_base_provider_for_instance(
    gcx: &Arc<GlobalContext>,
    provider_name: &str,
) -> Result<String, ScratchError> {
    let identity = resolve_provider_identity(gcx, provider_name).await?;
    if !oauth_supported_for_base(&identity.base_provider) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("OAuth not supported for provider '{}'", provider_name),
        ));
    }
    Ok(identity.base_provider)
}

pub async fn handle_v1_provider_oauth_start(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let base_provider = oauth_base_provider_for_instance(&gcx, &params.name).await?;
    match base_provider.as_str() {
        "claude_code" => {
            let mode = crate::providers::claude_code_oauth::OAuthMode::Max;
            let (session_id, authorize_url) =
                crate::providers::claude_code_oauth::start_oauth_session(mode, params.name.clone())
                    .await;
            json_response(
                StatusCode::OK,
                &json!({
                    "session_id": session_id,
                    "authorize_url": authorize_url,
                }),
            )
        }
        "openai_codex" => {
            let fallback_port = gcx.cmdline.http_port;
            let (session_id, authorize_url, callback_port) =
                crate::providers::openai_codex_oauth::start_oauth_session(
                    fallback_port,
                    params.name.clone(),
                )
                .await;

            if callback_port != fallback_port {
                let http_client = gcx.http_client.clone();
                match crate::providers::openai_codex_oauth::start_callback_listener(
                    callback_port,
                    http_client,
                )
                .await
                {
                    Ok(listener_handle) => {
                        let gcx_clone = gcx.clone();
                        tokio::spawn(async move {
                            if let Some((tokens, provider_instance_id)) =
                                listener_handle.await.ok().flatten()
                            {
                                let config_dir = gcx_clone.config_dir.clone();
                                if let Ok(tokens_value) = serde_yaml::to_value(&tokens) {
                                    if let Err(e) = save_provider_oauth_tokens(
                                        &gcx_clone,
                                        &config_dir,
                                        &provider_instance_id,
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
            let http_client = gcx.http_client.clone();
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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let request: OAuthExchangeRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    validate_instance_id_for_http(&params.name)?;
    let base_provider = oauth_base_provider_for_instance(&gcx, &params.name).await?;
    let http_client = gcx.http_client.clone();
    let config_dir = gcx.config_dir.clone();

    match base_provider.as_str() {
        "claude_code" => {
            let (tokens, session_provider_name) =
                crate::providers::claude_code_oauth::exchange_code(
                    &http_client,
                    &request.session_id,
                    &request.code,
                    &params.name,
                )
                .await
                .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
            if session_provider_name != params.name {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "OAuth session belongs to provider '{}'",
                        session_provider_name
                    ),
                ));
            }

            save_provider_oauth_tokens(
                &gcx,
                &config_dir,
                &session_provider_name,
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
            let (tokens, session_provider_name) =
                crate::providers::openai_codex_oauth::exchange_code_for_session(
                    &http_client,
                    &request.session_id,
                    &request.code,
                )
                .await
                .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
            if session_provider_name != params.name {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "OAuth session belongs to provider '{}'",
                        session_provider_name
                    ),
                ));
            }

            save_provider_oauth_tokens(
                &gcx,
                &config_dir,
                &session_provider_name,
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
                        &params.name,
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
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let base_provider = oauth_base_provider_for_instance(&gcx, &params.name).await?;
    let config_dir = gcx.config_dir.clone();

    match base_provider.as_str() {
        "claude_code" => {
            let empty =
                serde_yaml::to_value(&crate::providers::claude_code_oauth::OAuthTokens::default())
                    .map_err(|e| {
                        ScratchError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Failed to serialize: {}", e),
                        )
                    })?;
            save_provider_oauth_tokens(&gcx, &config_dir, &params.name, "claude_code", &empty)
                .await?;
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
            save_provider_oauth_tokens(&gcx, &config_dir, &params.name, "openai_codex", &empty)
                .await?;
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
            save_provider_oauth_tokens(&gcx, &config_dir, &params.name, "github_copilot", &empty)
                .await?;
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

async fn handle_openai_codex_oauth_callback_impl(
    gcx: Arc<GlobalContext>,
    query: OAuthCallbackParams,
    expected_provider_name: Option<String>,
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

    let http_client = gcx.http_client.clone();
    let config_dir = gcx.config_dir.clone();

    let (tokens, provider_instance_id) =
        match crate::providers::openai_codex_oauth::exchange_code_for_session(
            &http_client,
            &session_id,
            &code,
        )
        .await
        {
            Ok(result) => result,
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

    if let Some(expected_provider_name) = expected_provider_name {
        if provider_instance_id != expected_provider_name {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                &format!(
                    "OAuth session belongs to provider '{}'. Please restart login.",
                    provider_instance_id
                ),
            );
        }
    }

    if let Err(e) = save_provider_oauth_tokens(
        &gcx,
        &config_dir,
        &provider_instance_id,
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

pub async fn handle_v1_provider_oauth_callback(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
    Query(query): Query<OAuthCallbackParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let base_provider = oauth_base_provider_for_instance(&gcx, &params.name).await?;
    if base_provider != "openai_codex" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "OAuth callback not supported for provider '{}'",
                params.name
            ),
        ));
    }
    handle_openai_codex_oauth_callback_impl(gcx, query, Some(params.name)).await
}

pub async fn handle_openai_codex_auth_callback(
    State(app): State<AppState>,
    Query(query): Query<OAuthCallbackParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    handle_openai_codex_oauth_callback_impl(gcx, query, None).await
}

async fn claude_code_usage_response(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<Response<Body>, ScratchError> {
    let (provider, http_client) =
        resolve_provider_for_base(&gcx, provider_name, "claude_code").await?;
    let claude_code = downcast_provider::<ClaudeCodeProvider>(provider.as_ref(), "Claude Code")?;

    match claude_code.fetch_usage(&http_client).await {
        Ok(usage) => json_response(StatusCode::OK, &json!({"data": usage})),
        Err(e) => json_response(StatusCode::OK, &json!({"error": e})),
    }
}

async fn openai_codex_usage_response(
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<Response<Body>, ScratchError> {
    let result = fetch_openai_codex_usage_with_refresh(gcx, provider_name).await;

    match result {
        Ok(usage) => json_response(StatusCode::OK, &json!({"data": usage})),
        Err(e) => json_response(StatusCode::OK, &json!({"error": e})),
    }
}

/// GET /v1/claude-code/usage
pub async fn handle_v1_claude_code_usage(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    claude_code_usage_response(gcx, "claude_code").await
}

/// GET /v1/openai-codex/usage
pub async fn handle_v1_openai_codex_usage(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    openai_codex_usage_response(gcx, "openai_codex").await
}

pub async fn handle_v1_provider_usage(
    State(app): State<AppState>,
    Path(params): Path<ProviderPathParams>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_instance_id_for_http(&params.name)?;
    let identity = resolve_provider_identity(&gcx, &params.name).await?;
    if !usage_base_provider_supported(&identity.base_provider) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not support usage", params.name),
        ));
    }
    match identity.base_provider.as_str() {
        "claude_code" => claude_code_usage_response(gcx, &params.name).await,
        "openai_codex" => openai_codex_usage_response(gcx, &params.name).await,
        _ => Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not support usage", params.name),
        )),
    }
}

async fn current_openai_codex_provider(
    gcx: &Arc<GlobalContext>,
    provider_name: &str,
) -> Result<(OpenAICodexProvider, reqwest::Client, std::path::PathBuf), String> {
    let (provider, http_client, config_dir) = {
        let registry = gcx.providers.read().await;
        (
            registry
                .get(provider_name)
                .map(|provider| provider.clone_box()),
            gcx.http_client.clone(),
            gcx.config_dir.clone(),
        )
    };
    let provider = if let Some(provider) = provider {
        provider
    } else if provider_file_exists(&config_dir, provider_name) {
        let settings = read_existing_provider_settings(&config_dir, provider_name)
            .await
            .map_err(|e| e.message)?
            .ok_or_else(|| format!("OpenAI Codex provider '{}' is not available", provider_name))?;
        provider_from_yaml(provider_name, settings).map_err(|e| e.message)?
    } else if provider_name == "openai_codex" {
        create_provider("openai_codex")
            .ok_or_else(|| "OpenAI Codex provider is not available".to_string())?
    } else {
        return Err(format!(
            "OpenAI Codex provider '{}' is not available",
            provider_name
        ));
    };
    if provider.base_provider_name() != "openai_codex" {
        return Err(format!(
            "Provider '{}' is not an OpenAI Codex instance",
            provider_name
        ));
    }
    let Some(codex) = provider.as_any().downcast_ref::<OpenAICodexProvider>() else {
        return Err("Failed to resolve OpenAI Codex provider type".to_string());
    };
    Ok((codex.clone(), http_client, config_dir))
}

async fn force_refresh_openai_codex_usage_for_retry(
    gcx: Arc<GlobalContext>,
    http_client: &reqwest::Client,
    provider_name: &str,
    rejected_access_token: &str,
    rejected_status: Option<reqwest::StatusCode>,
) -> Result<Option<OpenAICodexProvider>, String> {
    let _guard = OpenAICodexProvider::lock_refresh_guard().await?;
    let (mut provider, _, config_dir) = current_openai_codex_provider(&gcx, provider_name).await?;

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
        .force_refresh_after_auth_rejection(http_client, &config_dir, provider_name)
        .await;

    if !provider.auth_state_matches(&previous_tokens, &previous_session_id) {
        if sync_openai_codex_auth_state(
            gcx.clone(),
            provider_name,
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
    gcx: Arc<GlobalContext>,
    provider_name: &str,
) -> Result<crate::providers::openai_codex::OpenAICodexUsage, String> {
    let (mut request_provider, http_client, _) =
        current_openai_codex_provider(&gcx, provider_name).await?;
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
                provider_name,
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
                provider_name,
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
    gcx: Arc<GlobalContext>,
    provider_name: &str,
    source: &OpenAICodexProvider,
    previous_tokens: &crate::providers::openai_codex_oauth::OAuthTokens,
    previous_session_id: &str,
) -> Result<bool, ScratchError> {
    if source.auth_state_matches(previous_tokens, previous_session_id) {
        return Ok(false);
    }

    let mut registry = gcx.providers.write().await;
    let provider = registry.get_mut(provider_name).ok_or_else(|| {
        ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("OpenAI Codex provider '{}' is not available", provider_name),
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
    gcx: &Arc<GlobalContext>,
    config_dir: &std::path::Path,
    provider_name: &str,
    base_provider: &str,
    tokens_value: &serde_yaml::Value,
) -> Result<(), ScratchError> {
    validate_instance_id_for_http(provider_name)?;
    if !oauth_supported_for_base(base_provider) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("OAuth not supported for provider '{}'", provider_name),
        ));
    }
    let _openai_codex_refresh_guard = if base_provider == "openai_codex" {
        Some(
            OpenAICodexProvider::lock_refresh_guard()
                .await
                .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?,
        )
    } else {
        None
    };
    config_store::update_provider_config_with(
        config_dir,
        provider_name,
        provider_config_store_error,
        |existing| {
            let mut yaml_map = match existing {
                Some(value) => value.as_mapping().cloned().ok_or_else(|| {
                    ScratchError::new(
                        StatusCode::CONFLICT,
                        "Config file root is not a YAML mapping. Cannot safely patch.".to_string(),
                    )
                })?,
                None => serde_yaml::Mapping::new(),
            };

            ensure_oauth_identity_for_instance_sync(
                config_dir,
                provider_name,
                base_provider,
                &mut yaml_map,
            )?;

            yaml_map.insert(
                serde_yaml::Value::String("oauth_tokens".to_string()),
                tokens_value.clone(),
            );

            if base_provider == "openai_codex" {
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

            Ok(serde_yaml::Value::Mapping(yaml_map))
        },
    )
    .await?;

    reload_provider_from_disk(gcx.clone(), provider_name, config_dir).await?;

    invalidate_caps(gcx.clone()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn response_json(response: Response<Body>) -> serde_json::Value {
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn provider_config_json(
        config_dir: &std::path::Path,
        provider_name: &str,
    ) -> serde_json::Value {
        let content = tokio::fs::read_to_string(provider_file_path(&config_dir, provider_name))
            .await
            .unwrap();
        serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(&content).unwrap()).unwrap()
    }

    fn merged_settings_json(
        provider_name: &str,
        existing: &str,
        incoming: &str,
    ) -> serde_json::Value {
        let existing = serde_yaml::from_str(existing).unwrap();
        let incoming = serde_yaml::from_str(incoming).unwrap();
        serde_json::to_value(
            merge_yaml_preserving_secrets_for_provider(provider_name, existing, incoming).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn custom_provider_merge_replaces_extra_headers_map() {
        let merged = merged_settings_json(
            "custom",
            r#"
api_key: sk-old
extra_headers:
  X-Keep: keep-secret
  X-Replace: old-value
  X-Remove-Null: old-null
  X-Remove-Number: old-number
  X-Absent: old-absent
"#,
            r#"
api_key: "***"
extra_headers:
  X-Keep: "***"
  X-Replace: new-value
  X-Remove-Null:
  X-Remove-Number: 7
"#,
        );

        assert_eq!(merged["api_key"], "sk-old");
        assert_eq!(merged["extra_headers"]["X-Keep"], "keep-secret");
        assert_eq!(merged["extra_headers"]["X-Replace"], "new-value");
        assert!(merged["extra_headers"].get("X-Remove-Null").is_none());
        assert!(merged["extra_headers"].get("X-Remove-Number").is_none());
        assert!(merged["extra_headers"].get("X-Absent").is_none());
    }

    #[test]
    fn custom_provider_merge_empty_extra_headers_clears_all() {
        let merged = merged_settings_json(
            "custom",
            r#"
extra_headers:
  X-Secret: old-secret
"#,
            r#"
extra_headers: {}
"#,
        );

        assert!(merged["extra_headers"].as_object().unwrap().is_empty());
    }

    #[test]
    fn custom_provider_merge_null_extra_headers_clears_all() {
        let merged = merged_settings_json(
            "custom",
            r#"
extra_headers:
  X-Secret: old-secret
"#,
            r#"
extra_headers:
"#,
        );

        assert!(merged["extra_headers"].as_object().unwrap().is_empty());
    }

    #[test]
    fn custom_provider_merge_yaml_string_extra_headers() {
        let merged = merged_settings_json(
            "custom",
            r#"
extra_headers:
  X-Keep: keep-secret
  X-Absent: old-absent
"#,
            r#"
extra_headers: |
  X-Keep: "***"
  X-New: new-value
  X-Remove-Number: 7
"#,
        );

        assert_eq!(merged["extra_headers"]["X-Keep"], "keep-secret");
        assert_eq!(merged["extra_headers"]["X-New"], "new-value");
        assert!(merged["extra_headers"].get("X-Remove-Number").is_none());
        assert!(merged["extra_headers"].get("X-Absent").is_none());
    }

    #[test]
    fn custom_provider_merge_json_string_extra_headers() {
        let merged = merged_settings_json(
            "custom",
            r#"
extra_headers:
  X-Keep: keep-secret
  X-Absent: old-absent
"#,
            r#"
extra_headers: '{"X-Keep":"***","X-Json":"json-value","X-Remove":7}'
"#,
        );

        assert_eq!(merged["extra_headers"]["X-Keep"], "keep-secret");
        assert_eq!(merged["extra_headers"]["X-Json"], "json-value");
        assert!(merged["extra_headers"].get("X-Remove").is_none());
        assert!(merged["extra_headers"].get("X-Absent").is_none());
    }

    #[test]
    fn custom_provider_merge_invalid_string_extra_headers_errors() {
        let existing = serde_yaml::from_str(
            r#"
extra_headers:
  X-Secret: old-secret
"#,
        )
        .unwrap();
        let incoming = serde_yaml::from_str("extra_headers: '['").unwrap();
        let err =
            merge_yaml_preserving_secrets_for_provider("custom", existing, incoming).unwrap_err();

        assert!(err.contains("extra_headers"));
    }

    #[test]
    fn custom_provider_merge_absent_extra_headers_preserves_existing() {
        let merged = merged_settings_json(
            "custom",
            r#"
api_key: sk-old
extra_headers:
  X-Secret: old-secret
"#,
            r#"
api_key: "***"
"#,
        );

        assert_eq!(merged["api_key"], "sk-old");
        assert_eq!(merged["extra_headers"]["X-Secret"], "old-secret");
    }

    #[test]
    fn non_custom_provider_merge_preserves_nested_omitted_keys() {
        let merged = merged_settings_json(
            "openai_codex",
            r#"
oauth_tokens:
  access_token: old-access
  refresh_token: old-refresh
"#,
            r#"
oauth_tokens:
  access_token: new-access
"#,
        );

        assert_eq!(merged["oauth_tokens"]["access_token"], "new-access");
        assert_eq!(merged["oauth_tokens"]["refresh_token"], "old-refresh");
    }

    #[tokio::test]
    async fn provider_update_openai_alias_writes_instance_file_with_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let response = handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "openai",
                    "display_name": "Work OpenAI",
                    "api_key": "sk-two",
                    "enabled": true,
                    "enabled_models": ["gpt-4.1"]
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert!(provider_file_exists(&config_dir, "openai_2"));
        assert!(!provider_file_exists(&config_dir, "openai"));
        let saved = provider_config_json(&config_dir, "openai_2").await;
        assert_eq!(saved["base_provider"], "openai");
        assert_eq!(saved["display_name"], "Work OpenAI");
        assert_eq!(saved["api_key"], "sk-two");

        let (name, base, display, enabled_models) = {
            let registry = gcx.providers.read().await;
            let provider = registry.get("openai_2").unwrap();
            (
                provider.name().to_string(),
                provider.base_provider_name().to_string(),
                provider.display_name().to_string(),
                provider.enabled_models().to_vec(),
            )
        };
        assert_eq!(name, "openai_2");
        assert_eq!(base, "openai");
        assert_eq!(display, "Work OpenAI");
        assert_eq!(enabled_models, vec!["gpt-4.1".to_string()]);
    }

    #[tokio::test]
    async fn provider_get_openai_alias_returns_identity_fields() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "openai",
                    "display_name": "Work OpenAI",
                    "api_key": "sk-two"
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

        let response = handle_v1_provider_get(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;

        assert_eq!(body["name"], "openai_2");
        assert_eq!(body["base_provider"], "openai");
        assert_eq!(body["display_name"], "Work OpenAI");
        assert_eq!(body["settings"]["base_provider"], "openai");
        assert_eq!(body["settings"]["display_name"], "Work OpenAI");
    }

    #[tokio::test]
    async fn provider_model_toggle_updates_only_alias_file_and_preserves_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openai.yaml"),
            "base_provider: openai\ndisplay_name: Main OpenAI\napi_key: sk-main\nenabled_models:\n  - gpt-4.1\n",
        )
        .await
        .unwrap();
        reload_provider_from_disk(gcx.clone(), "openai", &config_dir)
            .await
            .unwrap();
        handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "openai",
                    "display_name": "Work OpenAI",
                    "api_key": "sk-two",
                    "enabled_models": ["gpt-4.1-mini"]
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

        let response = handle_v1_provider_model_toggle(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
            hyper::body::Bytes::from(r#"{"model_id":"gpt-4.1","enabled":true}"#),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let main = provider_config_json(&config_dir, "openai").await;
        let alias = provider_config_json(&config_dir, "openai_2").await;
        assert_eq!(main["enabled_models"], json!(["gpt-4.1"]));
        assert_eq!(alias["base_provider"], "openai");
        assert_eq!(alias["display_name"], "Work OpenAI");
        assert_eq!(alias["enabled_models"], json!(["gpt-4.1-mini", "gpt-4.1"]));
    }

    #[tokio::test]
    async fn custom_alias_update_preserves_masked_api_key_and_replaces_extra_headers() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("custom_2.yaml"),
            r#"
base_provider: custom
display_name: Work Custom
api_key: sk-old
chat_endpoint: https://example.com/v1/chat/completions
enabled: true
enabled_models:
  - custom-model
custom_models:
  custom-model:
    n_ctx: 4096
extra_headers:
  X-Keep: keep-secret
  X-Replace: old-value
  X-Remove: remove-me
"#,
        )
        .await
        .unwrap();

        let response = handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "custom_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "custom",
                    "display_name": "Work Custom",
                    "api_key": "***",
                    "extra_headers": {
                        "X-Keep": "***",
                        "X-Replace": "new-value"
                    }
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let saved = provider_config_json(&config_dir, "custom_2").await;
        assert_eq!(saved["base_provider"], "custom");
        assert_eq!(saved["display_name"], "Work Custom");
        assert_eq!(saved["api_key"], "sk-old");
        assert_eq!(saved["extra_headers"]["X-Keep"], "keep-secret");
        assert_eq!(saved["extra_headers"]["X-Replace"], "new-value");
        assert!(saved["extra_headers"].get("X-Remove").is_none());

        let runtime = {
            let registry = gcx.providers.read().await;
            registry.get("custom_2").unwrap().build_runtime().unwrap()
        };
        assert_eq!(runtime.name, "custom_2");
        assert_eq!(runtime.display_name, "Work Custom");
        assert_eq!(runtime.api_key, "sk-old");
        assert_eq!(
            runtime.extra_headers.get("X-Keep").map(String::as_str),
            Some("keep-secret")
        );
        assert_eq!(
            runtime.extra_headers.get("X-Replace").map(String::as_str),
            Some("new-value")
        );
        assert!(runtime.extra_headers.get("X-Remove").is_none());
    }

    #[tokio::test]
    async fn provider_delete_instance_removes_only_that_instance() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openai.yaml"),
            "base_provider: openai\ndisplay_name: Main OpenAI\napi_key: sk-main\n",
        )
        .await
        .unwrap();
        reload_provider_from_disk(gcx.clone(), "openai", &config_dir)
            .await
            .unwrap();
        handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "openai",
                    "display_name": "Work OpenAI",
                    "api_key": "sk-two"
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

        let response = handle_v1_provider_delete(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert!(provider_file_exists(&config_dir, "openai"));
        assert!(!provider_file_exists(&config_dir, "openai_2"));
        let main = provider_config_json(&config_dir, "openai").await;
        assert_eq!(main["api_key"], "sk-main");
        let (has_main, has_alias) = {
            let registry = gcx.providers.read().await;
            (
                registry.get("openai").is_some(),
                registry.get("openai_2").is_some(),
            )
        };
        assert!(has_main);
        assert!(!has_alias);
    }

    #[tokio::test]
    async fn custom_provider_update_removes_deleted_extra_headers_on_disk_and_runtime() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("custom.yaml"),
            r#"
api_key: sk-old
chat_endpoint: https://example.com/v1/chat/completions
enabled: true
enabled_models:
  - custom-model
custom_models:
  custom-model:
    n_ctx: 4096
extra_headers:
  X-Keep: keep-secret
  X-Replace: old-value
  X-Remove: remove-me
"#,
        )
        .await
        .unwrap();

        let body = serde_json::to_vec(&json!({
            "api_key": "***",
            "extra_headers": {
                "X-Keep": "***",
                "X-Replace": "new-value"
            }
        }))
        .unwrap();
        let response = handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "custom".to_string(),
            }),
            hyper::body::Bytes::from(body),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let content = tokio::fs::read_to_string(providers_dir.join("custom.yaml"))
            .await
            .unwrap();
        let saved: serde_json::Value =
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(&content).unwrap())
                .unwrap();
        assert_eq!(saved["api_key"], "sk-old");
        assert_eq!(saved["extra_headers"]["X-Keep"], "keep-secret");
        assert_eq!(saved["extra_headers"]["X-Replace"], "new-value");
        assert!(saved["extra_headers"].get("X-Remove").is_none());

        let runtime = {
            let registry = gcx.providers.read().await;
            registry.get("custom").unwrap().build_runtime().unwrap()
        };
        assert_eq!(
            runtime.extra_headers.get("X-Keep").map(String::as_str),
            Some("keep-secret")
        );
        assert_eq!(
            runtime.extra_headers.get("X-Replace").map(String::as_str),
            Some("new-value")
        );
        assert!(runtime.extra_headers.get("X-Remove").is_none());
    }

    #[tokio::test]
    async fn custom_provider_update_invalid_extra_headers_string_returns_422_and_preserves_file() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        let config_path = providers_dir.join("custom.yaml");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            &config_path,
            r#"
api_key: sk-old
chat_endpoint: https://example.com/v1/chat/completions
enabled: true
enabled_models:
  - custom-model
extra_headers:
  X-Secret: keep-secret
"#,
        )
        .await
        .unwrap();

        let body = serde_json::to_vec(&json!({
            "extra_headers": "["
        }))
        .unwrap();
        let err = handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "custom".to_string(),
            }),
            hyper::body::Bytes::from(body),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(err.message.contains("extra_headers"));

        let content = tokio::fs::read_to_string(config_path).await.unwrap();
        let saved: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(
            saved
                .get("extra_headers")
                .and_then(|headers| headers.get("X-Secret"))
                .and_then(|value| value.as_str()),
            Some("keep-secret")
        );
    }

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
        let config_dir = gcx.config_dir.clone();
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

        save_provider_oauth_tokens(&gcx, &config_dir, "openai_codex", "openai_codex", &empty)
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
        let config_dir = gcx.config_dir.clone();
        let tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            openai_api_key: "sk-new".to_string(),
            access_token: "access".to_string(),
            expires_at: i64::MAX,
            ..Default::default()
        };
        let tokens_value = serde_yaml::to_value(&tokens).unwrap();

        save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "openai_codex",
            "openai_codex",
            &tokens_value,
        )
        .await
        .unwrap();

        let content = tokio::fs::read_to_string(provider_file_path(&config_dir, "openai_codex"))
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
        let config_dir = gcx.config_dir.clone();
        let tokens = crate::providers::github_copilot_oauth::OAuthTokens {
            access_token: "gho-secret".to_string(),
            expires_at: 0,
            enterprise_url: Some("company.ghe.com".to_string()),
            api_base: Some("https://copilot-api.company.ghe.com".to_string()),
        };
        let tokens_value = serde_yaml::to_value(&tokens).unwrap();

        save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "github_copilot",
            "github_copilot",
            &tokens_value,
        )
        .await
        .unwrap();

        let content = tokio::fs::read_to_string(provider_file_path(&config_dir, "github_copilot"))
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
            let registry = gcx.providers.read().await;
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
        let config_dir = gcx.config_dir.clone();
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

        save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "github_copilot",
            "github_copilot",
            &empty,
        )
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

    #[tokio::test]
    async fn save_oauth_tokens_writes_alias_config_and_preserves_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openai_codex_2.yaml"),
            "base_provider: openai_codex\ndisplay_name: Work Codex\noauth_tokens:\n  access_token: old\n",
        )
        .await
        .unwrap();
        let tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "alias-access".to_string(),
            refresh_token: "alias-refresh".to_string(),
            openai_api_key: "sk-alias".to_string(),
            expires_at: i64::MAX,
            ..Default::default()
        };

        save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "openai_codex_2",
            "openai_codex",
            &serde_yaml::to_value(&tokens).unwrap(),
        )
        .await
        .unwrap();

        assert!(provider_file_exists(&config_dir, "openai_codex_2"));
        assert!(!provider_file_exists(&config_dir, "openai_codex"));
        let saved = provider_config_json(&config_dir, "openai_codex_2").await;
        assert_eq!(saved["base_provider"], "openai_codex");
        assert_eq!(saved["display_name"], "Work Codex");
        assert_eq!(saved["OPENAI_API_KEY"], "sk-alias");
        assert_eq!(saved["oauth_tokens"]["access_token"], "alias-access");
        let identity = {
            let registry = gcx.providers.read().await;
            let provider = registry.get("openai_codex_2").unwrap();
            (
                provider.name().to_string(),
                provider.base_provider_name().to_string(),
                provider.display_name().to_string(),
            )
        };
        assert_eq!(
            identity,
            (
                "openai_codex_2".to_string(),
                "openai_codex".to_string(),
                "Work Codex".to_string()
            )
        );
    }

    #[tokio::test]
    async fn concurrent_oauth_save_and_model_toggle_preserve_auth_and_models() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openai_codex_2.yaml"),
            "base_provider: openai_codex\ndisplay_name: Work Codex\noauth_tokens:\n  access_token: old\n",
        )
        .await
        .unwrap();
        reload_provider_from_disk(gcx.clone(), "openai_codex_2", &config_dir)
            .await
            .unwrap();
        let tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "alias-access".to_string(),
            refresh_token: "alias-refresh".to_string(),
            openai_api_key: "sk-alias".to_string(),
            expires_at: i64::MAX,
            ..Default::default()
        };
        let tokens_value = serde_yaml::to_value(&tokens).unwrap();

        let save = save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "openai_codex_2",
            "openai_codex",
            &tokens_value,
        );
        let toggle =
            update_model_enabled_state(gcx.clone(), "openai_codex_2", "gpt-5.6-codex", true);
        let (save_result, toggle_result) = tokio::join!(save, toggle);
        save_result.unwrap();
        toggle_result.unwrap();

        let saved = provider_config_json(&config_dir, "openai_codex_2").await;
        assert_eq!(saved["oauth_tokens"]["access_token"], "alias-access");
        assert_eq!(saved["oauth_tokens"]["refresh_token"], "alias-refresh");
        assert_eq!(saved["OPENAI_API_KEY"], "sk-alias");
        assert!(saved["enabled_models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model.as_str() == Some("gpt-5.6-codex")));
    }

    #[tokio::test]
    async fn concurrent_refresh_save_and_custom_model_update_preserve_auth_and_models() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("claude_code_2.yaml"),
            "base_provider: claude_code\ndisplay_name: Work Claude\noauth_tokens:\n  access_token: old\n  refresh_token: old-refresh\n  expires_at: 1\n",
        )
        .await
        .unwrap();
        reload_provider_from_disk(gcx.clone(), "claude_code_2", &config_dir)
            .await
            .unwrap();
        {
            let mut registry = gcx.providers.write().await;
            registry.get_mut("claude_code_2").unwrap().add_custom_model(
                "claude-custom".to_string(),
                CustomModelConfig {
                    n_ctx: Some(4096),
                    ..Default::default()
                },
            );
        }

        let refresh_save = crate::providers::oauth_refresh::save_refreshed_tokens(
            &gcx,
            &config_dir,
            "claude_code_2",
            "claude_code",
            "Work Claude",
            "new-access",
            "new-refresh",
            i64::MAX,
        );
        let patch_model = patch_provider_model_config(gcx.clone(), &config_dir, "claude_code_2");
        let (refresh_result, model_result) = tokio::join!(refresh_save, patch_model);
        refresh_result.unwrap();
        model_result.unwrap();

        let saved = provider_config_json(&config_dir, "claude_code_2").await;
        assert_eq!(saved["oauth_tokens"]["access_token"], "new-access");
        assert_eq!(saved["oauth_tokens"]["refresh_token"], "new-refresh");
        assert_eq!(saved["custom_models"]["claude-custom"]["n_ctx"], 4096);
    }

    #[tokio::test]
    async fn concurrent_token_save_and_custom_model_update_preserve_auth_and_models() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "custom_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "custom",
                    "display_name": "Work Custom",
                    "api_key": "sk-old",
                    "chat_endpoint": "https://example.com/v1/chat/completions",
                    "enabled": true,
                    "enabled_models": []
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        {
            let mut registry = gcx.providers.write().await;
            registry.get_mut("custom_2").unwrap().add_custom_model(
                "my-custom".to_string(),
                CustomModelConfig {
                    n_ctx: Some(4096),
                    ..Default::default()
                },
            );
        }

        let update_auth = handle_v1_provider_update(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "custom_2".to_string(),
            }),
            hyper::body::Bytes::from(
                serde_json::to_vec(&json!({
                    "base_provider": "custom",
                    "display_name": "Work Custom",
                    "api_key": "sk-new"
                }))
                .unwrap(),
            ),
        );
        let patch_model = patch_provider_model_config(gcx.clone(), &config_dir, "custom_2");
        let (auth_result, model_result) = tokio::join!(update_auth, patch_model);
        auth_result.unwrap();
        model_result.unwrap();

        let saved = provider_config_json(&config_dir, "custom_2").await;
        assert_eq!(saved["api_key"], "sk-new");
        assert_eq!(saved["custom_models"]["my-custom"]["n_ctx"], 4096);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn provider_secret_writes_are_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            openai_api_key: "sk-secret".to_string(),
            expires_at: i64::MAX,
            ..Default::default()
        };

        save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "openai_codex",
            "openai_codex",
            &serde_yaml::to_value(&tokens).unwrap(),
        )
        .await
        .unwrap();

        let metadata = std::fs::metadata(provider_file_path(&config_dir, "openai_codex")).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn github_copilot_oauth_alias_redacts_settings() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let tokens = crate::providers::github_copilot_oauth::OAuthTokens {
            access_token: "gho-alias-secret".to_string(),
            expires_at: i64::MAX,
            ..Default::default()
        };

        save_provider_oauth_tokens(
            &gcx,
            &config_dir,
            "github_copilot_2",
            "github_copilot",
            &serde_yaml::to_value(&tokens).unwrap(),
        )
        .await
        .unwrap();

        let saved = provider_config_json(&config_dir, "github_copilot_2").await;
        assert_eq!(saved["base_provider"], "github_copilot");
        assert_eq!(saved["oauth_tokens"]["access_token"], "gho-alias-secret");
        let settings = {
            let registry = gcx.providers.read().await;
            registry
                .get("github_copilot_2")
                .unwrap()
                .provider_settings_as_json()
        };
        assert_eq!(settings["base_provider"], "github_copilot");
        assert_eq!(settings["oauth_tokens"]["access_token"], "***");
        assert!(!settings.to_string().contains("gho-alias-secret"));
    }

    #[tokio::test]
    async fn instance_aware_route_resolution_rejects_unsupported_base() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openai_2.yaml"),
            "base_provider: openai\ndisplay_name: Work OpenAI\napi_key: sk-test\n",
        )
        .await
        .unwrap();

        let err = handle_v1_provider_health(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);

        let err = handle_v1_provider_usage(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderPathParams {
                name: "openai_2".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn openrouter_model_endpoints_accepts_provider_instances() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("openrouter_2.yaml"),
            "base_provider: openrouter\ndisplay_name: Work OpenRouter\n",
        )
        .await
        .unwrap();

        let err = handle_v1_openrouter_model_endpoints(
            axum::extract::State(crate::app_state::AppState::from_gcx(gcx.clone()).await),
            Path(ProviderModelPathParams {
                name: "openrouter_2".to_string(),
                model_id: "openai/gpt-4.1".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::BAD_GATEWAY);
        assert_eq!(err.message, "OpenRouter API key is not configured");
    }

    async fn assert_remove_custom_model_clears_enabled_entry(
        provider_name: &'static str,
        provider: Box<dyn crate::providers::traits::ProviderTrait>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        {
            let mut registry = gcx.providers.write().await;
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
            let registry = gcx.providers.read().await;
            let provider = registry.get(provider_name).unwrap();
            (
                provider.enabled_models().to_vec(),
                provider.custom_models().contains_key("stale-custom"),
            )
        };
        assert!(!enabled_models.iter().any(|model| model == "stale-custom"));
        assert!(!has_custom_model);

        let content = tokio::fs::read_to_string(provider_file_path(&config_dir, provider_name))
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
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(current.clone()));
        }
        let mut source = OpenAICodexProvider::default();
        source.enabled_models = vec!["clobber-enabled".to_string()];
        source.oauth_tokens = previous_tokens.clone();
        source.session_id = previous_session_id.clone();

        let changed = sync_openai_codex_auth_state(
            gcx.clone(),
            "openai_codex",
            &source,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .unwrap();

        let stored = {
            let registry = gcx.providers.read().await;
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
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(current));
        }
        let mut source = OpenAICodexProvider::default();
        source.oauth_tokens.access_token = "stale-refresh-access".to_string();
        source.oauth_tokens.refresh_token = "stale-refresh".to_string();
        source.session_id = "stale-refresh-session".to_string();

        let changed = sync_openai_codex_auth_state(
            gcx.clone(),
            "openai_codex",
            &source,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .unwrap();

        let stored = {
            let registry = gcx.providers.read().await;
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
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(current));
        }
        let http_client = gcx.http_client.clone();

        let refreshed = force_refresh_openai_codex_usage_for_retry(
            gcx,
            &http_client,
            "openai_codex",
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
            let mut registry = gcx.providers.write().await;
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
            "openai_codex",
            &source,
            &previous_tokens,
            &previous_session_id,
        )
        .await
        .unwrap();

        let stored = {
            let registry = gcx.providers.read().await;
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
