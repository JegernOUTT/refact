use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use refact_core::claude_backend::ClaudeTransportConfig;
use refact_providers::bedrock_setup as policy;
use refact_providers::claude_code::ClaudeCodeProvider;
use refact_providers::config_store;
use refact_providers::traits::ProviderTrait;
use refact_claude_cloud::bedrock_setup as cloud;

const TTL: Duration = Duration::from_secs(900);

#[derive(Clone)]
struct Setup {
    owner: std::sync::Weak<GlobalContext>,
    provider: String,
    created: Instant,
    revision: String,
    disk: Option<serde_yaml::Value>,
    runtime: String,
    source: String,
    transport: ClaudeTransportConfig,
    identity: Value,
    supports_1m: HashSet<String>,
    operation: Arc<Mutex<policy::SessionState>>,
}

fn registry() -> &'static Mutex<HashMap<String, Setup>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Setup>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn error(status: StatusCode, message: &str) -> ScratchError {
    ScratchError::new(status, message.into())
}
fn bad(message: impl AsRef<str>) -> ScratchError {
    error(StatusCode::BAD_REQUEST, message.as_ref())
}
fn stale() -> ScratchError {
    error(
        StatusCode::CONFLICT,
        "Bedrock setup is stale or expired; verify again",
    )
}
fn digest(value: &impl serde::Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}
fn present(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|v| !v.is_empty())
}
fn stopping(gcx: &GlobalContext) -> Result<(), ScratchError> {
    if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
        Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Engine is shutting down",
        ))
    } else {
        Ok(())
    }
}

async fn disk(gcx: &GlobalContext, name: &str) -> Result<Option<serde_yaml::Value>, ScratchError> {
    let path = config_store::provider_config_path(&gcx.config_dir, name);
    match tokio::fs::read_to_string(path).await {
        Ok(text) => serde_yaml::from_str(&text)
            .map(Some)
            .map_err(|_| error(StatusCode::CONFLICT, "Provider configuration is invalid")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot read provider configuration",
        )),
    }
}

fn configured(
    name: &str,
    settings: serde_yaml::Value,
) -> Result<Box<dyn ProviderTrait>, ScratchError> {
    let identity =
        refact_providers::identity::provider_identity_from_yaml(name, &settings).map_err(bad)?;
    if identity.base_provider != "claude_code" {
        return Err(bad("Guided Bedrock setup requires a Claude Code provider"));
    }
    let mut provider = refact_providers::create_provider("claude_code")
        .ok_or_else(|| bad("Claude provider unavailable"))?;
    provider
        .provider_settings_apply(settings)
        .map_err(|_| bad("Invalid Claude provider settings"))?;
    if identity.wrap_instance {
        provider = Box::new(refact_providers::instance::ProviderInstance::new(
            name,
            "claude_code",
            identity
                .display_name
                .unwrap_or_else(|| "Claude Code".into()),
            provider,
        ));
    }
    Ok(provider)
}

async fn snapshot(
    gcx: &Arc<GlobalContext>,
    name: &str,
) -> Result<(ClaudeCodeProvider, String, Option<serde_yaml::Value>), ScratchError> {
    stopping(gcx)?;
    refact_providers::identity::validate_provider_instance_id(name).map_err(bad)?;
    let settings = disk(gcx, name).await?;
    let provider = {
        let providers = gcx.providers.read().await;
        providers.get(name).map(ProviderTrait::clone_box)
    };
    let provider = match provider {
        Some(provider) => provider,
        None => configured(
            name,
            settings
                .clone()
                .ok_or_else(|| bad("Save the Bedrock backend before guided setup"))?,
        )?,
    };
    if provider.is_readonly() {
        return Err(error(StatusCode::FORBIDDEN, "Provider is readonly"));
    }
    let claude = provider
        .as_any()
        .downcast_ref::<ClaudeCodeProvider>()
        .ok_or_else(|| bad("Guided setup requires a Bedrock Claude provider"))?;
    if claude.transport.backend != refact_core::claude_backend::ClaudeBackend::Bedrock {
        return Err(bad(
            "Select and save the Bedrock backend before guided setup",
        ));
    }
    let fingerprint = digest(claude);
    Ok((claude.clone(), fingerprint, settings))
}

