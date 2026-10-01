#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Component, Path as FsPath, PathBuf};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use hyper::body::Bytes;
use refact_lsp::refact_browser::BrowserLaunchOptions;
use serde::Deserialize;
use serde_json::json;

pub fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/browser_fixtures")
}

pub fn find_executable(name: &FsPath, path: Option<&OsString>) -> Option<PathBuf> {
    if name.components().count() > 1 || name.is_absolute() {
        return name.is_file().then(|| name.to_path_buf());
    }
    path.and_then(|value| {
        std::env::split_paths(value)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

pub fn discover_chrome_with(
    chrome_override: Option<OsString>,
    path: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(chrome_override) = chrome_override {
        if let Some(path) = find_executable(FsPath::new(&chrome_override), path.as_ref()) {
            return Some(path);
        }
    }
    ["chrome", "chromium", "google-chrome", "chromium-browser"]
        .iter()
        .find_map(|name| find_executable(FsPath::new(name), path.as_ref()))
}

pub fn discover_chrome() -> Option<PathBuf> {
    discover_chrome_with(std::env::var_os("CHROME"), std::env::var_os("PATH"))
}

pub fn e2e_enabled() -> bool {
    std::env::var("REFACT_BROWSER_E2E").as_deref() == Ok("1") && discover_chrome().is_some()
}

pub fn print_skip() {
    eprintln!(
        "skipped: set REFACT_BROWSER_E2E=1 and install Chrome, Chromium, google-chrome, or chromium-browser"
    );
}

pub fn e2e_launch_options(chrome_path: Option<PathBuf>) -> BrowserLaunchOptions {
    BrowserLaunchOptions {
        headless: true,
        chrome_path,
        idle_timeout: Some(Duration::from_secs(120)),
        ..BrowserLaunchOptions::default()
    }
}

#[derive(Deserialize)]
pub struct SlowEchoQuery {
    ms: Option<u64>,
}

pub async fn slow_echo(Query(query): Query<SlowEchoQuery>) -> impl IntoResponse {
    let delay_ms = query.ms.unwrap_or(0).min(5_000);
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    axum::Json(json!({"echo": "ok", "delay_ms": delay_ms}))
}

pub async fn download() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=browser-fixture.txt",
        )
        .body(Body::from("browser fixture download\n"))
        .unwrap()
}

pub async fn upload(headers: HeaderMap, body: Bytes) -> Response<Body> {
    let is_multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("multipart/form-data; boundary="))
        .unwrap_or(false);
    if !is_multipart {
        return Response::builder()
            .status(StatusCode::UNSUPPORTED_MEDIA_TYPE)
            .body(Body::from("multipart/form-data required"))
            .unwrap();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"accepted": true, "bytes": body.len()}).to_string(),
        ))
        .unwrap()
}

pub async fn route_data(headers: HeaderMap) -> impl IntoResponse {
    let source = headers
        .get("x-route-test")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("origin");
    axum::Json(json!({"source": source}))
}

pub async fn session_login() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::SET_COOKIE, "fixture_session=logged-in; Path=/")
        .body(Body::from("<html><body><h1>logged in</h1></body></html>"))
        .unwrap()
}

pub async fn session_probe(headers: HeaderMap) -> Response<Body> {
    let session = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(';')
                .map(str::trim)
                .find_map(|pair| pair.strip_prefix("fixture_session="))
        })
        .unwrap_or("anonymous")
        .to_string();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::SET_COOKIE, "api_issued=from-api; Path=/")
        .body(Body::from(json!({"session": session}).to_string()))
        .unwrap()
}

pub async fn route_redirect() -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/api/data")
        .body(Body::empty())
        .unwrap()
}

pub fn content_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub async fn static_fixture(
    State(root): State<PathBuf>,
    AxumPath(path): AxumPath<String>,
) -> Response<Body> {
    let relative = FsPath::new(&path);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .unwrap();
    }
    let file = root.join(relative);
    match tokio::fs::read(&file).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type(&file))
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    }
}

pub struct FixtureServer {
    pub base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    pub async fn start() -> Result<Self, String> {
        let root = fixture_root();
        if !root.is_dir() {
            return Err(format!(
                "fixture directory does not exist: {}",
                root.display()
            ));
        }
        let app = Router::new()
            .route("/slow-echo", get(slow_echo))
            .route("/api/data", get(route_data))
            .route("/api/redirect", get(route_redirect))
            .route("/login", get(session_login))
            .route("/api/session", get(session_probe))
            .route("/download", get(download))
            .route("/upload", post(upload))
            .route("/*path", get(static_fixture))
            .with_state(root);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let listener = listener.into_std().map_err(|error| error.to_string())?;
        let server = axum::Server::from_tcp(listener)
            .map_err(|error| error.to_string())?
            .serve(app.into_make_service());
        let task = tokio::spawn(async move {
            let _ = server.await;
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            task,
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
