use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationTrait;
use crate::integrations::mcp::integr_mcp_http::IntegrationMCPHttp;
use crate::integrations::mcp::session_mcp::{MCPAuthStatus, MCPConnectionStatus, SessionMCP};
use crate::providers::litellm::{LiteLLMMcpServer, LiteLLMProvider};

const PREFIX: &str = "mcp_http_llm_";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    server_id: String,
    slug: String,
    server_name: String,
    bound_identity: String,
    bound_base_url: String,
}

#[derive(Clone)]
struct Binding {
    gcx: Weak<GlobalContext>,
    instance: String,
    selection: Selection,
    url: String,
    auth_epoch: String,
}

fn auth_identity(binding: &Binding) -> String {
    format!(
        "{}:{}",
        binding.selection.bound_identity, binding.auth_epoch
    )
}

fn bindings() -> &'static Mutex<HashMap<String, Binding>> {
    static BINDINGS: OnceLock<Mutex<HashMap<String, Binding>>> = OnceLock::new();
    BINDINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn operation_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

fn error(status: StatusCode, message: &str) -> ScratchError {
    ScratchError::new(status, message.to_string())
}

fn reserved(path: &str) -> bool {
    FsPath::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(PREFIX))
}

fn slug(id: &str) -> String {
    hex::encode(Sha256::digest(id.as_bytes()))[..24].to_string()
}

fn path(gcx: &GlobalContext, instance: &str, selection: &Selection) -> PathBuf {
    gcx.config_dir
        .join("integrations.d")
        .join(format!("{PREFIX}{instance}_{}.yaml", selection.slug))
}

async fn selections(gcx: &GlobalContext, instance: &str) -> Result<Vec<Selection>, ScratchError> {
    let file = refact_providers::config_store::provider_config_path(&gcx.config_dir, instance);
    let content = match tokio::fs::read(&file).await {
        Ok(content) if content.len() <= 1024 * 1024 => content,
        Ok(_) => {
            return Err(error(
                StatusCode::CONFLICT,
                "Provider configuration is too large",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot read provider configuration",
            ))
        }
    };
    let yaml: serde_yaml::Value = serde_yaml::from_slice(&content)
        .map_err(|_| error(StatusCode::CONFLICT, "Invalid provider configuration"))?;
    let values = match yaml.get("mcp_servers") {
        Some(values) => serde_yaml::from_value::<Vec<Selection>>(values.clone())
            .map_err(|_| error(StatusCode::CONFLICT, "Invalid LiteLLM MCP selections"))?,
        None => Vec::new(),
    };
    if values.len() > 500
        || values.iter().any(|selection| {
            selection.slug != slug(&selection.server_id)
                || selection.bound_identity.len() != 64
                || refact_providers::litellm::validate_mcp_server_name(&selection.server_name)
                    .is_err()
        })
    {
        return Err(error(
            StatusCode::CONFLICT,
            "Invalid LiteLLM MCP binding identity",
        ));
    }
    Ok(values)
}

async fn save_selections(
    gcx: &GlobalContext,
    instance: &str,
    values: &[Selection],
) -> Result<(), ScratchError> {
    let value = serde_yaml::to_value(values).map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot save LiteLLM MCP selections",
        )
    })?;
    refact_providers::config_store::update_provider_config(&gcx.config_dir, instance, |existing| {
        let mut existing = existing.ok_or_else(|| "Provider no longer exists".to_string())?;
        let mapping = existing
            .as_mapping_mut()
            .ok_or_else(|| "Invalid provider configuration".to_string())?;
        mapping.insert(serde_yaml::Value::String("mcp_servers".into()), value);
        Ok(existing)
    })
    .await
    .map_err(|_| error(StatusCode::CONFLICT, "Cannot save LiteLLM MCP selections"))?;
    Ok(())
}

async fn active_binding(config_path: &str) -> Result<Option<(Binding, LiteLLMProvider)>, String> {
    let binding = bindings()
        .lock()
        .map_err(|_| "MCP binding unavailable".to_string())?
        .get(config_path)
        .cloned();
    let Some(binding) = binding else {
        return if reserved(config_path) {
            Err("Managed LiteLLM connection is disabled or untrusted".into())
        } else {
            Ok(None)
        };
    };
    let gcx = binding
        .gcx
        .upgrade()
        .ok_or_else(|| "LiteLLM provider is no longer active".to_string())?;
    let provider = super::litellm_http::resolve_provider(&gcx, &binding.instance)
        .await
        .map_err(|_| "LiteLLM provider is unavailable".to_string())?;
    if !provider.enabled
        || provider
            .identity_fingerprint(&binding.instance)
            .await
            .map_err(|_| "LiteLLM credential is unavailable".to_string())?
            != binding.selection.bound_identity
    {
        return Err("LiteLLM identity changed; reconfirm this connection".into());
    }
    if provider.validated_mcp_base_url().ok().as_deref()
        != Some(binding.selection.bound_base_url.as_str())
        || provider
            .mcp_endpoint(&binding.selection.server_name)
            .ok()
            .as_deref()
            != Some(binding.url.as_str())
    {
        return Err("LiteLLM gateway binding changed; reconfirm this connection".into());
    }
    if !selections(&gcx, &binding.instance)
        .await
        .map_err(|_| "LiteLLM selection is unavailable".to_string())?
        .iter()
        .any(|selected| {
            selected.server_id == binding.selection.server_id
                && selected.bound_identity == binding.selection.bound_identity
                && selected.server_name == binding.selection.server_name
        })
    {
        return Err("LiteLLM MCP connection is disabled".into());
    }
    let metadata = tokio::fs::symlink_metadata(config_path)
        .await
        .map_err(|_| "Managed MCP configuration is unavailable".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Managed MCP configuration is not a regular file".into());
    }
    Ok(Some((binding, provider)))
}

fn gateway_route_permitted(base_path: &str, name: &str, target_path: &str) -> bool {
    let base = base_path.trim_end_matches('/');
    let path = target_path.trim_end_matches('/');
    let mut permitted = Vec::new();
    for route in ["mcp", "register", "authorize", "token"] {
        permitted.push(format!("{base}/{name}/{route}"));
        permitted.push(format!("{base}/{route}"));
    }
    permitted.push(format!(
        "/.well-known/oauth-protected-resource{base}/{name}/mcp"
    ));
    permitted.push(format!(
        "{base}/.well-known/oauth-protected-resource/{name}/mcp"
    ));
    permitted.push(format!("{base}/.well-known/oauth-protected-resource"));
    for kind in ["oauth-authorization-server", "openid-configuration"] {
        permitted.push(format!("/.well-known/{kind}{base}"));
        permitted.push(format!("/.well-known/{kind}{base}/{name}"));
        permitted.push(format!("/.well-known/{kind}{base}/mcp"));
        permitted.push(format!("{base}/.well-known/{kind}"));
        permitted.push(format!("{base}/.well-known/{kind}/{name}"));
        permitted.push(format!("{base}/.well-known/{kind}/mcp"));
    }
    permitted.iter().any(|allowed| allowed == path)
}

