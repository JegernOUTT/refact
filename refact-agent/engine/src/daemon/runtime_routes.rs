//! Selection and cache preparation only. No installation, activation, restart or worker drain.
use super::*;
use crate::daemon::runtime_coordinator::{RevisionRequest, RuntimeCoordinator, SelectRequest, StageKey};
use axum::http::Request;
use hyper::body::HttpBody;
use hyper::Body;
use refact_self_update::public_release::{PublicTransport, ReadFuture, ReqwestPublicTransport};
use serde::de::DeserializeOwned;

type RouteState = (Arc<DaemonState>, u16);
const BODY_LIMIT: usize = 4096;

pub(super) fn routes() -> Router<RouteState> {
    Router::new()
        .route("/daemon/v1/runtime/status", get(runtime_status))
        .route("/daemon/v1/runtime/select", post(runtime_select))
        .route("/daemon/v1/runtime/resolve", post(runtime_resolve))
        .route("/daemon/v1/runtime/stage", post(runtime_stage))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusQuery {
    manifest_digest: Option<String>,
    selection_revision: Option<u64>,
}

fn error(code: StatusCode, message: impl Into<String>) -> Response {
    (code, Json(json!({"error": message.into()}))).into_response()
}

fn mutation_headers(headers: &HeaderMap) -> Result<(), Response> {
    let origins: Vec<_> = headers.get_all("origin").iter().collect();
    if !origins.is_empty() {
        let trusted = origins.len() == 1
            && origins[0].to_str().ok().is_some_and(|origin| {
                url::Url::parse(origin).ok().is_some_and(|parsed| {
                    matches!(parsed.scheme(), "http" | "https" | "vscode-webview")
                        && parsed.host_str().is_some()
                        && parsed.username().is_empty()
                        && parsed.password().is_none()
                        && parsed.query().is_none()
                        && parsed.fragment().is_none()
                        && &parsed[..url::Position::AfterPort] == origin
                        && (crate::daemon::auth::origin_is_trusted_ide_webview(origin)
                            || (matches!(parsed.scheme(), "http" | "https")
                                && headers.get_all("host").iter().count() == 1
                                && headers
                                    .get("host")
                                    .and_then(|host| host.to_str().ok())
                                    .is_some_and(|host| {
                                        parsed[url::Position::BeforeHost..url::Position::AfterPort]
                                            .eq_ignore_ascii_case(host)
                                    })))
                })
            });
        if !trusted {
            return Err(error(
                StatusCode::FORBIDDEN,
                "untrusted runtime request origin",
            ));
        }
    }
    let types: Vec<_> = headers.get_all("content-type").iter().collect();
    let valid = types.len() == 1
        && types[0].to_str().ok().is_some_and(|value| {
            let mut parts = value.split(';');
            if !parts
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("application/json")
            {
                return false;
            }
            let charset = parts.next().is_none_or(|parameter| {
                parameter
                    .trim()
                    .split_once('=')
                    .is_some_and(|(name, value)| {
                        name.trim().eq_ignore_ascii_case("charset")
                            && (value.trim().eq_ignore_ascii_case("utf-8")
                                || value.trim().eq_ignore_ascii_case("\"utf-8\""))
                    })
            });
            charset && parts.next().is_none()
        });
    if !valid {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "exactly one application/json content-type required",
        ));
    }
    Ok(())
}

fn coordinator(request: &Request<Body>) -> Result<RuntimeCoordinator, String> {
    #[cfg(test)]
    if let Some(c) = request.extensions().get::<RuntimeCoordinator>() {
        return Ok(c.clone());
    }
    let _ = request;
    RuntimeCoordinator::host()
}

// Sized wrapper permits an injected hermetic PublicTransport without changing core APIs.
#[derive(Clone)]
struct Transport(Arc<dyn PublicTransport + Send>);
impl PublicTransport for Transport {
    fn get<'a>(&'a self, url: &'a str, asset: bool, max_bytes: usize) -> ReadFuture<'a> {
        self.0.get(url, asset, max_bytes)
    }
}
fn transport(request: &Request<Body>) -> Result<Transport, String> {
    #[cfg(test)]
    if let Some(t) = request.extensions().get::<Transport>() {
        return Ok(t.clone());
    }
    let _ = request;
    ReqwestPublicTransport::new()
        .map(|t| Transport(Arc::new(t)))
        .map_err(|e| format!("public transport unavailable: {e:?}"))
}

