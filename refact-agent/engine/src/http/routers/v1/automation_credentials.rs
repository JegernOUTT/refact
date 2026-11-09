//! Operator-only private credential enrollment. This module never grants source execution.
//! Register on the existing host router, never on a model/proxy tool surface.
use std::{net::SocketAddr, path::PathBuf};

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    app_state::AppState,
    scheduler::credentials::{AutomationCredentialStore, CredentialIdentity},
};

type Request = hyper::Request<hyper::Body>;
type Error = (StatusCode, Json<Value>);
const BODY_LIMIT: usize = 32 * 1024;
fn error(status: StatusCode, message: &'static str) -> Error {
    (status, Json(json!({"error":message})))
}
fn denied() -> Error {
    error(
        StatusCode::FORBIDDEN,
        "trusted same-origin host operator required",
    )
}

/// Origin is mandatory for secret mutations, including bearer-authenticated ones.
/// Host must be a literal loopback authority (or localhost); DNS rebinding and
/// opaque/sandboxed App frames are not host capabilities. CORS is irrelevant.
fn check_origin(headers: &HeaderMap, mutation: bool) -> Result<(), Error> {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|h| h != "same-origin" && h != "none")
    {
        return Err(denied());
    }
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(denied)?;
    let target = url::Url::parse(&format!("http://{host}")).map_err(|_| denied())?;
    let local = match target.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if !local
        || target.path() != "/"
        || !target.username().is_empty()
        || target.password().is_some()
    {
        return Err(denied());
    }
    let Some(origin) = headers.get("origin") else {
        return if mutation { Err(denied()) } else { Ok(()) };
    };
    let raw = origin.to_str().map_err(|_| denied())?;
    let origin = url::Url::parse(raw).map_err(|_| denied())?;
    // This worker serves HTTP directly; forwarded headers cannot elevate a caller.
    if origin.scheme() != "http"
        || origin.origin() != target.origin()
        || origin.origin().ascii_serialization() != raw
    {
        return Err(denied());
    }
    Ok(())
}
fn authorize(
    app: &AppState,
    headers: &HeaderMap,
    peer: Option<ConnectInfo<SocketAddr>>,
    mutation: bool,
) -> Result<(), Error> {
    check_origin(headers, mutation)?;
    super::work::authorize(app, headers, peer.map(|p| p.0))
        .map_err(|_| error(StatusCode::UNAUTHORIZED, "host authorization required"))
}

/// Do not silently use a process-global active project in a multi-root worker.
async fn project(app: &AppState) -> Result<PathBuf, Error> {
    let roots = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    if roots.len() != 1 {
        return Err(error(
            StatusCode::CONFLICT,
            "credential enrollment requires exactly one operator project",
        ));
    }
    std::fs::canonicalize(&roots[0]).map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "operator project unavailable",
        )
    })
}
fn store_error(message: String) -> Error {
    // Never return parser/debug errors or credential values to clients/logs.
    if message == "credential revision conflict" {
        error(
            StatusCode::CONFLICT,
            "credential revision conflict; refresh metadata",
        )
    } else {
        error(StatusCode::BAD_REQUEST, "credential operation refused; check identity, revision, value limits, and private storage")
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(metadata))
        .route("/create", post(create))
        .route("/rotate", post(rotate))
        .route("/remove", post(remove))
}

pub async fn metadata(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    authorize(&app, &headers, peer, false)?;
    let root = project(&app).await?;
    if !cfg!(unix) {
        return Ok(Json(
            json!({"supported":false,"project_root":root,"credentials":[],"reason":"Private credential enrollment is unsupported on this platform"}),
        ));
    }
    let store = AutomationCredentialStore::open(&root, None).map_err(store_error)?;
    let credentials = store.list_metadata().map_err(store_error)?;
    Ok(Json(
        json!({"supported":true,"project_root":root,"credentials":credentials}),
    ))
}