pub async fn gateway_identity(config_path: &str) -> Result<Option<String>, String> {
    Ok(active_binding(config_path)
        .await?
        .map(|(binding, _)| auth_identity(&binding)))
}

pub async fn logout_gateway(config_path: &str) -> Result<bool, String> {
    let _guard = operation_lock().lock().await;
    if active_binding(config_path).await?.is_none() {
        return Ok(false);
    }
    if let Some(binding) = bindings()
        .lock()
        .map_err(|_| "MCP binding unavailable".to_string())?
        .get_mut(config_path)
    {
        binding.auth_epoch = uuid::Uuid::new_v4().to_string();
    }
    crate::integrations::mcp::mcp_auth::gateway_oauth::cancel_for_config(config_path).await;
    crate::integrations::mcp::mcp_auth::clear_tokens_from_config(config_path)
        .await
        .map_err(|_| "Cannot clear managed OAuth session".to_string())?;
    Ok(true)
}

pub async fn gateway_transport_url(config_path: &str) -> Result<Option<String>, String> {
    Ok(active_binding(config_path)
        .await?
        .map(|(binding, _)| binding.url))
}

pub async fn check_dispatch(_gcx: &Arc<GlobalContext>, config_path: &str) -> Result<(), String> {
    active_binding(config_path).await.map(|_| ())
}

pub async fn gateway_callback_url(
    config_path: &str,
    http_port: u16,
) -> Result<Option<String>, String> {
    let Some((_, provider)) = active_binding(config_path).await? else {
        return Ok(None);
    };
    provider
        .validated_callback_url(http_port)
        .map(Some)
        .map_err(|failure| failure.message)
}

pub fn gateway_is_managed(config_path: &str) -> bool {
    reserved(config_path)
}

pub async fn persist_gateway_tokens(
    config_path: &str,
    expected_identity: &str,
    expected_url: &str,
    tokens: &crate::integrations::mcp::mcp_auth::MCPOAuthTokens,
) -> Result<(), String> {
    let _guard = operation_lock().lock().await;
    let Some((binding, _)) = active_binding(config_path).await? else {
        return Err("Managed LiteLLM connection is no longer active".into());
    };
    if auth_identity(&binding) != expected_identity
        || binding.url != expected_url
        || tokens.bound_url.as_deref() != Some(expected_url)
    {
        return Err("LiteLLM identity or resource changed during authentication".into());
    }
    crate::integrations::mcp::mcp_auth::save_tokens_to_config(config_path, tokens)
        .await
        .map_err(|_| "Cannot save managed OAuth session".to_string())
}

pub async fn gateway_headers(
    config_path: &str,
    url: &str,
) -> Result<Option<HashMap<String, String>>, String> {
    let Some((binding, provider)) = active_binding(config_path).await? else {
        return Ok(None);
    };
    let base = reqwest::Url::parse(&binding.selection.bound_base_url)
        .map_err(|_| "Invalid gateway binding".to_string())?;
    let target = reqwest::Url::parse(url).map_err(|_| "Invalid gateway request URL".to_string())?;
    if !target.username().is_empty() || target.password().is_some() || target.fragment().is_some() {
        return Err("Unsafe gateway request URL".into());
    }
    if base.origin() != target.origin() {
        return Ok(Some(HashMap::new()));
    }
    let name = &binding.selection.server_name;
    if !gateway_route_permitted(base.path(), name, target.path()) {
        return Err("Gateway request outside approved MCP/OAuth routes".into());
    }
    let bearer = provider
        .resolve_bearer(&binding.instance)
        .await
        .map_err(|_| "LiteLLM credential unavailable".to_string())?;
    if provider
        .identity_fingerprint(&binding.instance)
        .await
        .map_err(|_| "LiteLLM credential unavailable".to_string())?
        != binding.selection.bound_identity
    {
        return Err("LiteLLM identity changed while preparing gateway request".into());
    }
    Ok(Some(HashMap::from([(
        "x-litellm-api-key".to_string(),
        format!("Bearer {}", bearer.expose()),
    )])))
}

async fn stop(gcx: &Arc<GlobalContext>, config_path: &str, delete: bool) {
    if let Ok(mut bindings) = bindings().lock() {
        bindings.remove(config_path);
    }
    let session = gcx.integration_sessions.lock().await.remove(config_path);
    if let Some(session) = session {
        let future = session.lock().await.try_stop(session.clone());
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(5), Box::into_pin(future)).await;
    }
    crate::integrations::mcp::integr_mcp_common::tool_catalog_cache_remove(config_path);
    crate::integrations::mcp::mcp_resources::remove_indexed_resources(
        Arc::downgrade(gcx),
        config_path.to_string(),
    )
    .await;
    if delete && reserved(config_path) {
        let owned = match tokio::fs::symlink_metadata(config_path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                tokio::fs::read(config_path)
                    .await
                    .ok()
                    .filter(|bytes| bytes.len() <= 256 * 1024)
                    .and_then(|bytes| serde_yaml::from_slice::<serde_yaml::Value>(&bytes).ok())
                    .and_then(|value| {
                        value
                            .get("litellm_managed_server_id")
                            .and_then(serde_yaml::Value::as_str)
                            .map(str::to_owned)
                    })
                    .is_some_and(|id| config_path.ends_with(&format!("_{}.yaml", slug(&id))))
            }
            _ => false,
        };
        if owned {
            let _ = tokio::fs::remove_file(config_path).await;
        }
    }
    gcx.tool_catalog_generations.advance_mcp();
}

