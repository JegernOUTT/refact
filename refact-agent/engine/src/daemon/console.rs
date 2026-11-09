use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::{ConnectInfo, Path, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{RwLock, Semaphore};

use super::{
    state::{now_ms, DaemonState},
    supervisor::WorkerState,
};
use crate::http::routers::v1::engine_observation::{EngineIdentity, LiveChat, LiveSnapshot};

pub(crate) struct Console {
    attachments: RwLock<HashMap<String, Attachment>>,
    client: reqwest::Client,
    permits: Semaphore,
}

#[derive(Clone)]
struct Attachment {
    id: String,
    host: String,
    port: u16,
    label: String,
    instance_id: String,
    cancel: tokio_util::sync::CancellationToken,
}

impl Console {
    pub fn new() -> Self {
        Self {
            attachments: RwLock::new(HashMap::new()),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(1))
                .timeout(Duration::from_secs(3))
                .build()
                .expect("observation client"),
            permits: Semaphore::new(8),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct EngineRow {
    id: String,
    kind: String,
    label: String,
    project_id: Option<String>,
    root: Option<String>,
    state: String,
    instance_id: Option<String>,
    http_port: Option<u16>,
    observe_only: bool,
    log_stream: bool,
    logs_unavailable_reason: Option<String>,
    observed_at_ms: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct ChatObservation {
    engine_id: String,
    status: String,
    coverage: &'static str,
    observed_at_ms: Option<u64>,
    error: Option<String>,
    chats: Vec<LiveChat>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachRequest {
    host: String,
    port: u16,
    label: Option<String>,
}

fn endpoint(host: &str, port: u16) -> Result<String, &'static str> {
    if port == 0 {
        return Err("port must be between 1 and 65535");
    }
    match host {
        "127.0.0.1" => Ok(format!("http://127.0.0.1:{port}")),
        "::1" => Ok(format!("http://[::1]:{port}")),
        _ => Err("only exact numeric loopback hosts 127.0.0.1 and ::1 are supported"),
    }
}

fn trusted_headers(headers: &HeaderMap, method: &Method, configured_auth: bool) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let Ok(authority) = host.parse::<hyper::http::uri::Authority>() else {
        return false;
    };
    if !configured_auth
        && !(authority.host() == "localhost"
            || authority
                .host()
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
    {
        return false;
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        let Ok(url) = url::Url::parse(origin) else {
            return false;
        };
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return false;
        }
        let expected = format!("{}://{host}", url.scheme());
        if url.origin().ascii_serialization() != expected {
            return false;
        }
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return false;
    }
    if method == Method::POST
        && !headers
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|v| v.split(';').next() == Some("application/json"))
    {
        return false;
    }
    true
}

pub async fn guard<B: Send + 'static>(
    State(state): State<Arc<DaemonState>>,
    req: Request<B>,
    next: Next<B>,
) -> Response {
    let configured_auth =
        state.config.auth.enabled && super::auth::daemon_lan_auth_ok(&state.config);
    if configured_auth
        && !super::auth::remote_authorized(
            &req,
            &super::auth::DaemonAuthPolicy {
                token: state.auth_token.clone(),
                username: state.config.auth.username.clone(),
                password: state.config.auth.password.clone(),
                ..Default::default()
            },
        )
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"daemon authentication required"})),
        )
            .into_response();
    }
    let local_peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(addr)| addr.ip().is_loopback());
    if (!configured_auth && !local_peer)
        || !trusted_headers(req.headers(), req.method(), configured_auth)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"same-origin local console request required"})),
        )
            .into_response();
    }
    next.run(req).await
}

async fn fetch<T: serde::de::DeserializeOwned>(
    state: &DaemonState,
    base: &str,
    path: &str,
    token: Option<&str>,
) -> Result<T, String> {
    let _permit = tokio::time::timeout(Duration::from_secs(4), state.console.permits.acquire())
        .await
        .map_err(|_| "observation capacity timed out")?
        .map_err(|_| "observation capacity closed")?;
    let mut shutdown = state.shutdown_receiver();
    if state.is_shutting_down() {
        return Err("daemon shutting down".into());
    }
    let work = async {
        let mut request = state.console.client.get(format!("{base}{path}"));
        if let Some(token) = token {
            request = request.header("x-refact-observation-token", token);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "engine observation connection failed")?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED => {
                return Err("unsupported engine observation API".into())
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                return Err("authenticated external engines are not supported".into())
            }
            status if !status.is_success() => return Err("engine observation refused".into()),
            _ => {}
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "engine observation body failed")?
        {
            if body.len() + chunk.len() > 1024 * 1024 {
                return Err("engine observation exceeds body limit".into());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| "invalid engine observation response".into())
    };
    tokio::select! { result = tokio::time::timeout(Duration::from_secs(4), work) => result.map_err(|_| "engine observation timed out".to_string())?, _ = shutdown.recv() => Err("daemon shutting down".into()) }
}

