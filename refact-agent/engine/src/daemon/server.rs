use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::HeaderMap;
use axum::middleware;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use futures::Stream;
use hyper::{Server, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::{AllowCredentials, AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

use crate::daemon::config::DaemonConfig;
use crate::daemon::state::{DaemonState, DaemonUpdateState, UpdatePhase};

const UPDATE_CHECK_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Serialize)]
struct StatusResponse {
    pid: u32,
    version: String,
    executable_sha256: Option<String>,
    installed_image_sha256: Option<String>,
    running_image_sha256: Option<String>,
    port: u16,
    started_at_ms: u64,
    uptime_secs: u64,
    workers: u64,
    cron_pending: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Deserialize)]
struct ShutdownRequest {
    reason: String,
}

#[derive(Debug, Deserialize)]
struct UpdateCheckQuery {
    #[serde(default)]
    refresh: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateInstallRequest {
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    #[serde(default)]
    follow: bool,
    after_seq: Option<u64>,
    last_event_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    project_id: Option<String>,
    #[serde(default = "default_log_tail")]
    tail: usize,
}

fn default_log_tail() -> usize {
    200
}

pub fn bind_listener(config: &DaemonConfig) -> Result<TcpListener, String> {
    let requested_ip = config
        .bind
        .parse::<IpAddr>()
        .map_err(|error| format!("invalid daemon bind address '{}': {error}", config.bind))?;
    crate::daemon::auth::validate_hooks_auth_policy(config, requested_ip)?;
    let ip = if !requested_ip.is_loopback() && !crate::daemon::auth::daemon_lan_auth_ok(config) {
        tracing::warn!(
            "daemon bind {requested_ip} is non-loopback but no auth credentials are configured; \
             falling back to 127.0.0.1 (enable auth with a username/password to allow LAN access)"
        );
        IpAddr::from([127, 0, 0, 1])
    } else {
        requested_ip
    };
    let addr = SocketAddr::new(ip, config.port);
    let listener = TcpListener::bind(addr)
        .map_err(|error| format!("failed to bind daemon control API at {addr}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon listener nonblocking: {error}"))?;
    Ok(listener)
}

pub fn make_router(state: Arc<DaemonState>, port: u16) -> Router {
    let (auth_token, auth_username, auth_password) = if state.config.auth.enabled {
        (
            state.auth_token.clone(),
            state.config.auth.username.clone(),
            state.config.auth.password.clone(),
        )
    } else {
        (None, None, None)
    };
    let hook_token = state.config.hooks.token.clone();
    let open_hooks_allowed =
        crate::daemon::auth::hooks_unauthenticated_allowed_for_bind(&state.config.bind);
    let control = Router::new()
        .route("/", get(crate::daemon::web::handle_dashboard_index))
        .route("/picker", get(crate::daemon::web::handle_project_picker))
        .route("/hooks", post(crate::daemon::hooks::bare))
        .route("/hooks/", post(crate::daemon::hooks::bare))
        .route("/hooks/wake", post(crate::daemon::hooks::wake))
        .route("/hooks/agent", post(crate::daemon::hooks::agent))
        .route("/hooks/:name", post(crate::daemon::hooks::named))
        .route(
            "/p/:project_id",
            get(crate::daemon::web::handle_project_gui_redirect),
        )
        .route(
            "/p/:project_id/",
            get(crate::daemon::web::handle_project_gui_index),
        )
        .route(
            "/dist/chat/*path",
            get(crate::daemon::web::handle_daemon_gui_asset),
        )
        .route("/cron/status", get(cron_status))
        .route("/daemon/v1/status", get(status))
        .route("/daemon/v1/doctor", get(crate::daemon::doctor::handle))
        .route("/daemon/v1/settings", get(get_settings))
        .route("/daemon/v1/settings", post(update_settings))
        .route("/daemon/v1/restart", post(restart))
        .route("/daemon/v1/update/check", get(update_check))
        .route("/daemon/v1/update/install", post(update_install))
        .route("/daemon/v1/update/status", get(update_status))
        .route(
            "/daemon/v1/projects/:id/candidate-installation/:hash/observation",
            get(observe_candidate_installation),
        )
        .route(
            "/daemon/v1/projects/:id/candidate-installation/prepare",
            post(prepare_candidate_installation).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/daemon/v1/projects/:id/candidate-installation/grant",
            post(grant_candidate_installation).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/daemon/v1/projects/:id/candidate-installation",
            post(candidate_installation).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/daemon/v1/projects/:id/operator/policy/preview",
            post(preview_operator_policy).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/daemon/v1/projects/:id/operator/policy/confirm",
            post(confirm_operator_policy).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/daemon/v1/projects/:id/host-release/prepare",
            post(prepare_host_release).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/daemon/v1/projects/:id/host-release/grant",
            post(grant_host_release).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/daemon/v1/projects/:id/host-release/dispatch",
            post(dispatch_host_release).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/daemon/v1/projects/:id/host-release/artifact",
            post(artifact_host_release).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/daemon/v1/projects/:id/host-release/receipt",
            post(receipt_host_release).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/daemon/v1/projects/:id/host-release/observe",
            post(observe_host_release).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/daemon/v1/projects/:id/gui-owner/observe",
            post(gui_owner_observation).layer(DefaultBodyLimit::max(4096)),
        )
        .route("/daemon/v1/shutdown", post(shutdown))
        .route("/daemon/v1/events", get(events))
        .route("/daemon/v1/workers", get(workers))
        .route("/daemon/v1/logs", get(logs))
        .route("/daemon/v1/logs/stream", get(logs_stream))
        .route(
            "/daemon/v1/fs/browse",
            post(crate::daemon::fs_browse::browse),
        )
        .route("/daemon/v1/worker-status", post(worker_status))
        .route(
            "/daemon/v1/buddy/worker-projection",
            post(buddy_worker_projection).layer(DefaultBodyLimit::max(65_536)),
        )
        .route(
            "/daemon/v1/projects/:id/buddy-wake-grant",
            get(buddy_wake_get)
                .post(buddy_wake_issue)
                .delete(buddy_wake_revoke)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/daemon/v1/projects/:id/buddy-projection",
            get(buddy_project_projection),
        )
        .route(
            "/daemon/v1/projects/open",
            post(crate::daemon::projects::open_project),
        )
        .route(
            "/daemon/v1/projects",
            get(crate::daemon::projects::list_projects),
        )
        .route(
            "/daemon/v1/projects/:id",
            get(crate::daemon::projects::get_project),
        )
        .route(
            "/daemon/v1/projects/:id",
            delete(crate::daemon::projects::forget_project),
        )
        .route(
            "/daemon/v1/projects/:id/pin",
            post(crate::daemon::projects::pin_project),
        )
        .route(
            "/daemon/v1/projects/:id/restart",
            post(crate::daemon::projects::restart_project_worker),
        )
        .route(
            "/daemon/v1/projects/:id/stop",
            post(crate::daemon::projects::stop_project_worker),
        )
        .layer(DefaultBodyLimit::disable());
    let proxy = Router::new()
        .route("/p/:project_id/v1", any(crate::daemon::proxy::proxy_v1))
        .route(
            "/p/:project_id/v1/*path",
            any(crate::daemon::proxy::proxy_v1),
        )
        .layer(DefaultBodyLimit::max(
            crate::daemon::proxy::PROXY_BODY_LIMIT,
        ));
    control
        .merge(proxy)
        .layer(middleware::from_fn(move |req, next| {
            let policy = crate::daemon::auth::DaemonAuthPolicy {
                token: auth_token.clone(),
                username: auth_username.clone(),
                password: auth_password.clone(),
                hook_token: hook_token.clone(),
                open_hooks_allowed,
            };
            crate::daemon::auth::enforce(policy, req, next)
        }))
        .layer(
            CorsLayer::new()
                .allow_methods(AllowMethods::mirror_request())
                .allow_headers(AllowHeaders::mirror_request())
                .allow_credentials(AllowCredentials::predicate(|origin, _parts| {
                    origin
                        .to_str()
                        .map(crate::daemon::auth::origin_allowed_for_loopback_or_ide)
                        .unwrap_or(false)
                }))
                .allow_origin(AllowOrigin::predicate(|origin, _parts| {
                    origin
                        .to_str()
                        .map(crate::daemon::auth::origin_allowed_for_loopback_or_ide)
                        .unwrap_or(false)
                })),
        )
        .with_state((state, port))
}

pub async fn serve(
    listener: TcpListener,
    state: Arc<DaemonState>,
    port: u16,
) -> Result<(), String> {
    let router = make_router(state.clone(), port);
    let server = Server::from_tcp(listener)
        .map_err(|error| format!("failed to create daemon server: {error}"))?
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(wait_for_shutdown(state));
    server
        .await
        .map_err(|error| format!("daemon server error: {error}"))
}

async fn status(State((state, port)): State<(Arc<DaemonState>, u16)>) -> Json<StatusResponse> {
    let uptime_secs =
        Duration::from_millis(crate::daemon::state::now_ms().saturating_sub(state.started_at_ms))
            .as_secs();
    let executable_sha256 =
        tokio::task::spawn_blocking(crate::daemon::state::current_executable_sha256)
            .await
            .unwrap_or(None);
    let running_image_sha256 = None;
    // Health polling must not rehash the kernel and installed images on every
    // request. Exact native-image reconciliation belongs to the authenticated
    // candidate-installation observation endpoint; null here means unobserved,
    // never that the installed and running images are equal.
    let installed_image_sha256 = None;
    Json(StatusResponse {
        pid: std::process::id(),
        version: state.version.clone(),
        executable_sha256,
        port,
        installed_image_sha256,
        running_image_sha256,
        started_at_ms: state.started_at_ms,
        uptime_secs,
        workers: state.supervisor.worker_count().await,
        cron_pending: state.cron_pending_snapshot().await,
    })
}

async fn shutdown(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<ShutdownRequest>,
) -> Json<serde_json::Value> {
    state.request_shutdown(request.reason);
    Json(json!({"success": true}))
}

async fn restart(State((state, _)): State<(Arc<DaemonState>, u16)>) -> Response {
    if let Err(error) = spawn_daemon_relaunch(&state) {
        tracing::warn!("daemon restart relaunch failed: {error}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("daemon restart failed: {error}")})),
        )
            .into_response();
    }
    state.request_shutdown("restart_requested".to_string());
    Json(json!({"success": true, "restarting": true})).into_response()
}

async fn update_check(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Query(query): Query<UpdateCheckQuery>,
) -> Response {
    let info = match cached_update_check(&state, query.refresh).await {
        Ok(info) => info,
        Err(error) => {
            return (StatusCode::BAD_GATEWAY, Json(json!({"error": error}))).into_response();
        }
    };
    update_check_response(info).into_response()
}

pub(crate) async fn cached_update_check(
    state: &DaemonState,
    refresh: bool,
) -> Result<refact_self_update::UpdateCheckInfo, String> {
    let now = Instant::now();
    if !refresh {
        let cached = {
            let cache = state.update_check_cache.lock().await;
            cache
                .as_ref()
                .filter(|(checked_at, _)| now.duration_since(*checked_at) < UPDATE_CHECK_CACHE_TTL)
                .map(|(_, info)| info.clone())
        };
        if let Some(info) = cached {
            return Ok(info);
        }
    }
    let info = refact_self_update::check_update_info(10).await?;
    let mut cache = state.update_check_cache.lock().await;
    *cache = Some((now, info.clone()));
    Ok(info)
}

fn update_check_response(info: refact_self_update::UpdateCheckInfo) -> Json<serde_json::Value> {
    Json(json!({
        "current_version": info.current_version,
        "latest_version": info.latest_version,
        "update_available": info.update_available,
        "releases": info.releases,
        "checked_at_ms": crate::daemon::state::now_ms(),
    }))
}

async fn update_install(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<UpdateInstallRequest>,
) -> Response {
    let target_version = request.version.clone();
    {
        let mut update_state = state.update_state.lock().await;
        if update_phase_active(&update_state.phase) {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error": "update already in progress"})),
            )
                .into_response();
        }
        *update_state = DaemonUpdateState {
            phase: UpdatePhase::Checking,
            detail: None,
            target_version: target_version.clone(),
            started_at_ms: Some(crate::daemon::state::now_ms()),
            finished_at_ms: None,
        };
    }
    let task_state = state.clone();
    let task_version = target_version.clone();
    let handle = tokio::spawn(async move {
        set_update_phase(
            &task_state,
            UpdatePhase::Downloading,
            None,
            task_version.clone(),
        )
        .await;
        match refact_self_update::install_update_version(task_version.as_deref(), false).await {
            Ok(outcome) => {
                {
                    let mut update_state = task_state.update_state.lock().await;
                    update_state.phase = UpdatePhase::Restarting;
                    update_state.detail = Some(format!("installed {}", outcome.installed_version));
                    update_state.finished_at_ms = Some(crate::daemon::state::now_ms());
                }
                match spawn_daemon_relaunch(&task_state) {
                    Ok(()) => task_state.request_shutdown("self_update".to_string()),
                    Err(error) => {
                        let mut update_state = task_state.update_state.lock().await;
                        update_state.phase = UpdatePhase::Failed;
                        update_state.detail = Some(format!(
                            "update installed, but daemon relaunch failed: {error}; restart the daemon manually"
                        ));
                    }
                }
            }
            Err(error) => {
                let mut update_state = task_state.update_state.lock().await;
                update_state.phase = UpdatePhase::Failed;
                update_state.detail = Some(error);
                update_state.finished_at_ms = Some(crate::daemon::state::now_ms());
            }
        }
    });
    {
        let mut update_task = state.update_task.lock().await;
        *update_task = Some(handle);
    }
    (
        StatusCode::ACCEPTED,
        Json(json!({"started": true, "target_version": target_version})),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareInstallationRequest {
    spec: refact_buddy_core::operator_types::ActionSpec,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantInstallationRequest {
    prepared_hash: String,
    expires_in_seconds: u32,
}

async fn prepare_candidate_installation(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<PrepareInstallationRequest>,
) -> Response {
    installation_producer(state, id, headers, Some(request.spec), None).await
}
async fn grant_candidate_installation(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<GrantInstallationRequest>,
) -> Response {
    installation_producer(state, id, headers, None, Some(request)).await
}

async fn installation_producer(
    state: Arc<DaemonState>,
    id: String,
    headers: HeaderMap,
    spec: Option<refact_buddy_core::operator_types::ActionSpec>,
    grant: Option<GrantInstallationRequest>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (id, spec, grant);
        StatusCode::NOT_IMPLEMENTED.into_response()
    }
    #[cfg(target_os = "linux")]
    {
        let result: Result<serde_json::Value, String> = async {
            use crate::buddy::{resource_execution::*, action_execution::*, improvement::installation::*};
            use crate::buddy::resources::AsyncResourceAdapter;
            use refact_buddy_core::operator_types::*;
            let _execution = execution_gate().lock().await;
            let update = state.update_state.lock().await;
            if state.is_shutting_down() || update_phase_active(&update.phase) { return Err("daemon update/shutdown active".into()); }
            let _drained = state.supervisor.installation_drained().await?;
            let registry = state.projects.read().await;
            let entry = registry.get(&id).ok_or("registered project unavailable")?;
            let project = ProjectIdentity { project_id: entry.id.clone(), canonical_root: entry.root.to_string_lossy().into_owned() };
            let worker = WorkerIdentity { worker_id: "daemon-native-installer".into(), engine_instance_id: format!("daemon:{}:{}", std::process::id(), state.started_at_ms), epoch: state.started_at_ms };
            let runtime = crate::buddy::outcome_runtime::OutcomeRuntime::open(&entry.root, project, worker).map_err(|e| e.to_string())?;
            let result = async {
                if let Some(mut spec) = spec {
                    let reference = spec.parameter_ref.clone().ok_or("draft missing")?;
                    let draft = load_draft::<InstallationDraft>(&runtime, reference).await.map_err(|e| e.to_string())?.ok_or("draft missing")?;
                    validate_spec(&spec, &draft).map_err(|e| e.to_string())?;
                    if spec.project != runtime.project() { return Err("project changed".into()); }
                    let adapter = DaemonInstallationAdapter { registry: state.supervisor.native_exec.clone(), shutdown: state.supervisor.native_shutdown.clone(), runtime: runtime.clone(), request: NativeRequest { prepared_hash: String::new(), grant_id: String::new() }, executable: state.relaunch_executable()?.to_path_buf(), spec: spec.clone() };
                    let caps = adapter.capabilities();
                    adapter.preview(&spec).await.map_err(|e| format!("{e:?}"))?;
                    spec.capabilities = ActionCapabilities { can_preview: caps.preview, can_execute: caps.apply, can_verify: caps.verify, can_recover: caps.recover, can_cancel: false };
                    let prepared = PreparedAction { spec_hash: action_spec_hash(&spec, &spec.parameter_ref), bound_parameter_ref: spec.parameter_ref.clone(), spec };
                    bind_prepared_draft(&runtime, &prepared).await.map_err(|e| e.to_string())?;
                    let value = json!({"schema_version": 1, "project": runtime.project(), "payload": prepared, "daemon_prepared_by": state.config.auth.username, "daemon_preparation_version": 1});
                    let key = prepared.spec_hash.clone();
                    runtime.with_store(move |s| s.put_document("prepared_action", &key, &value, None)).await.map_err(|e| e.to_string())?;
                    Ok(json!({"prepared": prepared, "authority": "daemon_user_only", "running_upgrade_verified": false}))
                } else {
                    let request = grant.ok_or("request missing")?;
                    if !(1..=3600).contains(&request.expires_in_seconds) { return Err("invalid grant duration".into()); }
                    let prepared = load_prepared(&runtime, request.prepared_hash.clone()).await?;
                    let key = request.prepared_hash.clone();
                    let user = state.config.auth.username.clone().ok_or("user unavailable")?;
                    runtime.with_store(move |s| {
                        let value = s.get_document::<serde_json::Value>("prepared_action", &key)?.ok_or(crate::buddy::operator_store::OperatorStoreError::Conflict("preparation missing"))?.value;
                        if value["daemon_prepared_by"].as_str() != Some(user.as_str()) || value["daemon_preparation_version"] != 1 { return Err(crate::buddy::operator_store::OperatorStoreError::Conflict("daemon user preparation required")); }
                        Ok(())
                    }).await.map_err(|e| e.to_string())?;
                    validate_prepared_draft(&runtime, &prepared).await?;
                    let draft = load_draft::<InstallationDraft>(&runtime, prepared.spec.parameter_ref.clone().ok_or("draft missing")?).await.map_err(|e| e.to_string())?.ok_or("draft missing")?;
                    validate_spec(&prepared.spec, &draft).map_err(|e| e.to_string())?;
                    if prepared.spec.project != runtime.project() || !prepared.spec.capabilities.can_execute { return Err("daemon preparation required".into()); }
                    DaemonInstallationAdapter { registry: state.supervisor.native_exec.clone(), shutdown: state.supervisor.native_shutdown.clone(), runtime: runtime.clone(), request: NativeRequest { prepared_hash: prepared.spec_hash.clone(), grant_id: String::new() }, executable: state.relaunch_executable()?.to_path_buf(), spec: prepared.spec.clone() }.preview(&prepared.spec).await.map_err(|e| format!("{e:?}"))?;
                    let id = uuid::Uuid::new_v4().to_string();
                    let value = ExactActionGrant {
                        native_rights: None, recovery_execution_id: None, spec_hash: request.prepared_hash,
                        requester: NativeSourceIdentity { source_kind: NativeSourceKind::External, source_id: state.config.auth.username.clone().ok_or("user unavailable")?, source_version: "daemon-basic-user-v1".into() },
                        decision: Some(DecisionReference { project: runtime.project(), worker: runtime.worker(), native: NativeSourceIdentity { source_kind: NativeSourceKind::External, source_id: id.clone(), source_version: "daemon-installation-grant-v1".into() } }),
                        expires_at: (chrono::Utc::now() + chrono::Duration::seconds(request.expires_in_seconds.into())).to_rfc3339(), revoked: false,
                    };
                    persist_grant(&runtime, id.clone(), value.clone()).await.map_err(|e| e.to_string())?;
                    Ok(json!({"grant_id": id, "grant": value}))
                }
            }.await;
            runtime.shutdown().await;
            result
        }.await;
        match result {
            Ok(value) => Json(value).into_response(),
            Err(error) => (StatusCode::CONFLICT, Json(json!({"error": error}))).into_response(),
        }
    }
}

async fn prepare_host_release(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<PrepareInstallationRequest>,
) -> Response {
    host_release_producer(state, id, headers, Some(request.spec), None).await
}
async fn grant_host_release(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<GrantInstallationRequest>,
) -> Response {
    host_release_producer(state, id, headers, None, Some(request)).await
}

async fn host_release_producer(
    state: Arc<DaemonState>,
    id: String,
    headers: HeaderMap,
    spec: Option<refact_buddy_core::operator_types::ActionSpec>,
    grant: Option<GrantInstallationRequest>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (id, spec, grant);
        StatusCode::NOT_IMPLEMENTED.into_response()
    }
    #[cfg(target_os = "linux")]
    {
        let result: Result<serde_json::Value, String> = async {
            use crate::buddy::{resource_execution::*, action_execution::*, improvement::installation::{NativeRequest, release::*}};
            use crate::buddy::resources::AsyncResourceAdapter;
            use refact_buddy_core::operator_types::*;
            let _execution = execution_gate().lock().await;
            let update = state.update_state.lock().await;
            if state.is_shutting_down() || update_phase_active(&update.phase) { return Err("daemon update/shutdown active".into()); }
            let _drained = state.supervisor.installation_drained().await?;
            let registry = state.projects.read().await;
            let entry = registry.get(&id).ok_or("registered project unavailable")?;
            let project = ProjectIdentity { project_id: entry.id.clone(), canonical_root: entry.root.to_string_lossy().into_owned() };
            let worker = WorkerIdentity { worker_id: "daemon-native-host-release".into(), engine_instance_id: format!("daemon:{}:{}", std::process::id(), state.started_at_ms), epoch: state.started_at_ms };
            let runtime = crate::buddy::outcome_runtime::OutcomeRuntime::open(&entry.root, project, worker).map_err(|e| e.to_string())?;
            let result = async {
                if let Some(mut spec) = spec {
                    let reference = spec.parameter_ref.clone().ok_or("draft missing")?;
                    let draft = load_draft::<ReleaseDraft>(&runtime, reference).await.map_err(|e| e.to_string())?.ok_or("draft missing")?;
                    validate_spec(&spec, &draft).map_err(|e| e.to_string())?;
                    if spec.project != runtime.project() { return Err("project changed".into()); }
                    let adapter = ReleaseAdapter { runtime: runtime.clone(), native: NativeRequest { prepared_hash: String::new(), grant_id: String::new() }, spec: spec.clone() };
                    let caps = adapter.capabilities();
                    adapter.preview(&spec).await.map_err(|e| format!("{e:?}"))?;
                    spec.capabilities = ActionCapabilities { can_preview: caps.preview, can_execute: caps.apply, can_verify: caps.verify, can_recover: caps.recover, can_cancel: false };
                    let prepared = PreparedAction { spec_hash: action_spec_hash(&spec, &spec.parameter_ref), bound_parameter_ref: spec.parameter_ref.clone(), spec };
                    bind_prepared_draft(&runtime, &prepared).await.map_err(|e| e.to_string())?;
                    let value = json!({"schema_version": 1, "project": runtime.project(), "payload": prepared, "daemon_prepared_by": state.config.auth.username, "daemon_preparation_version": 1});
                    let key = prepared.spec_hash.clone();
                    runtime.with_store(move |s| s.put_document("prepared_action", &key, &value, None)).await.map_err(|e| e.to_string())?;
                    Ok(json!({"prepared": prepared, "authority": "daemon_user_only", "running_upgrade_verified": false}))
                } else {
                    let request = grant.ok_or("request missing")?;
                    if !(1..=3600).contains(&request.expires_in_seconds) { return Err("invalid grant duration".into()); }
                    let prepared = load_prepared(&runtime, request.prepared_hash.clone()).await?;
                    let key = request.prepared_hash.clone();
                    let user = state.config.auth.username.clone().ok_or("user unavailable")?;
                    runtime.with_store(move |s| {
                        let value = s.get_document::<serde_json::Value>("prepared_action", &key)?.ok_or(crate::buddy::operator_store::OperatorStoreError::Conflict("preparation missing"))?.value;
                        if value["daemon_prepared_by"].as_str() != Some(user.as_str()) || value["daemon_preparation_version"] != 1 { return Err(crate::buddy::operator_store::OperatorStoreError::Conflict("daemon user preparation required")); }
                        Ok(())
                    }).await.map_err(|e| e.to_string())?;
                    validate_prepared_draft(&runtime, &prepared).await?;
                    let draft = load_draft::<ReleaseDraft>(&runtime, prepared.spec.parameter_ref.clone().ok_or("draft missing")?).await.map_err(|e| e.to_string())?.ok_or("draft missing")?;
                    validate_spec(&prepared.spec, &draft).map_err(|e| e.to_string())?;
                    if prepared.spec.project != runtime.project() || !prepared.spec.capabilities.can_execute { return Err("daemon preparation required".into()); }
                    ReleaseAdapter { runtime: runtime.clone(), native: NativeRequest { prepared_hash: prepared.spec_hash.clone(), grant_id: String::new() }, spec: prepared.spec.clone() }.preview(&prepared.spec).await.map_err(|e| format!("{e:?}"))?;
                    let id = uuid::Uuid::new_v4().to_string();
                    let value = ExactActionGrant {
                        native_rights: None, recovery_execution_id: None, spec_hash: request.prepared_hash,
                        requester: NativeSourceIdentity { source_kind: NativeSourceKind::External, source_id: state.config.auth.username.clone().ok_or("user unavailable")?, source_version: "daemon-basic-user-v1".into() },
                        decision: Some(DecisionReference { project: runtime.project(), worker: runtime.worker(), native: NativeSourceIdentity { source_kind: NativeSourceKind::External, source_id: id.clone(), source_version: "daemon-installation-grant-v1".into() } }),
                        expires_at: (chrono::Utc::now() + chrono::Duration::seconds(request.expires_in_seconds.into())).to_rfc3339(), revoked: false,
                    };
                    persist_grant(&runtime, id.clone(), value.clone()).await.map_err(|e| e.to_string())?;
                    Ok(json!({"grant_id": id, "grant": value}))
                }
            }.await;
            runtime.shutdown().await;
            result
        }.await;
        match result {
            Ok(value) => Json(value).into_response(),
            Err(error) => (StatusCode::CONFLICT, Json(json!({"error": error}))).into_response(),
        }
    }
}

/// Reconciliation is observation only: never dispatch or replay a restart.
async fn observe_candidate_installation(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path((id, hash)): axum::extract::Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (id, hash);
        StatusCode::NOT_IMPLEMENTED.into_response()
    }
    #[cfg(target_os = "linux")]
    {
        let result: Result<serde_json::Value, String> = async {
            use crate::buddy::improvement::installation::*;
            let _execution = crate::buddy::resource_execution::execution_gate().lock().await;
            let _drained = state.supervisor.installation_drained().await?;
            let registry = state.projects.read().await;
            let entry = registry.get(&id).ok_or("registered project unavailable")?;
            let project = refact_buddy_core::operator_types::ProjectIdentity { project_id: entry.id.clone(), canonical_root: entry.root.to_string_lossy().into_owned() };
            let generation = format!("daemon:{}:{}", std::process::id(), state.started_at_ms);
            let worker = refact_buddy_core::operator_types::WorkerIdentity { worker_id: "daemon-native-installer".into(), engine_instance_id: generation.clone(), epoch: state.started_at_ms };
            let runtime = crate::buddy::outcome_runtime::OutcomeRuntime::open(&entry.root, project, worker).map_err(|e| e.to_string())?;
            let running = refact_self_update::candidate_install::running_digest();
            let result = match running {
                Err(error) => Err(error),
                Ok(running) => runtime.with_store(move |s| {
                    let stored = s.get_document::<InstallationRecord>(NAMESPACE, &hash)?.ok_or(crate::buddy::operator_store::OperatorStoreError::Conflict("installation record missing"))?;
                    let mut record = stored.value;
                    if crate::buddy::improvement::installation::reconcile_running(&mut record, &generation, &running) {
                        s.put_reserved_document("native_installation", &hash, NAMESPACE, &hash, &record, Some(stored.revision))?;
                    }
                    Ok(json!({"native_observation": record, "workers_upgrade_verified": false, "restart_dispatched": false}))
                }).await.map_err(|e| e.to_string()),
            };
            runtime.shutdown().await;
            result
        }.await;
        match result {
            Ok(value) => Json(value).into_response(),
            Err(error) => (StatusCode::CONFLICT, Json(json!({"error": error}))).into_response(),
        }
    }
}