async fn ensure_file(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    selection: &Selection,
    url: &str,
) -> Result<String, ScratchError> {
    let file = path(gcx, instance, selection);
    let config_path = file.to_string_lossy().to_string();
    let existing = match tokio::fs::symlink_metadata(&file).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(error(
                StatusCode::CONFLICT,
                "Managed MCP path is not a regular file",
            ))
        }
        Ok(_) => {
            let bytes = tokio::fs::read(&file).await.map_err(|_| {
                error(
                    StatusCode::CONFLICT,
                    "Cannot read managed MCP configuration",
                )
            })?;
            if bytes.len() > 256 * 1024 {
                return Err(error(
                    StatusCode::CONFLICT,
                    "Managed MCP configuration is too large",
                ));
            }
            Some(
                serde_yaml::from_slice::<serde_yaml::Value>(&bytes).map_err(|_| {
                    error(StatusCode::CONFLICT, "Invalid managed MCP configuration")
                })?,
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return Err(error(
                StatusCode::CONFLICT,
                "Cannot inspect managed MCP configuration",
            ))
        }
    };
    if existing.as_ref().is_some_and(|value| {
        value
            .get("litellm_managed_server_id")
            .and_then(serde_yaml::Value::as_str)
            != Some(selection.server_id.as_str())
    }) {
        return Err(error(
            StatusCode::CONFLICT,
            "Managed MCP path is already owned by another configuration",
        ));
    }
    let mut value = existing.unwrap_or_else(|| serde_yaml::to_value(json!({
        "url": url, "auth_type":"none", "init_timeout":"15", "request_timeout":"30", "reconnect_enabled":false,
        "available":{"on_your_laptop":true,"when_isolated":true},
        "confirmation":{"ask_user":["*"],"deny":[]}, "litellm_managed_server_id": selection.server_id,
    })).expect("static MCP configuration"));
    value
        .as_mapping_mut()
        .ok_or_else(|| error(StatusCode::CONFLICT, "Invalid managed configuration"))?
        .insert(
            serde_yaml::Value::String("litellm_managed_provider".into()),
            serde_yaml::Value::String(instance.into()),
        );
    if value.get("url").and_then(serde_yaml::Value::as_str) != Some(url) {
        value
            .as_mapping_mut()
            .ok_or_else(|| error(StatusCode::CONFLICT, "Invalid managed configuration"))?
            .insert(
                serde_yaml::Value::String("url".into()),
                serde_yaml::Value::String(url.into()),
            );
    }
    let directory = file
        .parent()
        .ok_or_else(|| error(StatusCode::CONFLICT, "Invalid managed MCP path"))?;
    tokio::fs::create_dir_all(directory).await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot create integration directory",
        )
    })?;
    let temporary = file.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_yaml::to_string(&value).map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot serialize managed MCP configuration",
        )
    })?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut handle = options.open(&temporary).await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot save managed MCP configuration",
        )
    })?;
    use tokio::io::AsyncWriteExt;
    handle.write_all(bytes.as_bytes()).await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot save managed MCP configuration",
        )
    })?;
    drop(handle);
    tokio::fs::rename(&temporary, &file).await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot publish managed MCP configuration",
        )
    })?;
    Ok(config_path)
}

async fn start(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    selection: &Selection,
    provider: &LiteLLMProvider,
) -> Result<(), ScratchError> {
    let url = provider
        .mcp_endpoint(&selection.server_name)
        .map_err(|_| error(StatusCode::CONFLICT, "Invalid gateway MCP route"))?;
    let config_path = ensure_file(gcx, instance, selection, &url).await?;
    {
        let mut registry = bindings().lock().map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "MCP bindings unavailable",
            )
        })?;
        let auth_epoch = registry
            .get(&config_path)
            .filter(|old| {
                old.gcx.ptr_eq(&Arc::downgrade(gcx))
                    && old.instance == instance
                    && old.selection.server_id == selection.server_id
                    && old.selection.bound_identity == selection.bound_identity
                    && old.url == url
            })
            .map(|old| old.auth_epoch.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        registry.insert(
            config_path.clone(),
            Binding {
                gcx: Arc::downgrade(gcx),
                instance: instance.into(),
                selection: selection.clone(),
                url,
                auth_epoch,
            },
        );
    }
    let bytes = tokio::fs::read(&config_path).await.map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "Cannot read managed MCP configuration",
        )
    })?;
    let yaml: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|_| error(StatusCode::CONFLICT, "Invalid managed MCP configuration"))?;
    let value = serde_json::to_value(yaml)
        .map_err(|_| error(StatusCode::CONFLICT, "Invalid managed MCP configuration"))?;
    IntegrationMCPHttp::default()
        .integr_settings_apply(gcx.clone(), config_path, &value)
        .await
        .map_err(|_| error(StatusCode::CONFLICT, "Cannot start managed MCP integration"))?;
    Ok(())
}

async fn reconcile(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    provider: &LiteLLMProvider,
) -> Result<bool, ScratchError> {
    let selected = selections(gcx, instance).await?;
    if selected.is_empty() {
        return Ok(false);
    }
    let identity = provider.identity_fingerprint(instance).await.ok();
    let base = provider.validated_mcp_base_url().ok();
    let mut reconfirm = false;
    for selection in selected {
        let file = path(gcx, instance, &selection)
            .to_string_lossy()
            .to_string();
        if !provider.enabled
            || identity.as_deref() != Some(&selection.bound_identity)
            || base.as_deref() != Some(&selection.bound_base_url)
        {
            stop(gcx, &file, identity.is_some() && base.is_some()).await;
            reconfirm = true;
            continue;
        }
    }
    if provider.enabled {
        inventory(gcx, instance, provider).await?;
    }
    Ok(reconfirm)
}

pub async fn on_provider_saved(gcx: Arc<GlobalContext>, instance: &str) {
    let _guard = operation_lock().lock().await;
    if let Ok(provider) = super::litellm_http::resolve_provider(&gcx, instance).await {
        if reconcile(&gcx, instance, &provider).await.is_err() {
            tracing::warn!("LiteLLM MCP reconciliation failed");
        }
    }
}

pub async fn on_provider_deleted(gcx: Arc<GlobalContext>, instance: &str) {
    let _guard = operation_lock().lock().await;
    let paths = bindings()
        .lock()
        .ok()
        .map(|bindings| {
            bindings
                .iter()
                .filter(|(_, binding)| {
                    binding.instance == instance
                        && binding
                            .gcx
                            .upgrade()
                            .is_some_and(|owner| Arc::ptr_eq(&owner, &gcx))
                })
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for path in paths {
        stop(&gcx, &path, true).await;
    }
    if let Ok(mut entries) = tokio::fs::read_dir(gcx.config_dir.join("integrations.d")).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file = entry.path();
            if !reserved(&file.to_string_lossy()) {
                continue;
            }
            let Ok(metadata) = entry.file_type().await else {
                continue;
            };
            if metadata.is_symlink() || !metadata.is_file() {
                continue;
            }
            let Ok(bytes) = tokio::fs::read(&file).await else {
                continue;
            };
            if bytes.len() > 256 * 1024 {
                continue;
            }
            let Ok(value) = serde_yaml::from_slice::<serde_yaml::Value>(&bytes) else {
                continue;
            };
            if value
                .get("litellm_managed_provider")
                .and_then(serde_yaml::Value::as_str)
                == Some(instance)
            {
                stop(&gcx, &file.to_string_lossy(), true).await;
            }
        }
    }
}