async fn observe(
    state: &DaemonState,
    base: &str,
    token: Option<&str>,
    expected: Option<&str>,
    managed: Option<&super::buddy_reconciliation::ObservedIdentity>,
    include_chats: bool,
) -> Result<(EngineIdentity, LiveSnapshot), String> {
    let capability = token.zip(managed).map(|(secret, instance)| {
        crate::http::routers::v1::engine_observation::observation_capability(
            secret,
            &instance.instance_digest,
        )
    });
    let token = capability.as_deref();
    let identity: EngineIdentity = fetch(state, base, "/v1/engine/identity", token).await?;
    if identity.schema_version != 1 || uuid::Uuid::parse_str(&identity.instance_id).is_err() {
        return Err("unsupported engine identity".into());
    }
    if expected.is_some_and(|expected| expected != identity.instance_id) {
        return Err("engine identity changed; detach and reattach".into());
    }
    if managed.is_some_and(|m| {
        m.pid != identity.pid
            || Some(&m.instance_digest) != identity.managed_instance_digest.as_ref()
    }) {
        return Err("managed engine identity mismatch".into());
    }
    if !include_chats {
        let snapshot = LiveSnapshot {
            schema_version: 1,
            instance_id: identity.instance_id.clone(),
            coverage: "loaded_sessions".into(),
            observed_at_ms: now_ms(),
            chats: vec![],
        };
        return Ok((identity, snapshot));
    }
    let snapshot: LiveSnapshot = fetch(state, base, "/v1/chats/live", token).await?;
    if snapshot.schema_version != 1 || snapshot.coverage != "loaded_sessions" {
        return Err("unsupported engine live session coverage".into());
    }
    let now = now_ms();
    if now.saturating_sub(snapshot.observed_at_ms) > 30_000
        || snapshot.observed_at_ms > now.saturating_add(5_000)
    {
        return Err("engine observation is stale or future-dated".into());
    }
    if snapshot.instance_id != identity.instance_id {
        return Err("engine identity changed during observation".into());
    }
    if snapshot.chats.len() > 500
        || snapshot.chats.iter().any(|c| {
            c.id.len() > 1024
                || c.title.len() > 2048
                || c.mode.len() > 256
                || c.session_state.len() > 64
                || c.project_root.as_ref().is_some_and(|p| p.len() > 16384)
                || !matches!(c.kind.as_str(), "chat" | "subagent" | "buddy" | "internal")
        })
    {
        return Err("engine observation exceeds session limits".into());
    }
    let mut ids = std::collections::HashSet::new();
    if snapshot.chats.iter().any(|c| !ids.insert(&c.id)) {
        return Err("duplicate engine session identity".into());
    }
    Ok((identity, snapshot))
}

fn attached_row(a: &Attachment) -> EngineRow {
    EngineRow {
        id: a.id.clone(),
        kind: "attached".into(),
        label: a.label.clone(),
        project_id: None,
        root: None,
        state: "attached".into(),
        instance_id: Some(a.instance_id.clone()),
        http_port: Some(a.port),
        observe_only: true,
        log_stream: false,
        logs_unavailable_reason: Some(
            "External engines do not provide a supported live log stream".into(),
        ),
        observed_at_ms: None,
        error: None,
    }
}