async fn host() -> Result<policy::HostProfiles, ScratchError> {
    policy::host_profiles().await.map_err(bad)
}

async fn session(
    gcx: &Arc<GlobalContext>,
    name: &str,
    id: &str,
) -> Result<(Setup, OwnedMutexGuard<policy::SessionState>), ScratchError> {
    let setup = {
        let mut registry = registry().lock().await;
        registry.retain(|_, setup| setup.created.elapsed() < TTL && setup.owner.strong_count() > 0);
        registry
            .get(id)
            .filter(|s| {
                s.owner
                    .upgrade()
                    .is_some_and(|owner| Arc::ptr_eq(&owner, gcx))
                    && s.provider == name
            })
            .cloned()
            .ok_or_else(stale)?
    };
    // Never await an operation while holding the registry. Owned guard releases on cancellation.
    let operation = setup.operation.clone().try_lock_owned().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "A setup operation is already in progress",
        )
    })?;
    operation.available().map_err(|_| stale())?;
    current(gcx, name, &setup).await?;
    Ok((setup, operation))
}

async fn current(gcx: &Arc<GlobalContext>, name: &str, setup: &Setup) -> Result<(), ScratchError> {
    let (provider, runtime, settings) = snapshot(gcx, name).await?;
    if setup.created.elapsed() >= TTL
        || runtime != setup.runtime
        || settings != setup.disk
        || host().await?.fingerprint != setup.source
    {
        return Err(stale());
    }
    stopping(gcx)?;
    if setup.created.elapsed() >= TTL {
        return Err(stale());
    }
    policy::selected_transport(
        &provider,
        setup.transport.aws_profile.as_deref().unwrap_or_default(),
        setup.transport.aws_region.as_deref().unwrap_or_default(),
        present,
    )
    .map_err(|_| stale())?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ProviderPath {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    profile: String,
    region: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRequest {
    setup_id: String,
    model_ids: Vec<String>,
    #[serde(default)]
    context_1m: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    setup_id: String,
    config_revision: String,
    model_ids: Vec<String>,
    #[serde(default)]
    context_1m: bool,
}

pub async fn profiles(
    State(app): State<AppState>,
    Path(path): Path<ProviderPath>,
) -> Result<Json<Value>, ScratchError> {
    let (provider, _, _) = snapshot(&app.gcx, &path.name).await?;
    let host = host().await?;
    let configured = provider.resolved_transport();
    Ok(Json(
        serde_json::to_value(policy::suggestions(
            host.profiles,
            configured.aws_profile,
            configured.aws_region,
            std::env::var("AWS_PROFILE")
                .ok()
                .or_else(|| std::env::var("AWS_DEFAULT_PROFILE").ok()),
            std::env::var("AWS_REGION")
                .ok()
                .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok()),
        ))
        .map_err(|_| bad("Cannot serialize profile metadata"))?,
    ))
}

pub async fn verify(
    State(app): State<AppState>,
    Path(path): Path<ProviderPath>,
    Json(request): Json<VerifyRequest>,
) -> Result<Json<Value>, ScratchError> {
    let (provider, runtime, settings) = snapshot(&app.gcx, &path.name).await?;
    let host = host().await?;
    if !host.profiles.iter().any(|p| p.name == request.profile) {
        return Err(bad(
            "Selected AWS profile does not exist on the engine host; configure it first",
        ));
    }
    let transport =
        policy::selected_transport(&provider, &request.profile, &request.region, present)
            .map_err(bad)?;
    let revision = digest(&(settings.clone(), &runtime, &host.fingerprint));
    let mut setup = Setup {
        owner: Arc::downgrade(&app.gcx),
        provider: path.name.clone(),
        created: Instant::now(),
        revision: revision.clone(),
        disk: settings,
        runtime,
        source: host.fingerprint,
        transport,
        identity: Value::Null,
        supports_1m: HashSet::new(),
        operation: Arc::new(Mutex::new(policy::SessionState::default())),
    };
    let mut response = bounded(&app.gcx, cloud::verify(&setup.transport))
        .await?
        .map_err(|message| error(StatusCode::BAD_GATEWAY, &message))?;
    current(&app.gcx, &path.name, &setup).await?;
    setup.identity = response["identity"].clone();
    let mut candidates = Vec::new();
    if let Some(profiles) = response["inference_profiles"].as_array() {
        for profile in profiles {
            if let Some(id) = profile["id"].as_str() {
                let supported = cloud::supports_1m(id);
                if supported {
                    setup.supports_1m.insert(id.into());
                }
                candidates.push(json!({"model_id":id,"supports_1m":supported}));
            }
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    {
        let mut registry = registry().lock().await;
        registry.retain(|_, setup| setup.created.elapsed() < TTL && setup.owner.strong_count() > 0);
        if registry.len() >= 32 {
            return Err(error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many active setup sessions; retry after an existing session expires",
            ));
        }
        registry.insert(id.clone(), setup);
    }
    response["setup_id"] = json!(id);
    response["config_revision"] = json!(revision);
    response["profile"] = json!(request.profile);
    response["region"] = json!(request.region);
    response["candidates"] = json!(candidates);
    Ok(Json(response))
}

async fn bounded<T>(
    gcx: &GlobalContext,
    future: impl std::future::Future<Output = T>,
) -> Result<T, ScratchError> {
    let shutdown = async {
        loop {
            if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(90), future) => result.map_err(|_| error(StatusCode::GATEWAY_TIMEOUT, "AWS setup operation timed out")),
        _ = shutdown => Err(error(StatusCode::SERVICE_UNAVAILABLE, "Engine is shutting down")),
    }
}

fn validate_selection(ids: &[String], context_1m: bool, setup: &Setup) -> Result<(), ScratchError> {
    policy::validate_models(ids).map_err(bad)?;
    if context_1m
        && ids
            .iter()
            .any(|id| !setup.supports_1m.contains(id) && !cloud::supports_1m(id))
    {
        return Err(bad("1M context is not confirmed for all selected models"));
    }
    Ok(())
}

pub async fn probe(
    State(app): State<AppState>,
    Path(path): Path<ProviderPath>,
    Json(request): Json<ProbeRequest>,
) -> Result<Json<Value>, ScratchError> {
    let (setup, mut operation) = session(&app.gcx, &path.name, &request.setup_id).await?;
    validate_selection(&request.model_ids, request.context_1m, &setup)?;
    current(&app.gcx, &path.name, &setup).await?;
    operation.begin_probe(&request.model_ids, request.context_1m);
    let mut results = Vec::new();
    for id in &request.model_ids {
        current(&app.gcx, &path.name, &setup).await?;
        let result = bounded(
            &app.gcx,
            cloud::probe_verified(&setup.transport, id, request.context_1m, &setup.identity),
        )
        .await;
        let (status, code, message) = match result {
            Ok(Ok((status, message))) => {
                let (label, code) = match status {
                    200..=299 => ("working", None),
                    401 | 403 => ("denied", Some("AccessDenied")),
                    404 => ("unavailable", Some("ModelUnavailable")),
                    _ => ("error", Some("ProbeFailed")),
                };
                (label, code, Some(message))
            }
            Ok(Err(message)) => ("error", Some("ProbeFailed"), Some(message)),
            Err(error) => return Err(error),
        };
        current(&app.gcx, &path.name, &setup).await?;
        operation.record_probe(id, request.context_1m, status == "working");
        results.push(json!({"model_id":id,"status":status,"error_code":code,"message":message}));
    }
    Ok(Json(
        json!({"setup_id":request.setup_id,"results":results,"warnings":["Invocation probes do not establish tool, image or context capabilities for opaque custom IDs. Unknown IDs use conservative editable model metadata."]}),
    ))
}

async fn persist_selection(
    gcx: &GlobalContext,
    setup: &Setup,
    claude: &ClaudeCodeProvider,
    selected: &[String],
    context_1m: bool,
    operation: &mut policy::SessionState,
    source: impl FnOnce() -> Result<String, String>,
) -> Result<serde_yaml::Value, ScratchError> {
    operation
        .require_tested(selected, context_1m)
        .map_err(bad)?;
    let profile = setup.transport.aws_profile.as_deref().ok_or_else(stale)?;
    let region = setup.transport.aws_region.as_deref().ok_or_else(stale)?;
    let updated = config_store::update_provider_config_with(
        &gcx.config_dir,
        &setup.provider,
        |_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot persist Bedrock provider configuration",
            )
        },
        |existing| {
            commit_guard(gcx, &setup, &existing, source().map_err(bad)?)?;
            policy::selected_transport(claude, profile, region, present).map_err(|_| stale())?;
            let settings = existing.ok_or_else(stale)?;
            let updated = policy::pin_models(settings, profile, region, &selected, context_1m)
                .map_err(bad)?;
            configured(&setup.provider, updated.clone())?;
            // Burn the session before the first cancellable persistence await.
            // A failed/cancelled write requires verification, never replay of old evidence.
            operation.consume();
            Ok(updated)
        },
    )
    .await?;
    Ok(updated)
}