async fn bounded_json<T: DeserializeOwned>(mut body: Body) -> Result<T, Response> {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(|_| error(StatusCode::BAD_REQUEST, "request body unreadable"))?;
        if chunk.len() > BODY_LIMIT.saturating_sub(bytes.len()) {
            return Err(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "runtime request exceeds 4096 bytes",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        error(
            StatusCode::BAD_REQUEST,
            format!("invalid runtime request: {e}"),
        )
    })
}

async fn check_idle(state: &DaemonState) -> Result<(), String> {
    let update = state.update_state.lock().await;
    if state.is_shutting_down() || update_phase_active(&update.phase) {
        return Err("daemon update/shutdown active".into());
    }
    Ok(())
}

async fn runtime_status(request: Request<Body>) -> Response {
    let query = match Query::<StatusQuery>::try_from_uri(request.uri()) {
        Ok(Query(q)) => q,
        Err(e) => return error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let key = match (query.manifest_digest, query.selection_revision) {
        (None, None) => None,
        (Some(manifest_digest), Some(selection_revision))
            if selection_revision > 0
                && manifest_digest.len() == 64
                && manifest_digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            Some(StageKey {
                manifest_digest,
                selection_revision,
            })
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "manifest_digest and positive selection_revision must be supplied together",
            )
        }
    };
    let coordinator = match coordinator(&request) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match tokio::task::spawn_blocking(move || coordinator.status(key)).await {
        Ok(status) => Json(status).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn runtime_select(State((state, _)): State<RouteState>, request: Request<Body>) -> Response {
    if !buddy_user_authorized(&state, request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err(response) = mutation_headers(request.headers()) {
        return response;
    }
    let coordinator = match coordinator(&request) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let selection = match bounded_json::<SelectRequest>(request.into_body()).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    // Lock order stays execution -> update. SelectionStore's own filesystem CAS
    // remains authoritative even if a disconnected caller drops these async guards.
    let execution = crate::buddy::resource_execution::execution_gate()
        .lock()
        .await;
    let update = state.update_state.lock().await;
    if state.is_shutting_down() || update_phase_active(&update.phase) {
        return error(StatusCode::CONFLICT, "daemon update/shutdown active");
    }
    // The quick CAS owns no network work; guards remain held through completion.
    let result = tokio::task::spawn_blocking(move || coordinator.select(selection)).await;
    drop(update);
    drop(execution);
    match result {
        Ok(Ok(snapshot)) => Json(snapshot).into_response(),
        Ok(Err(e)) => error(StatusCode::CONFLICT, e),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn runtime_resolve(State((state, _)): State<RouteState>, request: Request<Body>) -> Response {
    prepare(state, request, false).await
}
async fn runtime_stage(State((state, _)): State<RouteState>, request: Request<Body>) -> Response {
    prepare(state, request, true).await
}

async fn prepare(state: Arc<DaemonState>, request: Request<Body>, stage: bool) -> Response {
    if !buddy_user_authorized(&state, request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err(response) = mutation_headers(request.headers()) {
        return response;
    }
    let coordinator = match coordinator(&request) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let transport = match transport(&request) {
        Ok(t) => t,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let revision = match bounded_json::<RevisionRequest>(request.into_body()).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    {
        let _execution = crate::buddy::resource_execution::execution_gate()
            .lock()
            .await;
        if let Err(e) = check_idle(&state).await {
            return error(StatusCode::CONFLICT, e);
        }
    }
    // Cache-only operations do not claim the legacy updater's phase. In particular,
    // neither global mutex is held over public IO; ordinary workers remain live.
    // Core fences publication against selection CAS, not against installed-image changes.
    let result = if stage {
        coordinator
            .stage(&transport, revision)
            .await
            .map(|r| json!(r))
    } else {
        coordinator
            .resolve(&transport, revision)
            .await
            .map(|r| json!(r))
    };
    let _execution = crate::buddy::resource_execution::execution_gate()
        .lock()
        .await;
    if let Err(e) = check_idle(&state).await {
        return error(
            StatusCode::CONFLICT,
            if stage {
                format!(
                    "{e}; stage outcome may already be cached; inspect runtime status before retry"
                )
            } else {
                e
            },
        );
    }
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

#[cfg(test)]
#[path = "runtime_routes_tests.rs"]
mod tests;