pub async fn attach(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<AttachRequest>,
) -> Result<Json<EngineRow>, (StatusCode, Json<serde_json::Value>)> {
    let run = async {
        let base = endpoint(&request.host, request.port).map_err(str::to_string)?;
        let label = request
            .label
            .unwrap_or_else(|| format!("{}:{}", request.host, request.port));
        if label.len() > 256 || label.chars().any(char::is_control) {
            return Err("label must contain at most 256 bytes without control characters".into());
        }
        let (identity, _) = tokio::time::timeout(
            Duration::from_secs(10),
            observe(&state, &base, None, None, None, true),
        )
        .await
        .map_err(|_| "attachment observation timed out".to_string())??;
        let mut attachments = state.console.attachments.write().await;
        if let Some(existing) = attachments
            .values()
            .find(|a| a.host == request.host && a.port == request.port)
        {
            if existing.instance_id != identity.instance_id {
                return Err("engine identity changed; detach first".into());
            }
            return Ok(attached_row(existing));
        }
        if attachments.len() >= 32 {
            return Err("attachment limit reached".into());
        }
        let a = Attachment {
            id: format!("attached:{}", uuid::Uuid::new_v4()),
            host: request.host,
            port: request.port,
            label,
            instance_id: identity.instance_id,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let mut row = attached_row(&a);
        row.observed_at_ms = Some(now_ms());
        attachments.insert(a.id.clone(), a);
        Ok(row)
    };
    run.await
        .map(Json)
        .map_err(|error: String| (StatusCode::BAD_REQUEST, Json(json!({"error":error}))))
}

pub async fn detach(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !id.starts_with("attached:") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let removed = state.console.attachments.write().await.remove(&id);
    if let Some(a) = &removed {
        a.cancel.cancel();
    }
    Ok(Json(json!({"detached":removed.is_some()})))
}

async fn collect(
    state: &Arc<DaemonState>,
    include_chats: bool,
) -> Vec<(EngineRow, ChatObservation)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut targets = Vec::new();
    let projects = state.projects.read().await.list();
    for entry in projects {
        let info = state.supervisor.worker_info(&entry.id).await;
        let running = info.as_ref().is_some_and(|i| i.state == WorkerState::Ready);
        let row = EngineRow {
            id: format!("managed:{}", entry.id),
            kind: "managed".into(),
            label: entry.slug.clone(),
            project_id: Some(entry.id.clone()),
            root: Some(entry.root.to_string_lossy().into()),
            state: info
                .as_ref()
                .map(|i| {
                    match i.state {
                        WorkerState::Stopped => "stopped",
                        WorkerState::Starting => "starting",
                        WorkerState::Ready => "ready",
                        WorkerState::Stopping => "stopping",
                        WorkerState::Crashed => "crashed",
                        WorkerState::Failed {
                            ..
                        } => "failed",
                    }
                    .to_string()
                })
                .unwrap_or("stopped".into()),
            instance_id: None,
            http_port: info.as_ref().map(|i| i.http_port),
            observe_only: false,
            log_stream: true,
            logs_unavailable_reason: None,
            observed_at_ms: None,
            error: None,
        };
        let base =
            running.then(|| format!("http://127.0.0.1:{}", info.as_ref().unwrap().http_port));
        targets.push((row, base, None, None));
    }
    for a in state.console.attachments.read().await.values() {
        targets.push((
            attached_row(a),
            endpoint(&a.host, a.port).ok(),
            Some(a.instance_id.clone()),
            Some(a.cancel.clone()),
        ));
    }
    let mut rows: Vec<_> = stream::iter(targets)
        .map(|(mut row, base, expected, cancel)| async move {
            let mut chats = ChatObservation {
                engine_id: row.id.clone(),
                status: "stopped".into(),
                coverage: "loaded_sessions",
                observed_at_ms: None,
                error: if row.project_id.is_some() && row.state != "stopped" { Some(format!("managed engine {}",row.state)) } else { None },
                chats: Vec::new(),
            };
            if base.is_none() && chats.error.is_some() { chats.status = "unavailable".into(); }
            if let Some(base) = base {
                let before = if let Some(id) = &row.project_id {
                    state.supervisor.observed_worker_identity(id).await
                } else {
                    None
                };
                let token = if row.project_id.is_some() {
                    state.auth_token.as_deref().or_else(|| {
                        state
                            .config
                            .hooks
                            .enabled
                            .then_some(state.config.hooks.token.as_deref())
                            .flatten()
                    })
                } else {
                    None
                };
                let result = if row.project_id.is_some() && before.is_none() {
                    Err("managed engine identity unavailable".into())
                } else {
                    let observation = async {
                        tokio::time::timeout_at(deadline, observe(state, &base, token, expected.as_deref(), before.as_ref(), include_chats))
                            .await.map_err(|_| "console observation deadline exceeded".to_string())?
                    };
                    if let Some(cancel) = &cancel {
                        tokio::select! { result = observation => result, _ = cancel.cancelled() => Err("attachment detached".into()) }
                    } else { observation.await }
                };
                let after = if let Some(id) = &row.project_id {
                    state.supervisor.observed_worker_identity(id).await
                } else {
                    None
                };
                let result = if before != after {
                    Err("managed engine changed during observation".into())
                } else {
                    result
                };
                match result {
                    Ok((identity, snapshot)) => {
                        row.instance_id = Some(identity.instance_id);
                        row.observed_at_ms = Some(snapshot.observed_at_ms);
                        chats.observed_at_ms = Some(snapshot.observed_at_ms);
                        chats.status = "available".into();
                        chats.error = None;
                        chats.chats = snapshot.chats;
                    }
                    Err(error) => {
                        chats.status = if error.starts_with("unsupported") {
                            "unsupported"
                        } else {
                            "unavailable"
                        }
                        .into();
                        row.error = Some(error.clone());
                        chats.error = Some(error);
                    }
                }
            }
            (row, chats)
        })
        .buffer_unordered(8)
        .collect()
        .await;
    rows.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    rows
}