pub async fn reconcile_all(gcx: Arc<GlobalContext>) {
    let names = {
        let registry = gcx.providers.read().await;
        registry
            .iter()
            .filter(|(_, provider)| provider.base_provider_name() == "litellm")
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>()
    };
    for name in names {
        on_provider_saved(gcx.clone(), &name).await;
    }
}

async fn server_view(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    server: &LiteLLMMcpServer,
    selection: Option<&Selection>,
    current_identity: Option<&str>,
) -> Value {
    let reconfirm = selection
        .is_some_and(|selected| Some(selected.bound_identity.as_str()) != current_identity);
    let config_path =
        selection.map(|selection| path(gcx, instance, selection).to_string_lossy().to_string());
    let session = if let Some(path) = &config_path {
        gcx.integration_sessions.lock().await.get(path).cloned()
    } else {
        None
    };
    let mut connection = if selection.is_none() {
        "disabled"
    } else if reconfirm {
        "reconfirm_required"
    } else {
        "connecting"
    };
    let mut connected = false;
    let mut tools_count = None;
    let oauth = server
        .auth_type
        .as_deref()
        .is_some_and(|auth| auth.contains("oauth"));
    let mut auth_state = "unknown";
    if let Some(session) = session {
        let mut session = session.lock().await;
        if let Some(session) = session.as_any_mut().downcast_mut::<SessionMCP>() {
            if !reconfirm {
                connection = match session.connection_status {
                    MCPConnectionStatus::Connected => "connected",
                    MCPConnectionStatus::NeedsAuth => "needs_auth",
                    MCPConnectionStatus::Failed {
                        ..
                    }
                    | MCPConnectionStatus::Disconnected => "failed",
                    _ => "connecting",
                };
                connected = session.connection_status == MCPConnectionStatus::Connected;
                tools_count = Some(session.mcp_tools.len());
            }
            auth_state = match session.auth_status {
                MCPAuthStatus::Authenticated => "ok",
                MCPAuthStatus::NeedsLogin | MCPAuthStatus::NeedsReauth => "auth_required",
                _ if session.connection_status == MCPConnectionStatus::NeedsAuth => "auth_required",
                _ if connected => "ok",
                _ => auth_state,
            };
        }
    }
    json!({"server_id":server.id,"display_name":server.display_name.as_ref().or(server.alias.as_ref()).unwrap_or(&server.name),"description":server.description,"transport":server.transport,
        "auth":{"kind":if oauth {"oauth"} else if server.auth_type.as_deref() == Some("none") {"none"} else if server.auth_type.is_some() {"api_key"} else {"unknown"},"state":auth_state},
        "health":"unknown","access":if connected {"allowed"} else {"unknown"},"enabled":selection.is_some(),"connected":connected,"connection":connection,"integration_path":config_path,"tools_count":tools_count,"removed":false})
}

async fn inventory(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    provider: &LiteLLMProvider,
) -> Result<Value, ScratchError> {
    let before = provider.identity_fingerprint(instance).await.ok();
    let inventory = provider.fetch_mcp_inventory(instance).await;
    let selected = selections(gcx, instance).await?;
    let identity = provider.identity_fingerprint(instance).await.ok();
    let current = super::litellm_http::resolve_provider(gcx, instance).await?;
    if before != identity || identity != current.identity_fingerprint(instance).await.ok() {
        return Ok(
            json!({"state":"unavailable","message":"LiteLLM identity changed while loading MCP servers; rescan to retry","fetched_at":chrono::Utc::now().to_rfc3339(),"stale":false,"reconfirm_required":true,"servers":[]}),
        );
    }
    let mut servers = Vec::new();
    for server in &inventory.servers {
        let selection = selected
            .iter()
            .find(|selected| selected.server_id == server.id);
        if let Some(selection) = selection {
            let config_path = path(gcx, instance, selection).to_string_lossy().to_string();
            if selection.server_name != server.name {
                stop(gcx, &config_path, true).await;
                let mut view = server_view(gcx, instance, server, Some(selection), None).await;
                view["connection"] = json!("reconfirm_required");
                servers.push(view);
                continue;
            }
            if provider.enabled && identity.as_deref() == Some(selection.bound_identity.as_str()) {
                start(gcx, instance, selection, provider).await?;
            }
        }
        servers.push(server_view(gcx, instance, server, selection, identity.as_deref()).await);
    }
    for selection in &selected {
        if inventory
            .servers
            .iter()
            .any(|server| server.id == selection.server_id)
        {
            continue;
        }
        let removed = inventory.state == crate::providers::litellm::LiteLLMFeatureState::Ok;
        if removed
            || matches!(
                inventory.state,
                crate::providers::litellm::LiteLLMFeatureState::PermissionDenied
                    | crate::providers::litellm::LiteLLMFeatureState::AuthRequired
            )
        {
            stop(
                gcx,
                &path(gcx, instance, selection).to_string_lossy(),
                removed,
            )
            .await;
        }
        servers.push(unavailable_server_view(
            selection,
            identity.as_deref(),
            removed,
        ));
    }
    let reconfirm_required = selected
        .iter()
        .any(|selected| Some(selected.bound_identity.as_str()) != identity.as_deref());
    let status_ids = selected
        .iter()
        .filter(|selected| Some(selected.bound_identity.as_str()) == identity.as_deref())
        .take(16)
        .map(|selected| selected.server_id.clone())
        .collect::<Vec<_>>();
    if !status_ids.is_empty() {
        let statuses = provider.fetch_mcp_status(instance, &status_ids).await;
        for status in statuses.servers {
            if let Some(server) = servers.iter_mut().find(|server| {
                server.get("server_id").and_then(Value::as_str) == Some(status.server_id.as_str())
            }) {
                server["health"] =
                    serde_json::to_value(status.health).unwrap_or_else(|_| json!("unknown"));
                if status.missing_setup == Some(true) {
                    server["auth"]["state"] = json!("missing_setup");
                    server["auth"]["setup_url"] =
                        serde_json::to_value(status.setup_url).unwrap_or(Value::Null);
                } else if server["auth"]["kind"] == "oauth" {
                    let native_state = serde_json::to_value(status.oauth_state)
                        .unwrap_or_else(|_| json!("unknown"));
                    apply_oauth_status(server, native_state.as_str().unwrap_or("unknown"));
                }
            }
        }
    }
    let current = super::litellm_http::resolve_provider(gcx, instance).await?;
    if identity != current.identity_fingerprint(instance).await.ok() {
        return Ok(
            json!({"state":"unavailable","message":"LiteLLM identity changed while loading MCP status; rescan to retry","fetched_at":chrono::Utc::now().to_rfc3339(),"stale":false,"reconfirm_required":true,"servers":[]}),
        );
    }
    Ok(
        json!({"state":inventory.state,"message":inventory.message,"fetched_at":inventory.fetched_at,"stale":inventory.state != crate::providers::litellm::LiteLLMFeatureState::Ok,"reconfirm_required":reconfirm_required,"servers":servers}),
    )
}