async fn candidate_installation(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::buddy::improvement::installation::NativeRequest>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (id, request);
        return (
            StatusCode::NOT_IMPLEMENTED,
            "Native candidate installation supports Linux only",
        )
            .into_response();
    }
    #[cfg(target_os = "linux")]
    {
        match consume_candidate_installation(state, id, request).await {
            Ok(result) => Json(result).into_response(),
            Err(error) => (StatusCode::CONFLICT, Json(json!({"error": error}))).into_response(),
        }
    }
}

#[cfg(target_os = "linux")]
async fn consume_candidate_installation(
    state: Arc<DaemonState>,
    id: String,
    request: crate::buddy::improvement::installation::NativeRequest,
) -> Result<serde_json::Value, String> {
    use crate::buddy::{improvement::installation::*, resource_execution::*, action_execution::*};
    let _execution = execution_gate().lock().await;
    let _update = state.update_state.lock().await;
    if state.is_shutting_down() || update_phase_active(&_update.phase) {
        return Err("daemon update/shutdown active".into());
    }
    let _drained = state.supervisor.installation_drained().await?;
    let registry = state.projects.read().await;
    let entry = registry.get(&id).ok_or("registered project unavailable")?;
    let project = refact_buddy_core::operator_types::ProjectIdentity {
        project_id: entry.id.clone(),
        canonical_root: entry.root.to_string_lossy().into_owned(),
    };
    let worker = refact_buddy_core::operator_types::WorkerIdentity {
        worker_id: "daemon-native-installer".into(),
        engine_instance_id: format!("daemon:{}:{}", std::process::id(), state.started_at_ms),
        epoch: state.started_at_ms,
    };
    // Existing OperatorStore exclusive owner lock prevents opening a live evaluator.
    let runtime =
        crate::buddy::outcome_runtime::OutcomeRuntime::open(&entry.root, project, worker.clone())
            .map_err(|e| e.to_string())?;
    let result = async {
        let prepared = load_prepared(&runtime, request.prepared_hash.clone()).await?;
        validate_prepared_draft(&runtime, &prepared).await?;
        let grant = load_grant(&runtime, request.grant_id.clone()).await.map_err(|e| e.to_string())?.ok_or("grant unavailable")?;
        if !daemon_grant(&grant, &request.grant_id) || !grant.authorize(&prepared.spec, &grant.requester, grant.decision.as_ref()) { return Err("explicit exact daemon user grant unavailable".into()); }
        let key = prepared.spec_hash.clone();
        runtime.with_store(move |s| crate::buddy::desk_actions::validate_resource_origin(s, &key)).await.map_err(|e| e.to_string())?;
        let action = ActionRequest {
            execution_id: format!("native-install:{}", request.prepared_hash), idempotency_key: format!("native-install:{}", request.prepared_hash),
            spec: prepared.spec.clone(), bound_parameter_ref: prepared.bound_parameter_ref,
            requester: grant.requester.clone(), decision: grant.decision.clone(), worker,
        };
        let mut catalog = ResourceCatalog::default();
        catalog.register(ACTION.into(), Arc::new(DaemonInstallationAdapter { registry: state.supervisor.native_exec.clone(), shutdown: state.supervisor.native_shutdown.clone(), runtime: runtime.clone(), request: request.clone(), executable: state.relaunch_executable()?.to_path_buf(), spec: action.spec.clone() })).map_err(|e| format!("{e:?}"))?;
        let outcome = catalog.service(runtime.clone(), Arc::new(grant)).map_err(|_| "native catalog unavailable")?.execute_async(&action).await;
        let key = request.prepared_hash.clone();
        let record = runtime.with_store(move |s| s.get_document::<InstallationRecord>(NAMESPACE, &key)).await.map_err(|e| e.to_string())?;
        // Restart is separate consent and happens only after ActionService persisted
        // its terminal receipt. A response-lost duplicate must not restart again.
        if matches!(&outcome, ActionResult::Finished(value) if value.execution.status == refact_buddy_core::operator_types::ActionExecutionState::Partial) && record.as_ref().is_some_and(|r| r.value.phase == "restart_requested_running_unverified") {
            let key = request.prepared_hash.clone();
            let generation = format!("daemon:{}:{}", std::process::id(), state.started_at_ms);
            runtime.with_store(move |s| {
                let stored = s.get_document::<InstallationRecord>(NAMESPACE, &key)?.ok_or(crate::buddy::operator_store::OperatorStoreError::Conflict("restart record missing"))?;
                if stored.value.restart_intent_generation.is_some() { return Err(crate::buddy::operator_store::OperatorStoreError::Conflict("restart already dispatched or outcome unknown")); }
                let mut record = stored.value;
                record.restart_intent_generation = Some(generation);
                record.phase = "restart_dispatch_outcome_unknown".into();
                s.put_reserved_document("native_installation", &key, NAMESPACE, &key, &record, Some(stored.revision))?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            spawn_daemon_relaunch(&state)?;
            state.request_shutdown("exact_candidate_restart".into());
        }
        let key = request.prepared_hash.clone();
        let record = runtime.with_store(move |s| s.get_document::<InstallationRecord>(NAMESPACE, &key)).await.map_err(|e| e.to_string())?;
        Ok(json!({"action_result": outcome, "native_observation": record.map(|r| r.value)}))
    }.await;
    runtime.shutdown().await;
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuiOwnerRequest {
    gui_candidate_id: String,
    gui_candidate_digest: String,
    owner_candidate_id: String,
    owner_candidate_digest: String,
}

async fn gui_owner_observation(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<GuiOwnerRequest>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (id, request);
        StatusCode::NOT_IMPLEMENTED.into_response()
    }
    #[cfg(target_os = "linux")]
    {
        native_control_response(async {
        use crate::buddy::improvement::{self, artifacts};
        let _execution = crate::buddy::resource_execution::execution_gate().lock().await;
        let _drained = state.supervisor.installation_drained().await?;
        let registry = state.projects.read().await;
        let entry = registry.get(&id).ok_or("registered project unavailable")?;
        let project = refact_buddy_core::operator_types::ProjectIdentity { project_id: entry.id.clone(), canonical_root: entry.root.to_string_lossy().into_owned() };
        let store = crate::buddy::operator_store::OperatorStore::open(&entry.root, &project).map_err(|e| e.to_string())?;
        let gui = improvement::candidate(&store, &request.gui_candidate_id).map_err(|e| e.to_string())?.value;
        let owner = improvement::candidate(&store, &request.owner_candidate_id).map_err(|e| e.to_string())?.value;
        if crate::buddy::improvement_evaluation::digest(&gui).map_err(|e| e.to_string())? != request.gui_candidate_digest || crate::buddy::improvement_evaluation::digest(&owner).map_err(|e| e.to_string())? != request.owner_candidate_digest { return Err("candidate binding changed".into()); }
        let binding = artifacts::gui_owner_binding(&store, &gui, &owner).map_err(|e| format!("{e:?}"))?;
        let observation = if binding.required_owner == "daemon_engine" { Some(artifacts::observe_daemon_gui(&binding).map_err(|e| format!("{e:?}"))?) } else { None };
        Ok(json!({"binding": binding, "native_observation": observation, "browser_loaded_verified": false, "standalone_install_supported": false}))
    }.await)
    }
}

fn native_controller_worker(
    state: &DaemonState,
    worker_id: &str,
) -> refact_buddy_core::operator_types::WorkerIdentity {
    refact_buddy_core::operator_types::WorkerIdentity {
        worker_id: worker_id.into(),
        engine_instance_id: format!("daemon:{}:{}", std::process::id(), state.started_at_ms),
        epoch: state.started_at_ms,
    }
}

fn native_control_response(result: Result<serde_json::Value, String>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => (StatusCode::CONFLICT, Json(json!({"error": error}))).into_response(),
    }
}

async fn preview_operator_policy(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(proposal): Json<crate::http::routers::v1::buddy_observability::PolicyProposal>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let result = async {
        let registry = state.projects.read().await;
        let entry = registry.get(&id).ok_or("registered project unavailable")?;
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: entry.id.clone(),
            canonical_root: entry.root.to_string_lossy().into_owned(),
        };
        let root = entry.root.clone();
        let worker = native_controller_worker(&state, "daemon-native-policy-controller");
        tokio::task::spawn_blocking(move || {
            let (before, charge) = crate::buddy::operator_store::OperatorStore::read_document_policy_snapshot(&root, &project)
                .map_err(|e| e.to_string())?;
            let preview = crate::http::routers::v1::buddy_observability::preview_policy_control_from_snapshot(
                before, charge, project, worker, proposal,
            ).map_err(|e| e.to_string())?;
            Ok(json!(preview))
        }).await.map_err(|e| e.to_string())?
    }.await;
    native_control_response(result)
}