pub async fn engines(State(state): State<Arc<DaemonState>>) -> Json<serde_json::Value> {
    let rows = collect(&state, false)
        .await
        .into_iter()
        .map(|r| r.0)
        .collect::<Vec<_>>();
    Json(json!({"schema_version":1,"observed_at_ms":now_ms(),"engines":rows}))
}
pub async fn chats(State(state): State<Arc<DaemonState>>) -> Json<serde_json::Value> {
    let rows = collect(&state, true)
        .await
        .into_iter()
        .map(|r| r.1)
        .collect::<Vec<_>>();
    Json(json!({"schema_version":1,"observed_at_ms":now_ms(),"engines":rows}))
}

pub fn routes(state: Arc<DaemonState>) -> axum::Router<(Arc<DaemonState>, u16)> {
    axum::Router::new()
        .route("/daemon/v1/engines", axum::routing::get(engines))
        .route("/daemon/v1/engines/attach", axum::routing::post(attach))
        .route("/daemon/v1/engines/:id", axum::routing::delete(detach))
        .route("/daemon/v1/chats/live", axum::routing::get(chats))
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_state(dir: &std::path::Path) -> Arc<DaemonState> {
        DaemonState::new_with_daemon_dir(
            super::super::config::DaemonConfig::default(),
            super::super::events::EventBus::new(dir.join("events.jsonl")),
            None,
            dir.to_path_buf(),
            0,
        )
    }

    async fn fixture_server(app: axum::Router) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum::Server::from_tcp(listener)
            .unwrap()
            .serve(app.into_make_service());
        let task = tokio::spawn(async move {
            server.await.unwrap();
        });
        (port, task)
    }

    fn fixture_app(identity: Arc<RwLock<String>>) -> axum::Router {
        let id = identity.clone();
        axum::Router::new().route("/v1/engine/identity", axum::routing::get(move |headers: HeaderMap| { let id = id.clone(); async move {
            assert!(!headers.contains_key(header::AUTHORIZATION));
            Json(json!({"schema_version":1,"instance_id":id.read().await.clone(),"pid":999,"managed_instance_digest":null}))
        }})).route("/v1/chats/live",axum::routing::get(move || { let id = identity.clone(); async move {
            Json(json!({"schema_version":1,"instance_id":id.read().await.clone(),"coverage":"loaded_sessions","observed_at_ms":now_ms(),"chats":[{"id":"same-chat","title":"<script>secret</script>","mode":"agent","session_state":"paused","paused":true,"project_root":null,"updated_at_ms":100,"kind":"chat"}]}))
        }}))
    }

    #[tokio::test]
    async fn reused_managed_port_never_receives_an_installation_credential() {
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let state = fixture_state(dir.path());
        let (sent, received) = tokio::sync::oneshot::channel();
        let sender = Arc::new(std::sync::Mutex::new(Some(sent)));
        let app = axum::Router::new().route("/v1/engine/identity", axum::routing::get(move |headers: HeaderMap| {
            let sender = sender.clone();
            async move {
                sender.lock().unwrap().take().unwrap().send(headers).unwrap();
                Json(json!({"schema_version":1,"instance_id":uuid::Uuid::new_v4().to_string(),"pid":999,"managed_instance_digest":null}))
            }
        }));
        let (port, task) = fixture_server(app).await;
        let identity = super::super::buddy_reconciliation::ObservedIdentity {
            pid: 123,
            generation: 1,
            instance_digest: "a".repeat(64),
        };
        let secret = "daemon-installation-secret";
        let result = observe(
            &state,
            &endpoint("127.0.0.1", port).unwrap(),
            Some(secret),
            None,
            Some(&identity),
            true,
        )
        .await;
        assert_eq!(result.err().unwrap(), "managed engine identity mismatch");
        let headers = received.await.unwrap();
        assert!(!headers.contains_key(header::AUTHORIZATION));
        assert!(!headers
            .iter()
            .any(|(_, v)| v.to_str().unwrap_or_default().contains(secret)));
        let capability = headers["x-refact-observation-token"].to_str().unwrap();
        assert_eq!(
            capability,
            crate::http::routers::v1::engine_observation::observation_capability(
                secret,
                &identity.instance_digest
            )
        );
        let mut config = super::super::config::DaemonConfig::default();
        config.auth.enabled = true;
        config.auth.token = Some(secret.into());
        let protected = DaemonState::new_with_daemon_dir(
            config,
            super::super::events::EventBus::new(dir.path().join("protected-events")),
            Some(secret.into()),
            dir.path().to_path_buf(),
            0,
        );
        let router = super::super::server::make_router(protected, 8488);
        for supplied in [None, Some(capability)] {
            let mut request = Request::builder()
                .uri("/daemon/v1/engines")
                .header(header::HOST, "svakhreev-work.local:8488")
                .header("x-refact-observation-token", capability);
            if let Some(token) = supplied {
                request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
            }
            let mut request = request.body(axum::body::Body::empty()).unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo("192.0.2.5:1234".parse::<SocketAddr>().unwrap()));
            assert_eq!(
                router.clone().oneshot(request).await.unwrap().status(),
                StatusCode::UNAUTHORIZED
            );
        }
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn attached_observation_is_isolated_fenced_and_detach_never_manages_process() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture_state(dir.path());
        let identity = Arc::new(RwLock::new(uuid::Uuid::new_v4().to_string()));
        let (port, task) = fixture_server(fixture_app(identity.clone())).await;
        let a = attach(
            State(state.clone()),
            Json(AttachRequest {
                host: "127.0.0.1".into(),
                port,
                label: Some("local".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(!a.log_stream);
        assert!(a.observe_only);
        let rows = collect(&state, true).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.status, "available");
        assert_eq!(rows[0].1.chats[0].id, "same-chat");
        assert_eq!(rows[0].1.chats[0].title, "<script>secret</script>");
        assert!(state.supervisor.worker_infos().await.is_empty());
        assert!(state.proxy_activity.read().is_empty());
        *identity.write().await = uuid::Uuid::new_v4().to_string();
        let changed = collect(&state, true).await;
        assert_eq!(changed[0].1.status, "unavailable");
        assert!(changed[0].1.chats.is_empty());
        assert_eq!(
            detach(State(state.clone()), Path(a.id)).await.unwrap().0["detached"],
            true
        );
        assert!(collect(&state, true).await.is_empty());
        let response = state
            .console
            .client
            .get(format!("http://127.0.0.1:{port}/v1/engine/identity"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        state.request_shutdown("fixture".into());
        assert!(state
            .console
            .client
            .get(format!("http://127.0.0.1:{port}/v1/engine/identity"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn unsupported_auth_redirect_and_body_limits_fail_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture_state(dir.path());
        for (status, expected) in [
            (StatusCode::NOT_FOUND, "unsupported"),
            (StatusCode::UNAUTHORIZED, "authenticated external"),
            (StatusCode::FOUND, "refused"),
        ] {
            let app = axum::Router::new().route(
                "/v1/engine/identity",
                axum::routing::get(move || async move {
                    (status, [(header::LOCATION, "http://192.0.2.1/")], "")
                }),
            );
            let (port, task) = fixture_server(app).await;
            let error = observe(
                &state,
                &endpoint("127.0.0.1", port).unwrap(),
                None,
                None,
                None,
                true,
            )
            .await
            .err()
            .unwrap();
            assert!(error.contains(expected), "{error}");
            task.abort();
            let _ = task.await;
        }
        let app = axum::Router::new().route(
            "/oversized",
            axum::routing::get(|| async { "x".repeat(1024 * 1024 + 1) }),
        );
        let (port, task) = fixture_server(app).await;
        let result = fetch::<serde_json::Value>(
            &state,
            &endpoint("127.0.0.1", port).unwrap(),
            "/oversized",
            None,
        )
        .await;
        assert_eq!(result.unwrap_err(), "engine observation exceeds body limit");
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn stopped_project_overview_does_not_wake_or_touch_activity() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture_state(dir.path());
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let project = state.projects.write().await.open(root).await.unwrap();
        let rows = collect(&state, true).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.id, format!("managed:{}", project.id));
        assert_eq!(rows[0].1.status, "stopped");
        assert!(state.supervisor.worker_infos().await.is_empty());
        assert!(state.proxy_activity.read().is_empty());
        assert_eq!(
            state
                .projects
                .read()
                .await
                .get(&project.id)
                .unwrap()
                .last_active_ms,
            project.last_active_ms
        );
    }

    #[tokio::test]
    async fn named_host_console_requires_existing_daemon_auth_and_same_origin() {
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let mut config = super::super::config::DaemonConfig::default();
        config.auth.enabled = true;
        config.auth.token = Some("daemon-secret".into());
        let state = DaemonState::new_with_daemon_dir(
            config,
            super::super::events::EventBus::new(dir.path().join("events")),
            Some("daemon-secret".into()),
            dir.path().to_path_buf(),
            0,
        );
        let app = super::super::server::make_router(state, 8488);
        for (token, origin, expected) in [
            (
                false,
                "http://svakhreev-work.local:8488",
                StatusCode::UNAUTHORIZED,
            ),
            (true, "http://svakhreev-work.local:8488", StatusCode::OK),
            (true, "http://evil.example", StatusCode::FORBIDDEN),
        ] {
            let mut req = Request::builder()
                .uri("/daemon/v1/engines")
                .header(header::HOST, "svakhreev-work.local:8488")
                .header(header::ORIGIN, origin);
            if token {
                req = req.header(header::AUTHORIZATION, "Bearer daemon-secret");
            }
            let mut req = req.body(axum::body::Body::empty()).unwrap();
            req.extensions_mut()
                .insert(ConnectInfo("192.0.2.2:5000".parse::<SocketAddr>().unwrap()));
            assert_eq!(app.clone().oneshot(req).await.unwrap().status(), expected);
        }
        let mut local = Request::builder()
            .uri("/daemon/v1/engines")
            .header(header::HOST, "127.0.0.1:8488")
            .body(axum::body::Body::empty())
            .unwrap();
        local
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:5000".parse::<SocketAddr>().unwrap()));
        assert_eq!(
            app.clone().oneshot(local).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let mut local = Request::builder()
            .uri("/daemon/v1/engines")
            .header(header::HOST, "127.0.0.1:8488")
            .header(header::COOKIE, "refact_daemon_auth=daemon-secret")
            .body(axum::body::Body::empty())
            .unwrap();
        local
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:5000".parse::<SocketAddr>().unwrap()));
        assert_eq!(app.oneshot(local).await.unwrap().status(), StatusCode::OK);
    }

    #[test]
    fn exact_loopback_only() {
        for host in [
            "localhost",
            "127.1",
            "127.0.0.2",
            "example.com",
            "http://127.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(endpoint(host, 8001).is_err());
        }
        assert!(endpoint("127.0.0.1", 0).is_err());
        assert_eq!(endpoint("::1", 8001).unwrap(), "http://[::1]:8001");
        assert!(serde_json::from_value::<AttachRequest>(
            json!({"host":"127.0.0.1","port":8001,"token":"secret"})
        )
        .is_err());
    }
    #[test]
    fn console_rejects_cross_origin_and_simple_mutation() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8488".parse().unwrap());
        assert!(trusted_headers(&headers, &Method::GET, false));
        assert!(!trusted_headers(&headers, &Method::POST, false));
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:8488".parse().unwrap());
        assert!(trusted_headers(&headers, &Method::POST, false));
        headers.insert(header::HOST, "svakhreev-work.local:8488".parse().unwrap());
        headers.insert(
            header::ORIGIN,
            "http://svakhreev-work.local:8488".parse().unwrap(),
        );
        assert!(trusted_headers(&headers, &Method::POST, true));
        assert!(!trusted_headers(&headers, &Method::POST, false));
        headers.insert(header::ORIGIN, "http://127.0.0.1:9999".parse().unwrap());
        assert!(!trusted_headers(&headers, &Method::POST, false));
    }
}