fn unavailable_server_view(selection: &Selection, identity: Option<&str>, removed: bool) -> Value {
    let reconfirm = identity != Some(selection.bound_identity.as_str());
    json!({"server_id":selection.server_id,"display_name":selection.server_name,"auth":{"kind":"unknown","state":"unknown"},"health":"unknown","access":"unknown","enabled":true,"connected":false,"connection":if reconfirm {"reconfirm_required"} else {"failed"},"removed":removed})
}

fn apply_oauth_status(server: &mut Value, native: &str) {
    match native {
        "connected" => server["auth"]["state"] = json!("ok"),
        "auth_required" if server["connected"] != true => {
            server["auth"]["state"] = json!("auth_required")
        }
        _ => {}
    }
    let check = match native {
        "connected" => "ok",
        "auth_required" | "unsupported" | "permission_denied" => native,
        _ => return,
    };
    server["auth"]["status_check_state"] = json!(check);
}

#[derive(Deserialize)]
pub struct LiteLLMMcpPath {
    name: String,
    server_id: Option<String>,
}

pub async fn handle_litellm_mcp_get(
    State(app): State<AppState>,
    Path(params): Path<LiteLLMMcpPath>,
) -> Result<Json<Value>, ScratchError> {
    let _guard = operation_lock().lock().await;
    let provider = super::litellm_http::resolve_provider(&app.gcx, &params.name).await?;
    let identity = provider.identity_fingerprint(&params.name).await.ok();
    for selection in selections(&app.gcx, &params.name).await? {
        if !provider.enabled || identity.as_deref() != Some(selection.bound_identity.as_str()) {
            stop(
                &app.gcx,
                &path(&app.gcx, &params.name, &selection).to_string_lossy(),
                identity.is_some(),
            )
            .await;
        }
    }
    inventory(&app.gcx, &params.name, &provider).await.map(Json)
}

pub async fn handle_litellm_mcp_rescan(
    state: State<AppState>,
    path: Path<LiteLLMMcpPath>,
) -> Result<Json<Value>, ScratchError> {
    handle_litellm_mcp_get(state, path).await
}

pub async fn handle_litellm_mcp_enable(
    State(app): State<AppState>,
    Path(params): Path<LiteLLMMcpPath>,
) -> Result<Json<Value>, ScratchError> {
    let _guard = operation_lock().lock().await;
    let provider = super::litellm_http::resolve_provider(&app.gcx, &params.name).await?;
    if !provider.enabled {
        return Err(error(
            StatusCode::CONFLICT,
            "Enable the LiteLLM provider first",
        ));
    }
    let server_id = params
        .server_id
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Missing MCP server identity"))?;
    let before = provider
        .identity_fingerprint(&params.name)
        .await
        .map_err(|_| error(StatusCode::CONFLICT, "LiteLLM credential unavailable"))?;
    let remote = provider.fetch_mcp_inventory(&params.name).await;
    if remote.state != crate::providers::litellm::LiteLLMFeatureState::Ok {
        return Err(error(
            StatusCode::CONFLICT,
            "Cannot verify LiteLLM MCP access",
        ));
    }
    let server = remote
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "MCP server not visible to this key"))?;
    let identity = provider
        .identity_fingerprint(&params.name)
        .await
        .map_err(|_| error(StatusCode::CONFLICT, "LiteLLM credential unavailable"))?;
    if identity != before {
        return Err(error(
            StatusCode::CONFLICT,
            "LiteLLM identity changed; rescan before enabling",
        ));
    }
    let current = super::litellm_http::resolve_provider(&app.gcx, &params.name).await?;
    if current
        .identity_fingerprint(&params.name)
        .await
        .ok()
        .as_deref()
        != Some(identity.as_str())
    {
        return Err(error(StatusCode::CONFLICT, "LiteLLM identity changed"));
    }
    let provider = current;
    let selection = Selection {
        server_id,
        slug: slug(&server.id),
        server_name: server.name.clone(),
        bound_identity: identity,
        bound_base_url: provider
            .validated_mcp_base_url()
            .map_err(|_| error(StatusCode::CONFLICT, "Invalid LiteLLM endpoint"))?,
    };
    let mut selected = selections(&app.gcx, &params.name).await?;
    if let Some(old) = selected
        .iter()
        .find(|selected| selected.server_id == selection.server_id)
    {
        stop(
            &app.gcx,
            &path(&app.gcx, &params.name, old).to_string_lossy(),
            true,
        )
        .await;
    }
    selected.retain(|selected| selected.server_id != selection.server_id);
    selected.push(selection.clone());
    save_selections(&app.gcx, &params.name, &selected).await?;
    start(&app.gcx, &params.name, &selection, &provider).await?;
    inventory(&app.gcx, &params.name, &provider).await.map(Json)
}