async fn confirm_operator_policy(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::http::routers::v1::buddy_observability::PolicyConfirmation>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let result = async {
        use crate::buddy::operator_store::OperatorStore;
        use refact_buddy_core::operator_types::*;
        let _execution = crate::buddy::resource_execution::execution_gate()
            .lock()
            .await;
        let update = state.update_state.lock().await;
        if state.is_shutting_down() || update_phase_active(&update.phase) {
            return Err("daemon update/shutdown active".into());
        }
        let _drained = state.supervisor.installation_drained().await?;
        let registry = state.projects.read().await;
        let entry = registry.get(&id).ok_or("registered project unavailable")?;
        let project = ProjectIdentity {
            project_id: entry.id.clone(),
            canonical_root: entry.root.to_string_lossy().into_owned(),
        };
        let root = entry.root.clone();
        let worker = native_controller_worker(&state, "daemon-native-policy-controller");
        let requester = NativeSourceIdentity {
            source_kind: NativeSourceKind::External,
            source_id: state
                .config
                .auth
                .username
                .clone()
                .ok_or("user unavailable")?,
            source_version: "daemon-basic-user-v1".into(),
        };
        if request.consent.project != project
            || request.consent.worker != worker
            || !request.confirm
        {
            return Err("exact native user policy confirmation required".into());
        }
        tokio::task::spawn_blocking(move || {
            OperatorStore::read_document_policy_snapshot(&root, &project)
                .map_err(|e| e.to_string())?;
            let mut store = OperatorStore::open(&root, &project).map_err(|e| e.to_string())?;
            let record = crate::http::routers::v1::buddy_observability::confirm_policy_control(
                &mut store, &project, &worker, requester, request,
            )
            .map_err(|e| e.to_string())?;
            Ok(json!(record))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    native_control_response(result)
}

enum HostReleaseControl {
    Dispatch(crate::buddy::improvement::installation::NativeRequest),
    Artifact(crate::buddy::improvement::installation::NativeRequest),
    Receipt(crate::buddy::improvement::installation::release::HostReleaseReceipt),
    Observe(crate::buddy::improvement::installation::release::HostRunningObservation),
}

async fn dispatch_host_release(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::buddy::improvement::installation::NativeRequest>,
) -> Response {
    host_release_control(state, id, headers, HostReleaseControl::Dispatch(request)).await
}

async fn artifact_host_release(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::buddy::improvement::installation::NativeRequest>,
) -> Response {
    host_release_control(state, id, headers, HostReleaseControl::Artifact(request)).await
}

async fn receipt_host_release(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::buddy::improvement::installation::release::HostReleaseReceipt>,
) -> Response {
    host_release_control(state, id, headers, HostReleaseControl::Receipt(request)).await
}

async fn observe_host_release(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::buddy::improvement::installation::release::HostRunningObservation>,
) -> Response {
    host_release_control(state, id, headers, HostReleaseControl::Observe(request)).await
}

#[cfg(target_os = "linux")]
fn host_release_user_binding(
    store: &crate::buddy::operator_store::OperatorStore,
    native: &crate::buddy::improvement::installation::NativeRequest,
    user: &str,
) -> Result<(), crate::buddy::operator_store::OperatorStoreError> {
    use crate::buddy::operator_store::OperatorStoreError;
    use crate::buddy::resource_execution::{ExactActionGrant, ResourceDocument};
    let preparation = store
        .get_document::<serde_json::Value>("prepared_action", &native.prepared_hash)?
        .ok_or(OperatorStoreError::Conflict("native preparation missing"))?;
    let grant = store
        .get_document::<ResourceDocument<ExactActionGrant>>("action_grant", &native.grant_id)?
        .ok_or(OperatorStoreError::Conflict("native grant missing"))?
        .value
        .payload;
    if preparation.value["daemon_prepared_by"].as_str() != Some(user)
        || preparation.value["daemon_preparation_version"] != 1
        || grant.requester.source_id != user
        || grant.spec_hash != native.prepared_hash
        || !crate::buddy::improvement::installation::daemon_grant(&grant, &native.grant_id)
    {
        return Err(OperatorStoreError::Conflict(
            "same native user preparation and grant required",
        ));
    }
    Ok(())
}

async fn host_release_control(
    state: Arc<DaemonState>,
    id: String,
    headers: HeaderMap,
    control: HostReleaseControl,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (id, control);
        StatusCode::NOT_IMPLEMENTED.into_response()
    }
    #[cfg(target_os = "linux")]
    {
        let result = async {
            use crate::buddy::improvement::installation::{NativeRequest, release};
            use crate::buddy::operator_store::OperatorStoreError;
            use crate::buddy::resource_execution::*;
            let _execution = execution_gate().lock().await;
            let update = state.update_state.lock().await;
            if state.is_shutting_down() || update_phase_active(&update.phase) {
                return Err("daemon update/shutdown active".into());
            }
            let _drained = state.supervisor.installation_drained().await?;
            let registry = state.projects.read().await;
            let entry = registry.get(&id).ok_or("registered project unavailable")?;
            let project = refact_buddy_core::operator_types::ProjectIdentity {
                project_id: entry.id.clone(), canonical_root: entry.root.to_string_lossy().into_owned(),
            };
            let worker = native_controller_worker(&state, "daemon-native-host-release");
            let user = state.config.auth.username.clone().ok_or("user unavailable")?;
            let runtime = crate::buddy::outcome_runtime::OutcomeRuntime::open(&entry.root, project, worker)
                .map_err(|e| e.to_string())?;
            let result = match control {
                HostReleaseControl::Dispatch(native) => execute_host_release(&runtime, native, user).await,
                HostReleaseControl::Artifact(native) => runtime.with_store(move |store| {
                    use base64::Engine;
                    host_release_user_binding(store, &native, &user)?;
                    let retained = store.get_document::<release::ReleaseRecord>(release::NAMESPACE, &native.prepared_hash)?
                        .ok_or(OperatorStoreError::Conflict("release not dispatched"))?.value;
                    if retained.request.grant_id != native.grant_id || retained.receipt.is_some() {
                        return Err(OperatorStoreError::Conflict("release already consumed or foreign grant"));
                    }
                    let (request, bytes) = release::prepare_request(store, &native)?;
                    if request != retained.request {
                        return Err(OperatorStoreError::Conflict("dispatched release changed"));
                    }
                    Ok(json!({"request": request, "artifact_base64": base64::prelude::BASE64_STANDARD.encode(bytes)}))
                }).await.map_err(|e| e.to_string()),
                HostReleaseControl::Receipt(receipt) => runtime.with_store(move |store| {
                    let native = NativeRequest { prepared_hash: receipt.prepared_hash.clone(), grant_id: receipt.grant_id.clone() };
                    host_release_user_binding(store, &native, &user)?;
                    Ok(json!(release::record_receipt(store, receipt)?))
                }).await.map_err(|e| e.to_string()),
                HostReleaseControl::Observe(observation) => runtime.with_store(move |store| {
                    let native = NativeRequest { prepared_hash: observation.prepared_hash.clone(), grant_id: observation.grant_id.clone() };
                    host_release_user_binding(store, &native, &user)?;
                    Ok(json!(release::record_running(store, observation)?))
                }).await.map_err(|e| e.to_string()),
            };
            runtime.shutdown().await;
            result
        }.await;
        native_control_response(result)
    }
}

#[cfg(target_os = "linux")]
async fn execute_host_release(
    runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
    native: crate::buddy::improvement::installation::NativeRequest,
    user: String,
) -> Result<serde_json::Value, String> {
    use crate::buddy::action_execution::*;
    use crate::buddy::improvement::installation::release;
    use crate::buddy::resource_execution::*;
    let binding = native.clone();
    runtime
        .with_store(move |store| host_release_user_binding(store, &binding, &user))
        .await
        .map_err(|e| e.to_string())?;
    let prepared = load_prepared(runtime, native.prepared_hash.clone()).await?;
    validate_prepared_draft(runtime, &prepared).await?;
    let grant = load_grant(runtime, native.grant_id.clone())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("grant unavailable")?;
    if !grant.authorize(&prepared.spec, &grant.requester, grant.decision.as_ref()) {
        return Err("explicit exact daemon user grant unavailable".into());
    }
    let key = native.prepared_hash.clone();
    runtime
        .with_store(move |store| crate::buddy::desk_actions::validate_resource_origin(store, &key))
        .await
        .map_err(|e| e.to_string())?;
    let action = ActionRequest {
        execution_id: format!("native-host-release:{}", native.prepared_hash),
        idempotency_key: format!("native-host-release:{}", native.prepared_hash),
        spec: prepared.spec.clone(),
        bound_parameter_ref: prepared.bound_parameter_ref,
        requester: grant.requester.clone(),
        decision: grant.decision.clone(),
        worker: runtime.worker(),
    };
    let mut catalog = ResourceCatalog::default();
    catalog
        .register(
            release::ACTION.into(),
            Arc::new(release::ReleaseAdapter {
                runtime: runtime.clone(),
                native: native.clone(),
                spec: action.spec.clone(),
            }),
        )
        .map_err(|e| format!("{e:?}"))?;
    let outcome = catalog
        .service(runtime.clone(), Arc::new(grant))
        .map_err(|_| "native catalog unavailable")?
        .execute_async(&action)
        .await;
    let record = runtime
        .with_store(move |store| {
            store.get_document::<release::ReleaseRecord>(release::NAMESPACE, &native.prepared_hash)
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"action_result": outcome, "native_observation": record.map(|r| r.value)}))
}

async fn update_status(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
) -> Json<DaemonUpdateState> {
    Json(state.update_state.lock().await.clone())
}

fn update_phase_active(phase: &UpdatePhase) -> bool {
    matches!(
        phase,
        UpdatePhase::Checking | UpdatePhase::Downloading | UpdatePhase::Restarting
    )
}

async fn set_update_phase(
    state: &DaemonState,
    phase: UpdatePhase,
    detail: Option<String>,
    target_version: Option<String>,
) {
    let mut update_state = state.update_state.lock().await;
    update_state.phase = phase;
    update_state.detail = detail;
    if target_version.is_some() {
        update_state.target_version = target_version;
    }
}

async fn workers(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
) -> Json<Vec<crate::daemon::state::WorkerRow>> {
    Json(state.worker_rows().await)
}

async fn cron_status(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
) -> Json<crate::daemon::cron_clock::CronClockStatus> {
    Json(crate::daemon::cron_clock::status(&state).await)
}

async fn logs(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let path = match log_path(&state, query.project_id.as_deref()).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    match tail_file(&path, query.tail.clamp(1, 10_000)).await {
        Ok(text) => ([("content-type", "text/plain; charset=utf-8")], text).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
    bind: String,
    lan_enabled: bool,
    mdns_enabled: bool,
    auth_enabled: bool,
    username: Option<String>,
    has_password: bool,
    hostname_local: String,
    urls: crate::daemon::state::DaemonUrls,
}

#[derive(Debug, Deserialize)]
struct UpdateSettingsRequest {
    lan_enabled: bool,
    mdns_enabled: bool,
    auth_enabled: bool,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

fn bind_is_loopback(bind: &str) -> bool {
    bind.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

async fn get_settings(
    State((state, port)): State<(Arc<DaemonState>, u16)>,
) -> Json<SettingsResponse> {
    let info = state.daemon_info(port, state.config.bind.clone());
    let lan_enabled = !bind_is_loopback(&state.config.bind);
    let mdns_enabled = state.config.mdns.enabled.unwrap_or(lan_enabled);
    Json(SettingsResponse {
        bind: state.config.bind.clone(),
        lan_enabled,
        mdns_enabled,
        auth_enabled: state.config.auth.enabled,
        username: state.config.auth.username.clone(),
        has_password: state
            .config
            .auth
            .password
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        hostname_local: info.hostname_local,
        urls: info.urls,
    })
}

async fn update_settings(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Response {
    let mut config = state.config.clone();
    config.bind = if request.lan_enabled {
        "0.0.0.0".to_string()
    } else {
        "127.0.0.1".to_string()
    };
    config.mdns.enabled = Some(request.mdns_enabled);
    config.auth.enabled = request.auth_enabled;
    config.auth.username = request
        .username
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(password) = request.password.filter(|value| !value.is_empty()) {
        config.auth.password = Some(password);
    }
    if request.lan_enabled {
        if !config.auth.enabled {
            return settings_error("Enabling LAN access requires authentication to be enabled.");
        }
        let has_username = config
            .auth
            .username
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false);
        let has_password = config
            .auth
            .password
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false);
        if !has_username || !has_password {
            return settings_error("Enabling LAN access requires a username and password.");
        }
    }
    if config.auth.enabled
        && config
            .auth
            .token
            .as_deref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
    {
        config.auth.token = Some(crate::daemon::auth::generate_token());
    }
    if let Err(error) =
        crate::daemon::config::save_to_path(&config, &crate::daemon::paths::daemon_config_path())
            .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response();
    }
    if let Err(error) = spawn_daemon_relaunch(&state) {
        tracing::warn!("settings relaunch failed: {error}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("settings saved, but daemon restart failed: {error}")
            })),
        )
            .into_response();
    }
    state.request_shutdown("settings_changed".to_string());
    Json(json!({"success": true, "restarting": true})).into_response()
}