// Called inside the config-store CAS callback, after provider AND config locks.
fn commit_guard(
    gcx: &GlobalContext,
    setup: &Setup,
    existing: &Option<serde_yaml::Value>,
    source: String,
) -> Result<(), ScratchError> {
    stopping(gcx)?;
    if existing != &setup.disk || setup.created.elapsed() >= TTL || source != setup.source {
        return Err(stale());
    }
    Ok(())
}

pub async fn apply(
    State(app): State<AppState>,
    Path(path): Path<ProviderPath>,
    Json(request): Json<ApplyRequest>,
) -> Result<Json<Value>, ScratchError> {
    let (setup, mut operation) = session(&app.gcx, &path.name, &request.setup_id).await?;
    if request.config_revision != setup.revision {
        return Err(stale());
    }
    validate_selection(&request.model_ids, request.context_1m, &setup)?;
    operation
        .require_tested(&request.model_ids, request.context_1m)
        .map_err(bad)?;
    let host_fingerprint = host().await?.fingerprint;
    if host_fingerprint != setup.source {
        return Err(stale());
    }
    let mut providers = app.gcx.providers.write().await;
    stopping(&app.gcx)?;
    if setup.created.elapsed() >= TTL || host().await?.fingerprint != setup.source {
        return Err(stale());
    }
    let provider = providers.get(&path.name).ok_or_else(stale)?;
    if provider.is_readonly() {
        return Err(error(StatusCode::FORBIDDEN, "Provider is readonly"));
    }
    let claude = provider
        .as_any()
        .downcast_ref::<ClaudeCodeProvider>()
        .ok_or_else(stale)?;
    if digest(claude) != setup.runtime {
        return Err(stale());
    }
    let selected = request.model_ids.clone();
    let updated = persist_selection(
        &app.gcx,
        &setup,
        claude,
        &selected,
        request.context_1m,
        &mut operation,
        policy::host_fingerprint_now,
    )
    .await?;
    providers.add(configured(&path.name, updated)?);
    drop(providers);
    refact_providers::credential::invalidate(&path.name);
    let mut caps = app.gcx.caps_state.write().await;
    caps.caps = None;
    caps.last_attempted_ts = 0;
    drop(caps);
    app.gcx.tool_catalog_generations.advance_capabilities();
    Ok(Json(json!({"success":true,"model_ids":request.model_ids})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_commit_guard_rechecks_state_after_lock_wait() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let disk = Some(serde_yaml::from_str("backend: bedrock\nenabled: false").unwrap());
        let mut setup = Setup {
            owner: Arc::downgrade(&gcx),
            provider: "claude_code".into(),
            created: Instant::now(),
            revision: "revision".into(),
            disk: disk.clone(),
            runtime: "runtime".into(),
            source: "original-source".into(),
            transport: ClaudeTransportConfig::default(),
            identity: Value::Null,
            supports_1m: HashSet::new(),
            operation: Arc::new(Mutex::new(policy::SessionState::default())),
        };
        assert!(commit_guard(&gcx, &setup, &disk, setup.source.clone()).is_ok());
        let provider_path = config_store::provider_config_path(&gcx.config_dir, &setup.provider);
        tokio::fs::create_dir_all(provider_path.parent().unwrap())
            .await
            .unwrap();
        let original = serde_yaml::to_string(disk.as_ref().unwrap()).unwrap();
        tokio::fs::write(&provider_path, &original).await.unwrap();
        setup.transport.backend = refact_core::claude_backend::ClaudeBackend::Bedrock;
        setup.transport.aws_profile = Some("team".into());
        setup.transport.aws_region = Some("us-east-1".into());
        let provider = ClaudeCodeProvider {
            transport: setup.transport.clone(),
            ..Default::default()
        };
        let ids = vec!["custom-manual-id".to_string()];
        let mut operation = policy::SessionState::default();
        operation.record_probe(&ids[0], false, true);
        let refusal =
            persist_selection(&gcx, &setup, &provider, &ids, false, &mut operation, || {
                Ok("changed-source".into())
            })
            .await
            .expect_err("changed credential source must refuse persistence");
        assert_eq!(refusal.status_code, StatusCode::CONFLICT);
        assert_eq!(
            tokio::fs::read_to_string(&provider_path).await.unwrap(),
            original,
            "stale setup persisted before final guard"
        );
        let updated =
            persist_selection(&gcx, &setup, &provider, &ids, false, &mut operation, || {
                Ok(setup.source.clone())
            })
            .await
            .unwrap();
        assert_eq!(
            updated["enabled_models"],
            serde_yaml::to_value(&ids).unwrap()
        );
        let pinned = tokio::fs::read_to_string(&provider_path).await.unwrap();
        assert_ne!(pinned, original);
        assert_eq!(
            serde_yaml::from_str::<serde_yaml::Value>(&pinned).unwrap(),
            updated
        );
        let replay =
            persist_selection(&gcx, &setup, &provider, &ids, false, &mut operation, || {
                panic!("replay inspected host before consumed guard")
            })
            .await
            .unwrap_err();
        assert_eq!(replay.status_code, StatusCode::BAD_REQUEST);
        assert_eq!(
            tokio::fs::read_to_string(&provider_path).await.unwrap(),
            pinned
        );

        assert_eq!(
            commit_guard(&gcx, &setup, &disk, "changed-source".into())
                .unwrap_err()
                .status_code,
            StatusCode::CONFLICT
        );
        let changed = Some(serde_yaml::from_str("backend: bedrock\nenabled: true").unwrap());
        assert_ne!(changed, disk);
        assert_eq!(
            commit_guard(&gcx, &setup, &changed, setup.source.clone())
                .unwrap_err()
                .status_code,
            StatusCode::CONFLICT
        );
        setup.created = Instant::now() - TTL;
        assert_eq!(
            commit_guard(&gcx, &setup, &disk, setup.source.clone())
                .unwrap_err()
                .status_code,
            StatusCode::CONFLICT
        );
        setup.created = Instant::now();
        gcx.shutdown_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            commit_guard(&gcx, &setup, &disk, setup.source.clone())
                .unwrap_err()
                .status_code,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