pub async fn handle_litellm_mcp_disable(
    State(app): State<AppState>,
    Path(params): Path<LiteLLMMcpPath>,
) -> Result<Json<Value>, ScratchError> {
    let _guard = operation_lock().lock().await;
    let provider = super::litellm_http::resolve_provider(&app.gcx, &params.name).await?;
    let server_id = params
        .server_id
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Missing MCP server identity"))?;
    let mut selected = selections(&app.gcx, &params.name).await?;
    let old = selected
        .iter()
        .find(|selected| selected.server_id == server_id)
        .cloned();
    selected.retain(|selected| selected.server_id != server_id);
    save_selections(&app.gcx, &params.name, &selected).await?;
    if let Some(old) = old {
        stop(
            &app.gcx,
            &path(&app.gcx, &params.name, &old).to_string_lossy(),
            true,
        )
        .await;
    }
    inventory(&app.gcx, &params.name, &provider).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::instance::ProviderInstance;

    async fn binding_fixture() -> (Arc<GlobalContext>, String, Selection) {
        binding_fixture_at("https://gateway.example/proxy").await
    }

    async fn binding_fixture_at(endpoint: &str) -> (Arc<GlobalContext>, String, Selection) {
        binding_fixture_with_gateway(endpoint, None).await
    }

    async fn binding_fixture_with_gateway(
        endpoint: &str,
        gateway: Option<&str>,
    ) -> (Arc<GlobalContext>, String, Selection) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let provider = LiteLLMProvider {
            endpoint: endpoint.into(),
            mcp_gateway_url: gateway.map(str::to_owned),
            api_key: "private-provider-key".into(),
            enabled: true,
            ..Default::default()
        };
        let selection = Selection {
            server_id: "immutable-id".into(),
            slug: slug("immutable-id"),
            server_name: "Slack".into(),
            bound_identity: provider.identity_fingerprint("work").await.unwrap(),
            bound_base_url: provider.validated_mcp_base_url().unwrap(),
        };
        refact_providers::config_store::write_provider_config(&gcx.config_dir, "work", serde_yaml::to_value(json!({"base_provider":"litellm","api_key":"private-provider-key","endpoint":provider.endpoint,"mcp_gateway_url":provider.mcp_gateway_url,"enabled":true})).unwrap()).await.unwrap();
        save_selections(&gcx, "work", &[selection.clone()])
            .await
            .unwrap();
        let url = provider.mcp_endpoint("Slack").unwrap();
        let file = ensure_file(&gcx, "work", &selection, &url).await.unwrap();
        gcx.providers
            .write()
            .await
            .add(Box::new(ProviderInstance::new(
                "work",
                "litellm",
                "Work",
                Box::new(provider),
            )));
        bindings().lock().unwrap().insert(
            file.clone(),
            Binding {
                gcx: Arc::downgrade(&gcx),
                instance: "work".into(),
                selection: selection.clone(),
                url,
                auth_epoch: "epoch-1".into(),
            },
        );
        (gcx, file, selection)
    }

    #[tokio::test]
    async fn gateway_secret_is_not_persisted_or_sent_off_origin() {
        let (gcx, file, _) = binding_fixture().await;
        let headers = gateway_headers(&file, "https://gateway.example/proxy/Slack/mcp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(headers["x-litellm-api-key"], "Bearer private-provider-key");
        assert!(gateway_headers(&file, "https://idp.example/token")
            .await
            .unwrap()
            .unwrap()
            .is_empty());
        assert!(
            gateway_headers(&file, "https://gateway.example/proxy/other/mcp")
                .await
                .is_err()
        );
        assert!(!tokio::fs::read_to_string(&file)
            .await
            .unwrap()
            .contains("private-provider-key"));
        stop(&gcx, &file, true).await;
        assert!(
            gateway_headers(&file, "https://gateway.example/proxy/Slack/mcp")
                .await
                .is_err()
        );
        assert!(!FsPath::new(&file).exists());
    }

    #[tokio::test]
    async fn forged_project_binding_and_rotation_cannot_borrow_provider_credential() {
        let (gcx, file, selection) = binding_fixture().await;
        let foreign = gcx
            .config_dir
            .join("project")
            .join(FsPath::new(&file).file_name().unwrap());
        tokio::fs::create_dir_all(foreign.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::copy(&file, &foreign).await.unwrap();
        assert!(gateway_headers(
            &foreign.to_string_lossy(),
            "https://gateway.example/proxy/Slack/mcp"
        )
        .await
        .is_err());
        let changed = LiteLLMProvider {
            endpoint: "https://gateway.example/proxy".into(),
            api_key: "rotated".into(),
            enabled: true,
            ..Default::default()
        };
        gcx.providers
            .write()
            .await
            .add(Box::new(ProviderInstance::new(
                "work",
                "litellm",
                "Renamed display",
                Box::new(changed),
            )));
        assert!(
            gateway_headers(&file, "https://gateway.example/proxy/Slack/mcp")
                .await
                .is_err()
        );
        assert_eq!(path(&gcx, "work", &selection).to_string_lossy(), file);
        on_provider_saved(gcx.clone(), "work").await;
        assert!(!FsPath::new(&file).exists());
        assert_eq!(selections(&gcx, "work").await.unwrap().len(), 1);
        assert!(foreign.exists());
    }

    #[tokio::test]
    async fn manual_config_is_not_deleted_by_managed_cleanup() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let file = gcx
            .config_dir
            .join("integrations.d")
            .join(format!("{PREFIX}work_{}.yaml", slug("manual")));
        tokio::fs::create_dir_all(file.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&file, "url: https://manual.example/mcp\n")
            .await
            .unwrap();
        stop(&gcx, &file.to_string_lossy(), true).await;
        assert_eq!(
            tokio::fs::read_to_string(file).await.unwrap(),
            "url: https://manual.example/mcp\n"
        );
    }

    #[tokio::test]
    async fn stale_or_wrong_resource_oauth_cannot_be_persisted_after_disable() {
        let (gcx, file, _) = binding_fixture().await;
        let identity = gateway_identity(&file).await.unwrap().unwrap();
        let expected_url = "https://gateway.example/proxy/Slack/mcp";
        assert!(
            crate::integrations::mcp::mcp_auth::gateway_oauth::revalidate_binding(
                &file,
                &identity,
                "https://gateway.example/proxy/Other/mcp"
            )
            .await
            .is_err()
        );
        crate::integrations::mcp::mcp_auth::gateway_oauth::revalidate_binding(
            &file,
            &identity,
            expected_url,
        )
        .await
        .unwrap();
        let mut tokens = crate::integrations::mcp::mcp_auth::MCPOAuthTokens {
            access_token: "oauth-access".into(),
            refresh_token: "oauth-refresh".into(),
            expires_at: 0,
            client_id: "client".into(),
            client_secret: None,
            scopes: Vec::new(),
            bound_url: Some("https://other.example/mcp".into()),
        };
        assert!(
            persist_gateway_tokens(&file, &identity, expected_url, &tokens)
                .await
                .is_err()
        );
        tokens.bound_url = Some(expected_url.into());
        persist_gateway_tokens(&file, &identity, expected_url, &tokens)
            .await
            .unwrap();
        assert!(tokio::fs::read_to_string(&file)
            .await
            .unwrap()
            .contains("oauth-access"));
        stop(&gcx, &file, true).await;
        assert!(
            persist_gateway_tokens(&file, &identity, expected_url, &tokens)
                .await
                .is_err()
        );
        assert!(!FsPath::new(&file).exists());
    }

    fn sample_tokens(url: &str) -> crate::integrations::mcp::mcp_auth::MCPOAuthTokens {
        crate::integrations::mcp::mcp_auth::MCPOAuthTokens {
            access_token: "oauth-access".into(),
            refresh_token: "oauth-refresh".into(),
            expires_at: 0,
            client_id: "client".into(),
            client_secret: None,
            scopes: Vec::new(),
            bound_url: Some(url.into()),
        }
    }

    #[tokio::test]
    async fn managed_logout_fences_stale_flows_and_clears_tokens() {
        let (_gcx, file, _) = binding_fixture().await;
        let url = "https://gateway.example/proxy/Slack/mcp";
        let old_identity = gateway_identity(&file).await.unwrap().unwrap();
        let tokens = sample_tokens(url);
        persist_gateway_tokens(&file, &old_identity, url, &tokens)
            .await
            .unwrap();
        assert!(logout_gateway(&file).await.unwrap());
        assert!(
            crate::integrations::mcp::mcp_auth::load_tokens_from_config(&file)
                .await
                .is_none()
        );
        assert!(
            persist_gateway_tokens(&file, &old_identity, url, &tokens)
                .await
                .is_err(),
            "stale refresh/code flow must not resurrect the session"
        );
        let new_identity = gateway_identity(&file).await.unwrap().unwrap();
        assert_ne!(new_identity, old_identity);
        persist_gateway_tokens(&file, &new_identity, url, &tokens)
            .await
            .unwrap();
        assert!(!logout_gateway("/tmp/not-managed.yaml").await.unwrap());
    }

    #[test]
    fn native_status_read_states_are_preserved_without_inferring_access() {
        let base =
            json!({"auth":{"kind":"oauth","state":"unknown"},"connected":false,"access":"unknown"});
        for (native, check, state) in [
            ("connected", Some("ok"), "ok"),
            ("auth_required", Some("auth_required"), "auth_required"),
            ("permission_denied", Some("permission_denied"), "unknown"),
            ("unsupported", Some("unsupported"), "unknown"),
            ("unknown", None, "unknown"),
        ] {
            let mut server = base.clone();
            apply_oauth_status(&mut server, native);
            assert_eq!(server["auth"]["state"], state, "{native}");
            assert_eq!(
                server["auth"]
                    .get("status_check_state")
                    .and_then(Value::as_str),
                check,
                "{native}"
            );
            assert_eq!(server["access"], "unknown");
        }
    }

    #[tokio::test]
    async fn inventory_denial_is_not_an_identity_change_and_retains_oauth() {
        let (base, _) = serve_gateway(|_, path| match path {
            "/proxy/v1/mcp/server" => (403, "{}".into()),
            _ => (404, "{}".into()),
        })
        .await;
        let (gcx, file, selection) = binding_fixture_at(&format!("{base}/proxy")).await;
        let identity = gateway_identity(&file).await.unwrap().unwrap();
        let url = format!("{base}/proxy/Slack/mcp");
        persist_gateway_tokens(&file, &identity, &url, &sample_tokens(&url))
            .await
            .unwrap();
        let provider = super::super::litellm_http::resolve_provider(&gcx, "work")
            .await
            .unwrap();
        let result = inventory(&gcx, "work", &provider).await.unwrap();
        assert_eq!(result["state"], "permission_denied");
        assert_eq!(result["reconfirm_required"], false);
        let view = &result["servers"][0];
        assert_eq!(view["connection"], "failed");
        assert_eq!(view["removed"], false);
        assert_eq!(view["access"], "unknown");
        assert!(
            crate::integrations::mcp::mcp_auth::load_tokens_from_config(&file)
                .await
                .is_some()
        );
        assert!(check_dispatch(&gcx, &file).await.is_err());
        assert_eq!(
            unavailable_server_view(&selection, Some("changed"), false)["connection"],
            "reconfirm_required"
        );
        stop(&gcx, &file, true).await;
    }

    #[test]
    fn credential_routes_are_exact_for_ingress_metadata() {
        let allowed = |p: &str| gateway_route_permitted("/proxy", "Slack", p);
        for ok in [
            "/proxy/Slack/mcp",
            "/proxy/register",
            "/.well-known/oauth-protected-resource/proxy/Slack/mcp",
            "/proxy/.well-known/oauth-protected-resource/Slack/mcp",
            "/proxy/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server/proxy",
            "/.well-known/oauth-authorization-server/proxy/mcp",
            "/proxy/.well-known/openid-configuration/mcp",
            "/.well-known/openid-configuration/proxy/Slack",
            "/proxy/.well-known/oauth-authorization-server",
        ] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server",
            "/.well-known/oauth-protected-resource/proxy/Other/mcp",
            "/.well-known/oauth-protected-resource/proxy/Slack/mcp/x",
            "/.well-known/oauth-authorization-server/proxy/Other",
            "/.well-known/oauth-authorization-server/proxy2",
            "/.well-known/oauth-authorization-server/proxy/mcp/Other",
            "/proxy/.well-known/openid-configuration/mcpx",
            "/proxy2/Slack/mcp",
            "/proxy/Slack/mcpx",
            "/proxy/Other/token",
            "/Slack/mcp",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
    }

    async fn serve_gateway(
        handler: fn(&str, &str) -> (u16, String),
    ) -> (String, Arc<Mutex<Vec<(String, Option<String>)>>>) {
        use axum::extract::State as AxState;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let base_for = base.clone();
        let app = axum::Router::new()
            .fallback(
                move |AxState(seen): AxState<Arc<Mutex<Vec<(String, Option<String>)>>>>,
                      uri: axum::http::Uri,
                      headers: axum::http::HeaderMap| {
                    let base = base_for.clone();
                    async move {
                        let key = headers
                            .get("x-litellm-api-key")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.lock().unwrap().push((uri.path().to_string(), key));
                        let (status, body) = handler(&base, uri.path());
                        axum::response::Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap()
                    }
                },
            )
            .with_state(seen.clone());
        tokio::spawn(
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service()),
        );
        (base, seen)
    }

    #[tokio::test]
    async fn ingress_oauth_discovery_works_with_registry_resolver_and_exact_scope() {
        use crate::integrations::mcp::mcp_auth::gateway_oauth::{
            discover, registry_resolver, ScopedSender,
        };
        let (base, seen) = serve_gateway(|base, path| match path {
            "/.well-known/oauth-protected-resource/proxy/Slack/mcp" => (
                200,
                format!(r#"{{"resource":"{base}/proxy/Slack/mcp","authorization_servers":["{base}/proxy"]}}"#),
            ),
            "/.well-known/oauth-authorization-server/proxy" => (
                200,
                format!(r#"{{"issuer":"{base}/proxy","authorization_endpoint":"{base}/proxy/Slack/authorize","token_endpoint":"{base}/proxy/Slack/token","registration_endpoint":"{base}/proxy/Slack/register","code_challenge_methods_supported":["S256"]}}"#),
            ),
            _ => (404, "{}".to_string()),
        })
        .await;
        let (gcx, file, _) = binding_fixture_at(&format!("{base}/proxy")).await;
        let url = format!("{base}/proxy/Slack/mcp");
        let resource = url::Url::parse(&url).unwrap();
        let sender = ScopedSender::new(&url, registry_resolver(&file)).unwrap();
        let metadata = discover(&sender, &resource).await.unwrap();
        assert_eq!(metadata.token_endpoint, format!("{base}/proxy/Slack/token"));
        let seen_now = seen.lock().unwrap().clone();
        assert!(!seen_now.is_empty());
        for (path, key) in &seen_now {
            assert_eq!(
                key.as_deref(),
                Some("Bearer private-provider-key"),
                "{path}"
            );
        }

        for bad in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/proxy/Other/mcp",
            "/.well-known/oauth-authorization-server",
            "/proxy2/Slack/mcp",
        ] {
            let before = seen.lock().unwrap().len();
            assert!(
                gateway_headers(&file, &format!("{base}{bad}"))
                    .await
                    .is_err(),
                "{bad}"
            );
            assert_eq!(seen.lock().unwrap().len(), before);
        }
        assert!(gateway_headers(
            &file,
            "https://external.example/.well-known/oauth-protected-resource"
        )
        .await
        .unwrap()
        .unwrap()
        .is_empty());
        stop(&gcx, &file, true).await;
    }

    #[tokio::test]
    async fn explicit_mcp_gateway_discovery_confines_credentials_and_revokes_changed_binding() {
        use crate::integrations::mcp::mcp_auth::gateway_oauth::{
            discover, registry_resolver, ScopedSender,
        };
        let (base, seen) = serve_gateway(|base, path| match path {
            "/.well-known/oauth-protected-resource/proxy/Slack/mcp" => (200, format!(r#"{{"resource":"{base}/proxy/Slack/mcp","authorization_servers":["{base}/proxy/mcp"]}}"#)),
            "/.well-known/oauth-authorization-server/proxy/mcp" => (200, format!(r#"{{"issuer":"{base}/proxy/mcp","authorization_endpoint":"{base}/proxy/authorize/mcp-session","token_endpoint":"{base}/proxy/token","registration_endpoint":"{base}/proxy/register","code_challenge_methods_supported":["S256"]}}"#)),
            _ => (404, "{}".into()),
        }).await;
        let gateway = format!("{base}/proxy");
        let api = "https://api.example";
        let (gcx, file, selection) = binding_fixture_with_gateway(api, Some(&gateway)).await;
        assert_eq!(selection.bound_base_url, gateway);
        let url = format!("{gateway}/Slack/mcp");
        assert_eq!(
            gateway_transport_url(&file).await.unwrap(),
            Some(url.clone())
        );
        let sender = ScopedSender::new(&url, registry_resolver(&file)).unwrap();
        let metadata = discover(&sender, &url::Url::parse(&url).unwrap())
            .await
            .unwrap();
        assert_eq!(metadata.token_endpoint, format!("{gateway}/token"));
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|(_, key)| key.as_deref() == Some("Bearer private-provider-key")));
        assert!(gateway_headers(&file, &format!("{api}/proxy/token"))
            .await
            .unwrap()
            .unwrap()
            .is_empty());
        assert!(gateway_headers(&file, &format!("{gateway}/Other/mcp"))
            .await
            .is_err());
        assert!(!tokio::fs::read_to_string(&file)
            .await
            .unwrap()
            .contains("private-provider-key"));
        let mut changed = super::super::litellm_http::resolve_provider(&gcx, "work")
            .await
            .unwrap();
        changed.mcp_gateway_url = Some(format!("{base}/new-ingress"));
        gcx.providers
            .write()
            .await
            .add(Box::new(ProviderInstance::new(
                "work",
                "litellm",
                "Work",
                Box::new(changed),
            )));
        assert_eq!(
            gateway_headers(&file, &url).await.unwrap_err(),
            "LiteLLM identity changed; reconfirm this connection"
        );
        assert!(check_dispatch(&gcx, &file).await.is_err());
        assert_eq!(seen.lock().unwrap().len(), requests.len());
        stop(&gcx, &file, true).await;
    }

    #[tokio::test]
    async fn ingress_discovery_rejects_out_of_scope_issuer_without_extra_requests() {
        use crate::integrations::mcp::mcp_auth::gateway_oauth::{
            discover, registry_resolver, ScopedSender,
        };
        let (base, seen) = serve_gateway(|base, path| match path {
            "/.well-known/oauth-protected-resource/proxy/Slack/mcp" => (
                200,
                format!(r#"{{"resource":"{base}/proxy/Slack/mcp","authorization_servers":["{base}/other"]}}"#),
            ),
            _ => (404, "{}".to_string()),
        })
        .await;
        let (gcx, file, _) = binding_fixture_at(&format!("{base}/proxy")).await;
        let url = format!("{base}/proxy/Slack/mcp");
        let sender = ScopedSender::new(&url, registry_resolver(&file)).unwrap();
        assert!(discover(&sender, &url::Url::parse(&url).unwrap())
            .await
            .is_err());
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .all(|(path, _)| !path.contains("other")));
        stop(&gcx, &file, true).await;
    }

    #[tokio::test]
    async fn undiscovered_server_auth_is_unknown_not_claimed_authenticated() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let server = LiteLLMMcpServer {
            id: "server".into(),
            name: "Slack".into(),
            alias: None,
            display_name: None,
            description: None,
            auth_type: Some("bearer_token".into()),
            oauth2_flow: None,
            transport: Some("http".into()),
            status: None,
            mcp_endpoint_path: "/Slack/mcp".into(),
        };
        let view = server_view(&gcx, "work", &server, None, None).await;
        assert_eq!(view["auth"]["state"], "unknown");
        assert_eq!(view["connected"], false);
        assert_eq!(view["enabled"], false);
    }
}