fn settings_error(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response()
}

#[cfg(test)]
fn spawn_daemon_relaunch(_state: &DaemonState) -> Result<(), String> {
    Ok(())
}

#[cfg(not(test))]
fn spawn_daemon_relaunch(state: &DaemonState) -> Result<(), String> {
    let mut command = daemon_relaunch_command(state.relaunch_executable()?);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("spawn failed: {error}"))
}

fn daemon_relaunch_command(executable: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(executable);
    command
        .arg("daemon")
        .env("REFACT_DAEMON_RELAUNCH", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

async fn logs_stream(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let path = match log_path(&state, query.project_id.as_deref()).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    let initial_tail = query.tail.clamp(1, 10_000);
    let mut shutdown_rx = state.shutdown_receiver();
    let stream = async_stream::stream! {
        let initial = tail_file(&path, initial_tail).await.unwrap_or_default();
        for line in initial.lines() {
            yield Ok::<Event, Infallible>(Event::default().data(line.to_string()));
        }
        let mut cursor = LogTailCursor::default();
        if let Ok(mut file) = tokio::fs::File::open(&path).await {
            if let Ok(meta) = file.metadata().await {
                let len = meta.len();
                cursor.head = log_head(&mut file, len).await;
                cursor.offset = len;
            }
        }
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = tokio::time::sleep(Duration::from_millis(1000)) => {
                    let (text, next, error) = read_log_tail(&path, cursor).await;
                    cursor = next;
                    if let Some(error) = error {
                        yield Ok(Event::default().data(format!("[refact: {error}]")));
                        continue;
                    }
                    if !text.is_empty() {
                        let redacted = crate::daemon::auth::redact_daemon_query_token(&text);
                        for line in redacted.lines() {
                            yield Ok(Event::default().data(line.to_string()));
                        }
                    }
                }
            }
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

const LOG_STREAM_READ_CAP: u64 = 256 * 1024;
const LOG_STREAM_HEAD_LEN: u64 = 256;

#[derive(Default)]
struct LogTailCursor {
    offset: u64,
    head: Vec<u8>,
}

async fn log_head(file: &mut tokio::fs::File, len: u64) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let want = len.min(LOG_STREAM_HEAD_LEN);
    if want == 0 {
        return Vec::new();
    }
    if file.seek(std::io::SeekFrom::Start(0)).await.is_err() {
        return Vec::new();
    }
    let mut buf = vec![0u8; want as usize];
    match file.read_exact(&mut buf).await {
        Ok(_) => buf,
        Err(_) => Vec::new(),
    }
}

async fn read_log_tail(
    path: &std::path::Path,
    cursor: LogTailCursor,
) -> (String, LogTailCursor, Option<String>) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (String::new(), LogTailCursor::default(), None);
        }
        Err(error) => {
            return (
                String::new(),
                cursor,
                Some(format!("failed to open log: {error}")),
            )
        }
    };
    let len = match file.metadata().await {
        Ok(meta) => meta.len(),
        Err(error) => {
            return (
                String::new(),
                cursor,
                Some(format!("failed to stat log: {error}")),
            )
        }
    };
    let head = log_head(&mut file, len).await;
    let rotated = !cursor.head.is_empty() && (head != cursor.head || len < cursor.offset);
    let start = if rotated { 0 } else { cursor.offset.min(len) };
    if len <= start {
        return (String::new(), LogTailCursor { offset: len, head }, None);
    }
    let to_read = (len - start).min(LOG_STREAM_READ_CAP);
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return (
            String::new(),
            LogTailCursor {
                offset: start,
                head,
            },
            Some("failed to seek log".to_string()),
        );
    }
    let mut buf = vec![0u8; to_read as usize];
    if let Err(error) = file.read_exact(&mut buf).await {
        return (
            String::new(),
            LogTailCursor {
                offset: start,
                head,
            },
            Some(format!("failed to read log: {error}")),
        );
    }
    (
        String::from_utf8_lossy(&buf).to_string(),
        LogTailCursor {
            offset: start + to_read,
            head,
        },
        None,
    )
}