// Deliberately no Debug, Serialize, flatten, approval flag, or generic export port.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    project_root: PathBuf,
    identity: CredentialIdentity,
    secret: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rotate {
    project_root: PathBuf,
    id: String,
    expected_secret_revision: String,
    secret: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remove {
    project_root: PathBuf,
    id: String,
    expected_secret_revision: String,
}
async fn body<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, Error> {
    if request
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        != Some("application/json")
    {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "application/json required",
        ));
    }
    use hyper::body::HttpBody;
    let mut body = request.into_body();
    let mut bytes = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(|_| error(StatusCode::BAD_REQUEST, "invalid credential body"))?;
        if bytes.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "credential request exceeds limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid credential request"))
}
fn scoped_store(root: &PathBuf, expected: &PathBuf) -> Result<AutomationCredentialStore, Error> {
    if root != expected {
        return Err(error(
            StatusCode::CONFLICT,
            "operator project changed; refresh metadata",
        ));
    }
    if !cfg!(unix) {
        return Err(error(
            StatusCode::NOT_IMPLEMENTED,
            "private credential enrollment unsupported on this platform",
        ));
    }
    AutomationCredentialStore::open(root, None).map_err(store_error)
}
pub async fn create(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    request: Request,
) -> Result<Json<Value>, Error> {
    authorize(&app, request.headers(), peer, true)?;
    let root = project(&app).await?;
    let request: Create = body(request).await?;
    let store = scoped_store(&root, &request.project_root)?;
    let metadata = store
        .set(request.identity, request.secret, None)
        .map_err(store_error)?;
    Ok(Json(
        json!({"metadata":metadata,"source_authorization":"not_granted"}),
    ))
}
pub async fn rotate(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    request: Request,
) -> Result<Json<Value>, Error> {
    authorize(&app, request.headers(), peer, true)?;
    let root = project(&app).await?;
    let request: Rotate = body(request).await?;
    let store = scoped_store(&root, &request.project_root)?;
    let metadata = store
        .rotate(
            &request.id,
            &request.expected_secret_revision,
            request.secret,
        )
        .map_err(store_error)?;
    Ok(Json(
        json!({"metadata":metadata,"source_authorization":"unchanged"}),
    ))
}
pub async fn remove(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    request: Request,
) -> Result<Json<Value>, Error> {
    authorize(&app, request.headers(), peer, true)?;
    let root = project(&app).await?;
    let request: Remove = body(request).await?;
    let store = scoped_store(&root, &request.project_root)?;
    store
        .remove(&request.id, &request.expected_secret_revision)
        .map_err(store_error)?;
    Ok(Json(json!({"removed":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(origin: Option<&str>, host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("host", host.parse().unwrap());
        if let Some(origin) = origin {
            headers.insert("origin", origin.parse().unwrap());
        }
        headers
    }
    #[test]
    fn mutations_require_exact_nonopaque_loopback_origin() {
        for origin in [
            None,
            Some("null"),
            Some("https://localhost:8001"),
            Some("http://localhost:8002"),
            Some("http://evil.example"),
            Some("http://localhost:8001/path"),
        ] {
            assert!(check_origin(&headers(origin, "localhost:8001"), true).is_err());
        }
        assert!(check_origin(
            &headers(Some("http://localhost:8001"), "localhost:8001"),
            true
        )
        .is_ok());
        assert!(check_origin(
            &headers(Some("http://127.0.0.1:8001"), "127.0.0.1:8001"),
            true
        )
        .is_ok());
        assert!(check_origin(&headers(Some("http://evil.example"), "evil.example"), true).is_err());
        let mut h = headers(Some("http://localhost:8001"), "localhost:8001");
        h.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(check_origin(&h, true).is_err());
    }
    #[tokio::test]
    async fn bounded_parser_redacts_raw_errors_and_rejects_approval_flags() {
        let request = hyper::Request::builder()
            .header("content-type", "application/json")
            .body(axum::body::Body::from("fixture-secret-invalid-json"))
            .unwrap();
        let err = body::<Create>(request).await.err().unwrap();
        assert!(!err.1 .0.to_string().contains("fixture-secret"));
        let request = hyper::Request::builder()
            .header("content-type", "application/json")
            .body(axum::body::Body::from("x".repeat(BODY_LIMIT + 1)))
            .unwrap();
        assert_eq!(
            body::<Create>(request).await.err().unwrap().0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert!(serde_json::from_value::<Remove>(
            json!({"project_root":"/tmp","id":"x","expected_secret_revision":"r","approved":true})
        )
        .is_err());
    }
}