/// User credentials are deliberately distinct from daemon/worker/hook bearer tokens.
fn buddy_user_authorized(state: &DaemonState, headers: &HeaderMap) -> bool {
    use base64::Engine;
    if !state.config.auth.enabled || headers.get_all("authorization").iter().count() != 1 {
        return false;
    }
    let (Some(user), Some(password)) = (
        state.config.auth.username.as_deref(),
        state.config.auth.password.as_deref(),
    ) else {
        return false;
    };
    if user.is_empty() || password.is_empty() {
        return false;
    }
    let Some(encoded) = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Basic "))
    else {
        return false;
    };
    let Ok(decoded) = base64::prelude::BASE64_STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };
    let Some((actual_user, actual_pass)) = decoded.split_once(':') else {
        return false;
    };
    crate::daemon::auth::token_matches(actual_user, user)
        && crate::daemon::auth::token_matches(actual_pass, password)
}

async fn buddy_wake_get(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.buddy_wake_grant(&id).await {
        Ok(grant) => Json(json!({"grant": grant, "authority": "worker_start_only", "goal_authority": false, "tool_consent": false})).into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn buddy_wake_issue(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(request): Json<crate::daemon::buddy_wake::WakeGrantRequest>,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if state.is_shutting_down() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match state
        .issue_buddy_wake_grant(&id, request, state.config.auth.username.clone().unwrap())
        .await
    {
        Ok(grant) => Json(json!({"grant": grant})).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response(),
    }
}

async fn buddy_wake_revoke(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if !buddy_user_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.revoke_buddy_wake_grant(&id).await {
        Ok(()) => Json(json!({"revoked": true, "running_worker_stopped": false})).into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

fn buddy_transport_authorized(state: &DaemonState, headers: &HeaderMap) -> bool {
    let expected = state
        .auth_token
        .as_deref()
        .or_else(|| state.config.hooks.token.as_deref())
        .filter(|v| !v.is_empty());
    expected
        .zip(crate::daemon::auth::bearer_token_from_headers(headers))
        .is_some_and(|(expected, actual)| crate::daemon::auth::token_matches(actual, expected))
}

async fn buddy_worker_projection(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    headers: HeaderMap,
    Json(report): Json<crate::daemon::buddy_reconciliation::WorkerProjection>,
) -> Response {
    if !buddy_transport_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.accept_buddy_projection(report).await {
        Ok(changed) => {
            Json(json!({"accepted": true, "changed": changed, "wake_authorized": false}))
                .into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response(),
    }
}

async fn buddy_project_projection(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if !buddy_transport_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.buddy_projection(&id).await {
        Ok(Some(observation)) => Json(json!({"observation": observation, "daemon_wake_supported": true, "coverage": "Observed references only; offline gaps are unknown, not completion. Worker must verify live mandate; no grant, goal, tool consent or question replay."})).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": error}))).into_response(),
    }
}

async fn worker_status(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(report): Json<crate::daemon_link::WorkerStatusReport>,
) -> Response {
    match state.store_validated_worker_status(report).await {
        Ok(event_emitted) => {
            Json(json!({"success": true, "event_emitted": event_emitted})).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "error": error})),
        )
            .into_response(),
    }
}

async fn events(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = query.follow.then(|| state.events.subscribe());
    let cursor = event_cursor(&headers, &query).unwrap_or(0);
    let initial = state.events.replay_after(cursor).await;
    let events = state.events.clone();
    let mut shutdown_rx = state.shutdown_receiver();
    let stream = async_stream::stream! {
        let mut last_seq = cursor;
        if let Some(gap) = initial.gap {
            let event = resync_event(&gap);
            if event.seq > last_seq {
                last_seq = event.seq;
                yield Ok(sse_event(&event));
            }
        }
        for event in initial.events {
            if event.seq > last_seq {
                last_seq = event.seq;
                yield Ok(sse_event(&event));
            }
        }
        if let Some(mut rx) = rx {
            loop {
                tokio::select! {
                    result = rx.recv() => match result {
                        Ok(event) => {
                            if event.seq > last_seq {
                                last_seq = event.seq;
                                yield Ok(sse_event(&event));
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let replay = events.replay_after(last_seq).await;
                            if let Some(gap) = replay.gap {
                                let event = resync_event(&gap);
                                if event.seq > last_seq {
                                    last_seq = event.seq;
                                    yield Ok(sse_event(&event));
                                }
                            }
                            for event in replay.events {
                                if event.seq > last_seq {
                                    last_seq = event.seq;
                                    yield Ok(sse_event(&event));
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn event_cursor(headers: &HeaderMap, query: &EventsQuery) -> Option<u64> {
    query
        .after_seq
        .or(query.last_event_id)
        .or_else(|| header_seq(headers, "last-event-id"))
}

fn header_seq(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

fn resync_event(gap: &crate::daemon::events::EventReplayGap) -> crate::daemon::events::DaemonEvent {
    crate::daemon::events::DaemonEvent {
        seq: gap.oldest_seq.saturating_sub(1),
        ts_ms: crate::daemon::state::now_ms(),
        kind: "daemon_events_resync_required".to_string(),
        project_id: None,
        payload: json!({
            "requested_after_seq": gap.requested_after_seq,
            "oldest_seq": gap.oldest_seq,
            "latest_seq": gap.latest_seq,
        }),
    }
}

async fn log_path(state: &Arc<DaemonState>, project_id: Option<&str>) -> Result<PathBuf, Response> {
    let Some(project_id) = project_id.filter(|value| !value.is_empty()) else {
        return Ok(state.daemon_dir.join("logs").join("daemon.log"));
    };
    let registry = state.projects.read().await;
    match registry.get(project_id) {
        Some(entry) => Ok(state
            .daemon_dir
            .join("logs")
            .join(format!("worker-{}.log", entry.slug))),
        None => Err((StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()),
    }
}

async fn tail_file(path: &std::path::Path, tail: usize) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("failed to open {}: {error}", path.display())),
    };
    let len = file
        .metadata()
        .await
        .map_err(|error| format!("failed to stat {}: {error}", path.display()))?
        .len();
    let start = len.saturating_sub(1024 * 1024);
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|error| format!("failed to seek {}: {error}", path.display()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&buf);
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(tail);
    let mut out = lines[start..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(crate::daemon::auth::redact_daemon_query_token(&out))
}

fn sse_event(event: &crate::daemon::events::DaemonEvent) -> Event {
    Event::default()
        .id(event.seq.to_string())
        .event("daemon")
        .data(serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string()))
}

async fn wait_for_shutdown(state: Arc<DaemonState>) {
    let mut rx = state.shutdown_receiver();
    let _ = rx.recv().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectRegistry;

    async fn native_router_fixture() -> (
        tempfile::TempDir,
        Arc<DaemonState>,
        refact_buddy_core::operator_types::ProjectIdentity,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = DaemonConfig::default();
        config.auth.enabled = true;
        config.auth.username = Some("native-user".into());
        config.auth.password = Some("native-password".into());
        let state = DaemonState::new_with_daemon_dir(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("worker-token".into()),
            dir.path().join("daemon"),
            0,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let entry = state.projects.write().await.open(root).await.unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: entry.id,
            canonical_root: entry.root.to_string_lossy().into_owned(),
        };
        (dir, state, project)
    }

    async fn native_post(
        router: Router,
        uri: &str,
        authorization: &str,
        body: serde_json::Value,
    ) -> Response {
        use tower::ServiceExt;
        let mut request = hyper::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", authorization)
            .body(hyper::Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(SocketAddr::from((
                [127, 0, 0, 1],
                40000,
            ))));
        router.oneshot(request).await.unwrap()
    }

    fn native_basic() -> String {
        use base64::Engine;
        format!(
            "Basic {}",
            base64::prelude::BASE64_STANDARD.encode("native-user:native-password")
        )
    }

    #[tokio::test]
    async fn policy_router_preview_no_documents_and_exact_raise_replay() {
        use crate::buddy::operator_store::OperatorStore;
        let (_dir, state, project) = native_router_fixture().await;
        let root = std::path::Path::new(&project.canonical_root);
        drop(OperatorStore::open(root, &project).unwrap());
        let before = OperatorStore::read_document_policy_snapshot(root, &project).unwrap();
        let router = make_router(state.clone(), 0);
        let preview_uri = format!(
            "/daemon/v1/projects/{}/operator/policy/preview",
            project.project_id
        );
        let confirm_uri = format!(
            "/daemon/v1/projects/{}/operator/policy/confirm",
            project.project_id
        );
        let proposal = json!({"project_id": project.project_id, "policy": {"max_rows_per_namespace": 8, "replace_editorial_aggregate": false}});
        assert_eq!(
            native_post(
                router.clone(),
                &preview_uri,
                "Bearer worker-token",
                proposal.clone()
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let (status, preview) = response_json(
            native_post(router.clone(), &preview_uri, &native_basic(), proposal).await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(
            OperatorStore::read_document_policy_snapshot(root, &project).unwrap(),
            before
        );
        assert_eq!(
            preview["consent"]["worker"]["worker_id"],
            "daemon-native-policy-controller"
        );
        let request = json!({"consent": preview["consent"], "expected_digest": preview["digest"], "idempotency_key": "native-policy", "confirm": true});
        assert_eq!(
            native_post(
                router.clone(),
                &confirm_uri,
                "Bearer worker-token",
                request.clone()
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let (status, receipt) = response_json(
            native_post(
                router.clone(),
                &confirm_uri,
                &native_basic(),
                request.clone(),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        let (status, replay) = response_json(
            native_post(
                router.clone(),
                &confirm_uri,
                &native_basic(),
                request.clone(),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(receipt, replay);
        let mut changed = request;
        changed["consent"]["worker"]["epoch"] = json!(state.started_at_ms + 1);
        assert_eq!(
            native_post(router, &confirm_uri, &native_basic(), changed)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!state.is_shutting_down());
        assert_eq!(state.supervisor.worker_count().await, 0);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_release_router_native_prepare_grant_bind_same_worker() {
        use crate::buddy::{operator_store::OperatorStore, improvement::installation::release::*};
        use refact_buddy_core::operator_types::*;
        let (_dir, state, project) = native_router_fixture().await;
        let draft = ReleaseDraft {
            recovery_ref: None,
            project: project.clone(),
            candidate_id: "unproven-fixture".into(),
            candidate_digest: "b".repeat(64),
            proof_id: "missing-proof".into(),
            artifact_digest: "c".repeat(64),
            host_session_id: "native-host".into(),
            target: HostTarget::Vscode,
            operation: HostOperation::Install,
            expected_installed_manifest_digest: "d".repeat(64),
            expected_running_manifest_digest: "e".repeat(64),
        };
        let version = crate::buddy::improvement_evaluation::digest(&draft).unwrap();
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "fixture-release".into(),
            kind: ACTION.into(),
            adapter: ADAPTER.into(),
            project: project.clone(),
            resource: ResourceRef {
                resource_kind: ADAPTER.into(),
                resource_id: "vscode".into(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("fixture-release-draft".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: false,
                can_verify: false,
                can_recover: false,
                can_cancel: false,
            },
        };
        {
            let mut store =
                OperatorStore::open(std::path::Path::new(&project.canonical_root), &project)
                    .unwrap();
            store
                .put_document(
                    "resource_draft",
                    "fixture-release-draft",
                    &json!({"schema_version":1,"project":project,"payload":draft}),
                    None,
                )
                .unwrap();
        }
        let router = make_router(state.clone(), 0);
        let base = format!("/daemon/v1/projects/{}/host-release", project.project_id);
        assert_eq!(
            native_post(
                router.clone(),
                &format!("{base}/prepare"),
                "Bearer worker-token",
                json!({"spec":spec})
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let (status, prepared) = response_json(
            native_post(
                router.clone(),
                &format!("{base}/prepare"),
                &native_basic(),
                json!({"spec":spec}),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prepared}");
        assert_eq!(
            prepared["prepared"]["spec"]["capabilities"]["can_execute"],
            true
        );
        let hash = prepared["prepared"]["spec_hash"].as_str().unwrap();
        let (status, grant) = response_json(
            native_post(
                router.clone(),
                &format!("{base}/grant"),
                &native_basic(),
                json!({"prepared_hash":hash,"expires_in_seconds":60}),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{grant}");
        assert_eq!(
            grant["grant"]["decision"]["worker"],
            json!(native_controller_worker(
                &state,
                "daemon-native-host-release"
            ))
        );
        assert_eq!(grant["grant"]["requester"]["source_id"], "native-user");
        let native = json!({"prepared_hash":hash,"grant_id":grant["grant_id"]});
        assert_eq!(
            native_post(
                router.clone(),
                &format!("{base}/artifact"),
                &native_basic(),
                native.clone()
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        // Preparation/consent is not compiler proof: the real ActionService must
        // fail closed on this deliberately unproven candidate, without a release.
        let response = native_post(
            router.clone(),
            &format!("{base}/dispatch"),
            &native_basic(),
            native.clone(),
        )
        .await;
        assert!(response.status() == StatusCode::OK || response.status() == StatusCode::CONFLICT);
        assert_eq!(
            native_post(router, &format!("{base}/artifact"), &native_basic(), native)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        let store =
            OperatorStore::open(std::path::Path::new(&project.canonical_root), &project).unwrap();
        assert!(store
            .get_document::<ReleaseRecord>(NAMESPACE, hash)
            .unwrap()
            .is_none());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_release_router_dispatch_fence_immutable_receipt_exact_reconcile() {
        use crate::buddy::{operator_store::OperatorStore, improvement::installation::release::*};
        let (_dir, state, project) = native_router_fixture().await;
        let root = std::path::Path::new(&project.canonical_root);
        let hash = "a".repeat(64);
        let request = HostReleaseRequest {
            schema_version: 1,
            prepared_hash: hash.clone(),
            grant_id: "fixture-grant".into(),
            project_id: project.project_id.clone(),
            host_session_id: "old-host".into(),
            target: HostTarget::Vscode,
            operation: HostOperation::Install,
            candidate_id: "candidate".into(),
            candidate_digest: "b".repeat(64),
            proof_id: "proof".into(),
            artifact_digest: "c".repeat(64),
            manifest_digest: "d".repeat(64),
            package_version: "1.2.3".into(),
            expected_installed_manifest_digest: "e".repeat(64),
            expected_running_manifest_digest: "e".repeat(64),
            files: std::collections::BTreeMap::new(),
        };
        {
            let mut store = OperatorStore::open(root, &project).unwrap();
            store
                .put_document(
                    "prepared_action",
                    &hash,
                    &json!({"schema_version":1,"project":project,"daemon_prepared_by":"native-user","daemon_preparation_version":1}),
                    None,
                )
                .unwrap();
        }
        {
            use refact_buddy_core::operator_types::*;
            let mut store = OperatorStore::open(root, &project).unwrap();
            let grant = crate::buddy::resource_execution::ExactActionGrant {
                native_rights: None,
                recovery_execution_id: None,
                spec_hash: hash.clone(),
                requester: NativeSourceIdentity {
                    source_kind: NativeSourceKind::External,
                    source_id: "native-user".into(),
                    source_version: "daemon-basic-user-v1".into(),
                },
                decision: Some(DecisionReference {
                    project: project.clone(),
                    worker: native_controller_worker(&state, "daemon-native-host-release"),
                    native: NativeSourceIdentity {
                        source_kind: NativeSourceKind::External,
                        source_id: "fixture-grant".into(),
                        source_version: "daemon-installation-grant-v1".into(),
                    },
                }),
                expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
                revoked: false,
            };
            store
                .put_document(
                    "action_grant",
                    "fixture-grant",
                    &json!({"schema_version":1,"project":project,"payload":grant}),
                    None,
                )
                .unwrap();
        }
        let router = make_router(state.clone(), 0);
        let base = format!("/daemon/v1/projects/{}/host-release", project.project_id);
        let native = json!({"prepared_hash":hash,"grant_id":"fixture-grant"});
        assert_eq!(
            native_post(
                router.clone(),
                &format!("{base}/artifact"),
                &native_basic(),
                native.clone()
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let receipt = HostReleaseReceipt {
            schema_version: 1,
            prepared_hash: hash.clone(),
            grant_id: request.grant_id.clone(),
            host_session_id: request.host_session_id.clone(),
            artifact_digest: request.artifact_digest.clone(),
            status: ReceiptStatus::InstallerReturned,
            installed: Some(InstalledIdentity {
                manifest_digest: request.manifest_digest.clone(),
                version: request.package_version.clone(),
            }),
            running: None,
        };
        let observation = HostRunningObservation {
            schema_version: 1,
            prepared_hash: hash.clone(),
            grant_id: request.grant_id.clone(),
            host_session_id: request.host_session_id.clone(),
            artifact_digest: request.artifact_digest.clone(),
            installed: InstalledIdentity {
                manifest_digest: request.manifest_digest.clone(),
                version: request.package_version.clone(),
            },
            running: RunningIdentity {
                manifest_digest: request.manifest_digest.clone(),
                version: request.package_version.clone(),
                host_session_id: "fresh-host".into(),
            },
        };
        for (endpoint, body) in [
            ("artifact", native.clone()),
            ("dispatch", native.clone()),
            (
                "grant",
                json!({"prepared_hash":hash,"expires_in_seconds":60}),
            ),
            ("receipt", json!(receipt)),
            ("observe", json!(observation)),
        ] {
            for auth in ["Bearer worker-token", "Basic Zm9yZWlnbjpwdw=="] {
                assert_eq!(
                    native_post(
                        router.clone(),
                        &format!("{base}/{endpoint}"),
                        auth,
                        body.clone()
                    )
                    .await
                    .status(),
                    StatusCode::UNAUTHORIZED,
                    "{endpoint}"
                );
            }
        }
        {
            let mut store = OperatorStore::open(root, &project).unwrap();
            store
                .reserve_native_documents("host_release", &hash, &[(NAMESPACE, hash.clone(), 3)])
                .unwrap();
            store
                .put_reserved_document(
                    "host_release",
                    &hash,
                    NAMESPACE,
                    &hash,
                    &ReleaseRecord {
                        schema_version: 1,
                        project: project.clone(),
                        request,
                        receipt: None,
                        installed_verified: false,
                        running_verified: false,
                        running_observation: None,
                    },
                    None,
                )
                .unwrap();
        }
        let (status, installed) = response_json(
            native_post(
                router.clone(),
                &format!("{base}/receipt"),
                &native_basic(),
                json!(receipt),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{installed}");
        assert_eq!(installed["installed_verified"], true);
        assert_eq!(installed["running_verified"], false);
        let mut changed = receipt.clone();
        changed.status = ReceiptStatus::Declined;
        assert_eq!(
            native_post(
                router.clone(),
                &format!("{base}/receipt"),
                &native_basic(),
                json!(changed)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            native_post(
                router.clone(),
                &format!("{base}/artifact"),
                &native_basic(),
                native
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let mut wrong = observation.clone();
        wrong.running.manifest_digest = "e".repeat(64);
        assert_eq!(
            native_post(
                router.clone(),
                &format!("{base}/observe"),
                &native_basic(),
                json!(wrong)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let (status, running) = response_json(
            native_post(
                router.clone(),
                &format!("{base}/observe"),
                &native_basic(),
                json!(observation),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{running}");
        assert_eq!(running["running_verified"], true);
        assert_eq!(running["receipt"], json!(receipt));
        assert!(!state.is_shutting_down());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_release_router_evaluated_packages_restart_and_known_good_recovery() {
        use crate::buddy::{improvement::installation::release::*, operator_store::OperatorStore};
        use refact_buddy_core::operator_types::*;
        for target in [HostTarget::Vscode, HostTarget::Jetbrains] {
            let (_dir, state, project) = native_router_fixture().await;
            let root = std::path::Path::new(&project.canonical_root);
            let runtime = crate::buddy::outcome_runtime::OutcomeRuntime::open(
                root,
                project.clone(),
                native_controller_worker(&state, "daemon-native-host-release"),
            )
            .unwrap();
            let (mut good, good_bytes) =
                crate::buddy::improvement::installation::release::tests::evaluated_package(
                    &runtime,
                    target.clone(),
                    "1.0.0",
                )
                .await;
            let (mut next, next_bytes) =
                crate::buddy::improvement::installation::release::tests::evaluated_package(
                    &runtime,
                    target.clone(),
                    "2.0.0",
                )
                .await;
            // Obtain the physical owner manifest from real ZIP inspection, not bundle existence.
            let known_good = runtime
                .with_store(move |store| {
                    let candidate = crate::buddy::improvement::activation_gate(
                        store,
                        &good.candidate_id,
                        &good.candidate_digest,
                        &good.proof_id,
                    )?;
                    let manifest =
                        crate::buddy::improvement::installation::release::tests::package_manifest(
                            store, &candidate,
                        );
                    Ok((good, manifest))
                })
                .await
                .unwrap();
            good = known_good.0;
            next.expected_installed_manifest_digest = known_good.1.clone();
            next.expected_running_manifest_digest = known_good.1;
            runtime.shutdown().await;
            let base = format!("/daemon/v1/projects/{}/host-release", project.project_id);
            let mut original: Option<(String, Option<HostReleaseReceipt>)> = None;
            let mut install_count = 0;
            for (index, bytes) in [next_bytes, good_bytes].into_iter().enumerate() {
                let draft = if index == 0 {
                    next.clone()
                } else {
                    good.clone()
                };
                let version = crate::buddy::improvement_evaluation::digest(&draft).unwrap();
                let reference = format!("release-draft-{index}");
                let spec = ActionSpec {
                    schema_version: 1,
                    action_id: format!("release-{index}"),
                    kind: ACTION.into(),
                    adapter: ADAPTER.into(),
                    project: project.clone(),
                    resource: ResourceRef {
                        resource_kind: ADAPTER.into(),
                        resource_id: match target {
                            HostTarget::Vscode => "vscode",
                            HostTarget::Jetbrains => "jetbrains",
                        }
                        .into(),
                        version: version.clone(),
                    },
                    expected_version: version,
                    parameter_ref: Some(reference.clone()),
                    availability: ActionAvailability::Available,
                    confirmation: ConfirmationClass::UserConfirmationRequired,
                    authority_class: AuthorityClass::Mutating,
                    capabilities: ActionCapabilities {
                        can_preview: true,
                        can_execute: false,
                        can_verify: false,
                        can_recover: false,
                        can_cancel: false,
                    },
                };
                {
                    let mut store = OperatorStore::open(root, &project).unwrap();
                    store
                        .put_document(
                            "resource_draft",
                            &reference,
                            &json!({"schema_version":1,"project":project,"payload":draft}),
                            None,
                        )
                        .unwrap();
                }
                // New router for each operation also exercises reconnect with retained store.
                let router = make_router(state.clone(), 0);
                let (status, prepared) = response_json(
                    native_post(
                        router.clone(),
                        &format!("{base}/prepare"),
                        &native_basic(),
                        json!({"spec":spec}),
                    )
                    .await,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{prepared}");
                let hash = prepared["prepared"]["spec_hash"].as_str().unwrap();
                let (status, grant) = response_json(
                    native_post(
                        router.clone(),
                        &format!("{base}/grant"),
                        &native_basic(),
                        json!({"prepared_hash":hash,"expires_in_seconds":3600}),
                    )
                    .await,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{grant}");
                assert_eq!(grant["grant"]["requester"]["source_id"], "native-user");
                let native = json!({"prepared_hash":hash,"grant_id":grant["grant_id"]});
                let (status, dispatch) = response_json(
                    native_post(
                        router.clone(),
                        &format!("{base}/dispatch"),
                        &native_basic(),
                        native.clone(),
                    )
                    .await,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{dispatch}");
                assert_eq!(dispatch["native_observation"]["running_verified"], false);
                let (status, fetched) = response_json(
                    native_post(
                        router.clone(),
                        &format!("{base}/artifact"),
                        &native_basic(),
                        native.clone(),
                    )
                    .await,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{fetched}");
                use base64::Engine;
                assert_eq!(
                    base64::prelude::BASE64_STANDARD
                        .decode(fetched["artifact_base64"].as_str().unwrap())
                        .unwrap(),
                    bytes
                );
                let request: HostReleaseRequest =
                    serde_json::from_value(fetched["request"].clone()).unwrap();
                assert_eq!(request.target, target);
                assert_eq!(
                    request.operation,
                    if index == 0 {
                        HostOperation::Install
                    } else {
                        HostOperation::Recover
                    }
                );
                assert!(!request.files.is_empty());
                // Inject only native installer return. No host API, reload or installation runs.
                install_count += 1;
                let receipt = HostReleaseReceipt {
                    schema_version: 1,
                    prepared_hash: hash.into(),
                    grant_id: request.grant_id.clone(),
                    host_session_id: request.host_session_id.clone(),
                    artifact_digest: request.artifact_digest.clone(),
                    status: ReceiptStatus::InstallerReturned,
                    installed: Some(InstalledIdentity {
                        manifest_digest: request.expected_installed_manifest_digest.clone(),
                        version: "restart-required-old".into(),
                    }),
                    running: Some(RunningIdentity {
                        manifest_digest: request.expected_running_manifest_digest.clone(),
                        version: "restart-required-old".into(),
                        host_session_id: request.host_session_id.clone(),
                    }),
                };
                let (status, pending) = response_json(
                    native_post(
                        router.clone(),
                        &format!("{base}/receipt"),
                        &native_basic(),
                        json!(receipt),
                    )
                    .await,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{pending}");
                assert_eq!(pending["installed_verified"], false);
                assert_eq!(pending["running_verified"], false);
                let observation = HostRunningObservation {
                    schema_version: 1,
                    prepared_hash: hash.into(),
                    grant_id: request.grant_id.clone(),
                    host_session_id: request.host_session_id.clone(),
                    artifact_digest: request.artifact_digest.clone(),
                    installed: InstalledIdentity {
                        manifest_digest: request.manifest_digest.clone(),
                        version: request.package_version.clone(),
                    },
                    running: RunningIdentity {
                        manifest_digest: request.manifest_digest.clone(),
                        version: request.package_version.clone(),
                        host_session_id: format!("fresh-host-{index}"),
                    },
                };
                for field in [
                    "prepared_hash",
                    "grant_id",
                    "artifact_digest",
                    "host_session_id",
                    "version",
                    "manifest",
                    "session",
                ] {
                    let mut wrong = json!(observation);
                    match field {
                        "version" => wrong["running"]["version"] = json!("wrong"),
                        "manifest" => wrong["installed"]["manifest_digest"] = json!("f".repeat(64)),
                        "session" => {
                            wrong["running"]["host_session_id"] = json!(request.host_session_id)
                        }
                        _ => wrong[field] = json!("foreign"),
                    }
                    assert_eq!(
                        native_post(
                            router.clone(),
                            &format!("{base}/observe"),
                            &native_basic(),
                            wrong
                        )
                        .await
                        .status(),
                        StatusCode::CONFLICT,
                        "{field}"
                    );
                }
                assert_eq!(
                    native_post(
                        router.clone(),
                        &format!("{base}/observe"),
                        "Basic Zm9yZWlnbjpwdw==",
                        json!(observation)
                    )
                    .await
                    .status(),
                    StatusCode::UNAUTHORIZED
                );
                let (status, final_record) = response_json(
                    native_post(
                        make_router(state.clone(), 0),
                        &format!("{base}/observe"),
                        &native_basic(),
                        json!(observation),
                    )
                    .await,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{final_record}");
                assert_eq!(final_record["installed_verified"], true);
                assert_eq!(final_record["running_verified"], true);
                assert_eq!(final_record["receipt"], json!(receipt));
                assert_eq!(
                    native_post(
                        router.clone(),
                        &format!("{base}/artifact"),
                        &native_basic(),
                        native.clone()
                    )
                    .await
                    .status(),
                    StatusCode::CONFLICT
                );
                let (_, replay) = response_json(
                    native_post(
                        router.clone(),
                        &format!("{base}/dispatch"),
                        &native_basic(),
                        native,
                    )
                    .await,
                )
                .await;
                assert_eq!(replay["native_observation"]["receipt"], json!(receipt));
                let store = OperatorStore::open(root, &project).unwrap();
                let retained = store
                    .get_document::<ReleaseRecord>(NAMESPACE, hash)
                    .unwrap()
                    .unwrap()
                    .value;
                assert_eq!(retained.receipt, Some(receipt));
                if let Some((old_hash, old_receipt)) = &original {
                    assert_eq!(
                        store
                            .get_document::<ReleaseRecord>(NAMESPACE, old_hash)
                            .unwrap()
                            .unwrap()
                            .value
                            .receipt,
                        *old_receipt
                    );
                } else {
                    original = Some((hash.to_string(), retained.receipt.clone()));
                    good.operation = HostOperation::Recover;
                    good.recovery_ref = Some(hash.into());
                    good.host_session_id = observation.running.host_session_id;
                    good.expected_installed_manifest_digest = request.manifest_digest.clone();
                    good.expected_running_manifest_digest = request.manifest_digest;
                }
            }
            assert_eq!(install_count, 2);
            assert!(!state.is_shutting_down());
        }
    }

    async fn test_state(dir: &tempfile::TempDir) -> Arc<DaemonState> {
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        *state.projects.write().await = ProjectRegistry::empty(dir.path().join("projects.json"));
        state
    }

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn fake_worker() -> Option<Self> {
            let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
            if std::process::Command::new(&python)
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            let keys = vec![
                (
                    "REFACT_DAEMON_WORKER_CMD",
                    std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                ),
                (
                    "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                    std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                ),
                ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
                (
                    "FAKE_WORKER_DELAY_READY",
                    std::env::var("FAKE_WORKER_DELAY_READY").ok(),
                ),
                (
                    "FAKE_WORKER_PORT_BUSY_EXIT",
                    std::env::var("FAKE_WORKER_PORT_BUSY_EXIT").ok(),
                ),
                (
                    "FAKE_WORKER_SKIP_LSP",
                    std::env::var("FAKE_WORKER_SKIP_LSP").ok(),
                ),
            ];
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join([python.as_str(), script.to_string_lossy().as_ref()]),
            );
            std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
            std::env::remove_var("FAKE_WORKER_CRASH");
            std::env::remove_var("FAKE_WORKER_DELAY_READY");
            std::env::remove_var("FAKE_WORKER_PORT_BUSY_EXIT");
            std::env::remove_var("FAKE_WORKER_SKIP_LSP");
            Some(Self { keys })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.keys.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn daemon_server_bind_enforces_loopback_open_hooks_policy() {
        let loopback = DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let listener = bind_listener(&loopback).unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());

        let wildcard = DaemonConfig {
            bind: "0.0.0.0".to_string(),
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let error = bind_listener(&wildcard).unwrap_err();
        assert!(error.contains("hooks without hooks.token or daemon auth"));
    }

    #[test]
    fn daemon_server_bind_falls_back_to_loopback_without_lan_auth() {
        let wildcard = DaemonConfig {
            bind: "0.0.0.0".to_string(),
            port: 0,
            ..DaemonConfig::default()
        };
        let listener = bind_listener(&wildcard).unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn installation_producer_requires_basic_and_persisted_daemon_preparation() {
        use crate::buddy::{resource_execution::*, action_execution::*};
        use refact_buddy_core::operator_types::*;
        use crate::buddy::improvement::installation::*;
        use base64::Engine;
        use std::os::unix::fs::PermissionsExt;
        // The native owner rejects a shared install parent, independently of
        // the image mode. Keep this scratch fixture private under any umask.
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let mut config = DaemonConfig::default();
        config.auth.enabled = true;
        config.auth.username = Some("fixture-user".into());
        config.auth.password = Some("fixture-password".into());
        let target = dir.path().join("test-owned-engine");
        let state = DaemonState::with_test_installation_target(
            config,
            dir.path().join("daemon"),
            target.clone(),
        );
        state
            .load_projects(dir.path().join("daemon/projects.json"))
            .await;
        let entry = state.projects.write().await.open(root).await.unwrap();
        let project = ProjectIdentity {
            project_id: entry.id.clone(),
            canonical_root: entry.root.to_string_lossy().into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "fixture".into(),
            engine_instance_id: "fixture".into(),
            epoch: 1,
        };
        let runtime =
            crate::buddy::outcome_runtime::OutcomeRuntime::open(&entry.root, project, worker)
                .unwrap();
        let (draft, mut spec) =
            crate::buddy::improvement::installation::tests::seed(&runtime, &target).await;
        spec.capabilities.can_execute = false;
        spec.capabilities.can_verify = false;
        let generic = PreparedAction {
            spec_hash: action_spec_hash(&spec, &spec.parameter_ref),
            spec: spec.clone(),
            bound_parameter_ref: spec.parameter_ref.clone(),
        };
        bind_prepared_draft(&runtime, &generic).await.unwrap();
        RuntimeActionRepository(runtime.clone())
            .prepare(generic.clone())
            .await
            .unwrap();
        runtime.shutdown().await;
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer worker-bearer".parse().unwrap());
        assert_eq!(
            prepare_candidate_installation(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(PrepareInstallationRequest { spec: spec.clone() })
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        headers.insert(
            "authorization",
            format!(
                "Basic {}",
                base64::prelude::BASE64_STANDARD.encode("fixture-user:fixture-password")
            )
            .parse()
            .unwrap(),
        );
        assert_eq!(
            grant_candidate_installation(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(GrantInstallationRequest {
                    prepared_hash: generic.spec_hash,
                    expires_in_seconds: 60
                })
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let (status, body) = response_json(
            prepare_candidate_installation(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(PrepareInstallationRequest { spec }),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["prepared"]["spec"]["capabilities"]["can_execute"],
            true
        );
        let hash = body["prepared"]["spec_hash"].as_str().unwrap().to_string();
        let (status, body) = response_json(
            grant_candidate_installation(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(GrantInstallationRequest {
                    prepared_hash: hash.clone(),
                    expires_in_seconds: 60,
                }),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["grant"]["requester"]["source_id"], "fixture-user");
        assert_eq!(body["grant"]["spec_hash"], hash);
        let request = NativeRequest {
            prepared_hash: hash.clone(),
            grant_id: body["grant_id"].as_str().unwrap().into(),
        };
        let (status, effect) = response_json(
            candidate_installation(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(request.clone()),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{effect}");
        assert_eq!(effect["native_observation"]["phase"], "installed");
        assert_eq!(
            refact_self_update::candidate_install::Installer::open(&target)
                .unwrap()
                .installed_digest()
                .unwrap(),
            draft.artifact_digest
        );
        let (status, retry) = response_json(
            candidate_installation(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(request),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["native_observation"], effect["native_observation"]);
        assert!(!state.is_shutting_down());
        assert_eq!(state.supervisor.worker_count().await, 0);
        assert!(serde_json::from_value::<GrantInstallationRequest>(
            json!({"prepared_hash": hash, "expires_in_seconds": 60, "requester": "forged"})
        )
        .is_err());
    }

    #[tokio::test]
    async fn candidate_installer_disabled_default_does_not_open_project_or_install() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = candidate_installation(
            State((state, 0)),
            axum::extract::Path("missing".into()),
            HeaderMap::new(),
            Json(crate::buddy::improvement::installation::NativeRequest {
                prepared_hash: "a".repeat(64),
                grant_id: "missing".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!dir.path().join(".refact").exists());
    }

    #[tokio::test]
    async fn auth_matrix_project_cookie_cannot_reach_fs_browse_or_doctor() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let cookie = format!(
            "{}={}",
            crate::daemon::auth::DAEMON_AUTH_COOKIE,
            crate::daemon::auth::project_cookie_value("project-a", "secret")
        );
        let router = make_router(state, 8488);
        let remote = || ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000)));

        let mut browse = Request::builder()
            .method("POST")
            .uri("/daemon/v1/fs/browse")
            .header("content-type", "application/json")
            .header("cookie", &cookie)
            .body(Body::from(r#"{}"#))
            .unwrap();
        browse.extensions_mut().insert(remote());
        let browse = router.clone().oneshot(browse).await.unwrap();
        assert_eq!(browse.status(), StatusCode::UNAUTHORIZED);

        let mut doctor = Request::builder()
            .uri("/daemon/v1/doctor")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        doctor.extensions_mut().insert(remote());
        let doctor = router.oneshot(doctor).await.unwrap();
        assert_eq!(doctor.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_matrix_project_cookie_scopes_proxied_v1_routes() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let cookie = format!(
            "{}={}",
            crate::daemon::auth::DAEMON_AUTH_COOKIE,
            crate::daemon::auth::project_cookie_value("project-a", "secret")
        );
        let router = make_router(state, 8488);
        let remote = || ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000)));

        let mut own_project = Request::builder()
            .uri("/p/project-a/v1/files/read?path=%2Ftmp%2Fx")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        own_project.extensions_mut().insert(remote());
        let own_project = router.clone().oneshot(own_project).await.unwrap();
        assert_eq!(own_project.status(), StatusCode::NOT_FOUND);
        let body = hyper::body::to_bytes(own_project.into_body())
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "project not found");

        for (method, uri) in [
            ("GET", "/p/project-b/v1/files/read?path=%2Ftmp%2Fx"),
            ("GET", "/p/project-b/v1/git/status"),
            ("POST", "/p/project-b/v1/exec/spawn"),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(r#"{}"#))
                .unwrap();
            request.extensions_mut().insert(remote());
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri} must not accept another project's cookie"
            );
        }
    }

    #[tokio::test]
    async fn sse_daemon_events_replay_streams_and_terminates() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        state
            .events
            .emit("sse_smoke_event", None, json!({"n": 1}))
            .await
            .unwrap();
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri("/daemon/v1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        let body = tokio::time::timeout(
            Duration::from_secs(5),
            hyper::body::to_bytes(response.into_body()),
        )
        .await
        .expect("events stream without follow must terminate")
        .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("event:daemon"), "body: {text:?}");
        assert!(text.contains("sse_smoke_event"), "body: {text:?}");
        assert!(text.contains("id:1"), "body: {text:?}");
    }

    #[tokio::test]
    async fn daemon_router_restart_returns_success_and_requests_shutdown() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let mut shutdown_rx = state.shutdown_receiver();
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/restart")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, json) = response_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["success"], true);
        assert_eq!(json["restarting"], true);
        assert_eq!(shutdown_rx.recv().await.unwrap(), "restart_requested");
    }

    #[tokio::test]
    async fn daemon_router_update_install_conflicts_when_active() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        {
            let mut update_state = state.update_state.lock().await;
            update_state.phase = UpdatePhase::Downloading;
        }
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/update/install")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, json) = response_json(response).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], "update already in progress");
    }

    #[tokio::test]
    async fn update_settings_rejects_lan_without_credentials() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"lan_enabled":true,"mdns_enabled":true,"auth_enabled":false}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_rejects_missing_token() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let mut request = Request::builder()
            .method("POST")
            .uri("/daemon/v1/shutdown")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"reason":"t"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_accepts_correct_token() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let mut request = Request::builder()
            .method("POST")
            .uri("/daemon/v1/shutdown")
            .header("content-type", "application/json")
            .header("Authorization", "Bearer secret")
            .body(Body::from(r#"{"reason":"t"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_accepts_basic_auth_from_remote() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use base64::Engine;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                username: Some("alice".to_string()),
                password: Some("hunter2".to_string()),
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let encoded = base64::prelude::BASE64_STANDARD.encode("alice:hunter2");
        let mut request = Request::builder()
            .uri("/daemon/v1/projects")
            .header("Authorization", format!("Basic {encoded}"))
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_control_plane_body_is_not_proxy_limited() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let body = serde_json::json!({
            "project_id": "missing",
            "pid": 1,
            "lsp_clients": 0,
            "busy_chats": 0,
            "exec_running": 0,
            "last_activity_ts": 0,
            "padding": "x".repeat(crate::daemon::proxy::PROXY_BODY_LIMIT + 1024),
        })
        .to_string();

        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/worker-status")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn tail_file_redacts_daemon_token_query_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");
        tokio::fs::write(
            &path,
            "GET /p/project/v1?daemon_token=secret-token&chat=1 failed\n",
        )
        .await
        .unwrap();

        let text = tail_file(&path, 10).await.unwrap();

        assert!(!text.contains("secret-token"));
        assert!(text.contains("daemon_token=<redacted>&chat=1"));
    }

    #[tokio::test]
    async fn worker_status_handler_rejects_missing_project_and_wrong_pid() {
        use crate::daemon_link::WorkerStatusReport;

        fn report(project_id: &str, pid: u32) -> WorkerStatusReport {
            WorkerStatusReport {
                project_id: project_id.to_string(),
                pid,
                instance_token: "token".to_string(),
                lsp_clients: 1,
                busy_chats: 0,
                exec_running: 0,
                last_activity_ts: 55,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let root = tempfile::tempdir().unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.path().to_path_buf()).await.unwrap()
        };
        state
            .supervisor
            .set_test_worker_info(
                &entry.id,
                123,
                crate::daemon::supervisor::WorkerState::Ready,
                "token",
            )
            .await;

        let missing =
            worker_status(State((state.clone(), 8488)), Json(report("missing", 123))).await;
        let (status, missing) = response_json(missing).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(missing["success"], false);
        assert!(missing["error"]
            .as_str()
            .unwrap()
            .contains("project not found"));

        let wrong_pid =
            worker_status(State((state.clone(), 8488)), Json(report(&entry.id, 999))).await;
        let (status, wrong_pid) = response_json(wrong_pid).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(wrong_pid["success"], false);
        assert!(wrong_pid["error"]
            .as_str()
            .unwrap()
            .contains("current worker"));
        assert!(state.latest_worker_status(&entry.id).await.is_none());
    }

    #[tokio::test]
    async fn cors_preflight_allows_ide_webview_origins_on_proxy_path() {
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/p/project/v1/ping")
                    .header(header::ORIGIN, "http://refactai")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://refactai")
        );
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
    }

    #[tokio::test]
    async fn cors_preflight_allows_vscode_webview_origin_on_subscribe_path() {
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let origin = "vscode-webview://11112222-3333-4444-5555-666677778888";
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/p/project/v1/sidebar/subscribe")
                    .header(header::ORIGIN, origin)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some(origin)
        );
    }

    #[tokio::test]
    async fn cors_preflight_rejects_untrusted_origin() {
        use axum::http::header;
        use hyper::{Body, Request};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/p/project/v1/ping")
                    .header(header::ORIGIN, "http://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(daemon)]
    async fn daemon_proxy_path_trusts_ide_webview_origin_and_rejects_others() {
        use axum::extract::ConnectInfo;
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let project_root = dir.path().join("proxy-project");
        std::fs::create_dir_all(&project_root).unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        state.sync_project_liveness(&entry).await;
        let proxy_path = format!("/p/{}/v1/ping", entry.id);

        let mut trusted = Request::builder()
            .uri(&proxy_path)
            .header(header::ORIGIN, "http://refactai")
            .body(Body::empty())
            .unwrap();
        trusted
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state.clone(), 8488)
            .oneshot(trusted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://refactai")
        );

        let mut untrusted = Request::builder()
            .uri(&proxy_path)
            .header(header::ORIGIN, "http://evil.example")
            .body(Body::empty())
            .unwrap();
        untrusted
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state.clone(), 8488)
            .oneshot(untrusted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        state.supervisor.stop_all().await;
    }

    #[tokio::test]
    async fn daemon_auth_trusts_ide_webview_origin_but_requires_auth_from_remote() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );

        let mut trusted = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::ORIGIN, "http://refactai")
            .body(Body::empty())
            .unwrap();
        trusted
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state.clone(), 8488)
            .oneshot(trusted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://refactai")
        );

        for origin in ["http://evil.example", "null"] {
            let mut request = Request::builder()
                .uri("/daemon/v1/projects")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
            let response = make_router(state.clone(), 8488)
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "loopback origin {origin} must not be trusted"
            );
        }

        let mut remote = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::ORIGIN, "http://refactai")
            .body(Body::empty())
            .unwrap();
        remote
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(remote).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    async fn response_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(daemon)]
    async fn buddy_projection_fake_workers_restart_dedup_and_no_authority_replay() {
        use crate::daemon::buddy_reconciliation::{WorkerProjection, SupervisionRef};
        let _env = EnvGuard::fake_worker().expect("python3 fake worker is required");
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            Some("fixture-secret".into()),
            dir.path().join("daemon"),
            0,
        );
        let path = dir.path().join("daemon/projects.json");
        state.load_projects(path.clone()).await;
        let mut entries = Vec::new();
        for name in ["a", "b"] {
            let root = dir.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            entries.push(state.projects.write().await.open(root).await.unwrap());
        }
        let mut reports = Vec::new();
        for entry in &entries {
            let worker = state.supervisor.ensure_ready_worker(entry).await.unwrap();
            reports.push(WorkerProjection {
                project_id: entry.id.clone(),
                canonical_root: entry.root.clone(),
                pid: worker.pid.unwrap(),
                instance_token: state
                    .supervisor
                    .test_worker_instance_token(&entry.id)
                    .await
                    .unwrap(),
                cursor: 1,
                supervision: vec![SupervisionRef {
                    mandate_id: "ref-only".into(),
                    owner_epoch: 1,
                    chat_id: "chat".into(),
                    worker_agent_id: None,
                }],
            });
        }
        let mut headers = HeaderMap::new();
        let denied = buddy_worker_projection(
            State((state.clone(), 0)),
            headers.clone(),
            Json(reports[0].clone()),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        for report in &reports {
            let response = buddy_worker_projection(
                State((state.clone(), 0)),
                headers.clone(),
                Json(report.clone()),
            )
            .await;
            let (status, body) = response_json(response).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["wake_authorized"], false);
            assert_eq!(body["changed"], true);
            assert!(!state.accept_buddy_projection(report.clone()).await.unwrap());
        }
        let before_b = state
            .buddy_projection(&entries[1].id)
            .await
            .unwrap()
            .unwrap();
        let before_a = state
            .buddy_projection(&entries[0].id)
            .await
            .unwrap()
            .unwrap();
        let next = state.supervisor.restart_worker(&entries[0]).await.unwrap();
        assert!(state
            .accept_buddy_projection(reports[0].clone())
            .await
            .is_err());
        let mut stale_token = reports[0].clone();
        stale_token.pid = next.pid.unwrap();
        assert!(state
            .accept_buddy_projection(stale_token.clone())
            .await
            .is_err());
        stale_token.instance_token = state
            .supervisor
            .test_worker_instance_token(&entries[0].id)
            .await
            .unwrap();
        assert!(state.accept_buddy_projection(stale_token).await.unwrap());
        let after_a = state
            .buddy_projection(&entries[0].id)
            .await
            .unwrap()
            .unwrap();
        assert!(after_a.worker.generation > before_a.worker.generation);
        assert_ne!(
            after_a.worker.instance_digest,
            before_a.worker.instance_digest
        );
        assert_eq!(
            state
                .buddy_projection(&entries[1].id)
                .await
                .unwrap()
                .unwrap(),
            before_b
        );
        state.supervisor.stop_all().await;
        let restarted = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events2.jsonl")),
            Some("fixture-secret".into()),
            dir.path().join("daemon"),
            0,
        );
        restarted.load_projects(path).await;
        restarted
            .reconcile_buddy_batch(crate::daemon::state::now_ms())
            .await;
        assert_eq!(restarted.supervisor.worker_count().await, 0);
        assert!(restarted
            .accept_buddy_projection(reports[1].clone())
            .await
            .is_err());
        for entry in entries {
            let row = restarted
                .buddy_projection(&entry.id)
                .await
                .unwrap()
                .unwrap();
            assert!(row.offline_gap.unwrap().recovered_at_ms.is_none());
        }
        let bytes = tokio::fs::read_to_string(dir.path().join("daemon/buddy-observations.json"))
            .await
            .unwrap();
        for report in reports {
            assert!(!bytes.contains(&report.instance_token));
        }
        assert!(!bytes.contains("allowed_operations"));
        assert!(!bytes.contains("tool_consent"));
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(daemon)]
    async fn buddy_wake_user_auth_fake_worker_bound_and_revoke() {
        use base64::Engine;
        let _env = EnvGuard::fake_worker().expect("python3 fake worker required");
        let dir = tempfile::tempdir().unwrap();
        let mut config = DaemonConfig::default();
        config.auth.enabled = true;
        config.auth.username = Some("fixture-user".into());
        config.auth.password = Some("fixture-password".into());
        let state = DaemonState::new_with_daemon_dir(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("worker-bearer".into()),
            dir.path().join("daemon"),
            0,
        );
        state
            .load_projects(dir.path().join("daemon/projects.json"))
            .await;
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let entry = state.projects.write().await.open(root).await.unwrap();
        let request = crate::daemon::buddy_wake::WakeGrantRequest {
            canonical_root: entry.root.clone(),
            trigger_kind: crate::daemon::buddy_wake::WakeTriggerKind::ObservationTick,
            trigger_id: "explicit-timer".into(),
            expires_at_ms: crate::daemon::state::now_ms() + 600_000,
            max_starts: 1,
            interval_ms: 60_000,
            wake_only: true,
        };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer worker-bearer".parse().unwrap());
        assert_eq!(
            buddy_wake_issue(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(request.clone())
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        headers.insert(
            "authorization",
            format!(
                "Basic {}",
                base64::prelude::BASE64_STANDARD.encode("fixture-user:fixture-password")
            )
            .parse()
            .unwrap(),
        );
        assert_eq!(
            buddy_wake_issue(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers.clone(),
                Json(request)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let grant = state.buddy_wake_grant(&entry.id).await.unwrap().unwrap();
        assert_eq!(grant.authored_by, "fixture-user");
        let due = grant.issued_at_ms + 60_000;
        state.tick_buddy_wakes(due).await;
        let worker = state.supervisor.worker_info(&entry.id).await.unwrap();
        assert_eq!(worker.state, crate::daemon::supervisor::WorkerState::Ready);
        state.tick_buddy_wakes(due + 1).await;
        assert_eq!(
            state.supervisor.worker_info(&entry.id).await.unwrap().pid,
            worker.pid
        );
        assert_eq!(
            state
                .buddy_wake_grant(&entry.id)
                .await
                .unwrap()
                .unwrap()
                .remaining_starts,
            0
        );
        assert!(state.buddy_projection(&entry.id).await.unwrap().is_none());
        assert!(!entry.root.join(".refact/buddy").exists());
        assert_eq!(
            buddy_wake_revoke(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                HeaderMap::new()
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            buddy_wake_revoke(
                State((state.clone(), 0)),
                axum::extract::Path(entry.id.clone()),
                headers
            )
            .await
            .status(),
            StatusCode::OK
        );
        state.supervisor.stop_all().await;
        let bytes = tokio::fs::read_to_string(dir.path().join("daemon/buddy-wake-grants.json"))
            .await
            .unwrap();
        assert!(!bytes.contains("worker-bearer"));
        assert!(!bytes.contains("fixture-password"));
        assert!(!bytes.contains("tool_consent"));
        assert!(!bytes.contains("mandate_id"));
    }
}
