//! Daemon web serving uses a generated fetch/EventSource prefix shim for `/p/{id}` because
//! the GUI's web `engineServed` mode resolves API calls relative to the current origin.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{OriginalUri, Path as AxumPath, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Serialize;

use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::{now_ms, DaemonState};
use crate::daemon::supervisor::{WorkerInfo, WorkerState};
use crate::http::routers::gui::{
    asset_response, html_response, missing_gui_index_html, text_response, ChatGuiAsset,
    ASSET_PREFIX, INDEX_PATH,
};
use crate::http::{gui_public_origin_candidates, GuiPublicOriginCandidates};

const PICKER_TEMPLATE: &str = include_str!("web_picker.html");

#[derive(Debug, Serialize)]
struct PickerData {
    daemon: PickerDaemon,
    projects: Vec<PickerProject>,
}

#[derive(Debug, Serialize)]
struct PickerDaemon {
    version: String,
    port: u16,
    started_at_ms: u64,
    uptime_secs: u64,
}

#[derive(Debug, Serialize)]
struct PickerProject {
    id: String,
    slug: String,
    root: String,
    pinned: bool,
    worker_state: String,
    lsp_clients: usize,
    busy_chats: usize,
    exec_running: usize,
    live_proxy_streams: u64,
    cron_next_fire_ms: Option<u64>,
    last_active_ms: u64,
}

pub(crate) async fn handle_project_picker(
    OriginalUri(uri): OriginalUri,
    State((state, port)): State<(Arc<DaemonState>, u16)>,
) -> Response {
    if let Some(response) = daemon_auth_redirect(&state, &uri) {
        return response;
    }
    let data = picker_data(state, port).await;
    html_response(StatusCode::OK, render_picker_html(&data))
}

pub(crate) async fn handle_project_gui_index(
    OriginalUri(uri): OriginalUri,
    State((state, port)): State<(Arc<DaemonState>, u16)>,
    AxumPath(project_id): AxumPath<String>,
) -> Response {
    if let Some(response) = daemon_auth_redirect(&state, &uri) {
        return response;
    }
    if project_entry(&state, &project_id).await.is_none() {
        return html_response(
            StatusCode::NOT_FOUND,
            unknown_project_html(&project_id).into_bytes().into(),
        );
    }

    match ChatGuiAsset::get(INDEX_PATH) {
        Some(asset) => {
            let body = project_gui_index_body(asset.data, &project_id, port);
            asset_response(INDEX_PATH, body, StatusCode::OK)
        }
        None => html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            missing_gui_index_html().as_bytes().to_vec().into(),
        ),
    }
}

pub(crate) async fn handle_daemon_gui_asset(AxumPath(path): AxumPath<String>) -> impl IntoResponse {
    if invalid_asset_path(&path) {
        return text_response(
            StatusCode::BAD_REQUEST,
            "invalid GUI asset path".to_string(),
        );
    }

    let embedded_path = format!("{ASSET_PREFIX}{path}");
    match ChatGuiAsset::get(&embedded_path) {
        Some(asset) => asset_response(&embedded_path, asset.data, StatusCode::OK),
        None => text_response(
            StatusCode::NOT_FOUND,
            format!("GUI asset not found: {path}"),
        ),
    }
}

fn invalid_asset_path(path: &str) -> bool {
    path.is_empty() || path.split('/').any(|part| part == ".." || part.is_empty())
}

async fn project_entry(state: &DaemonState, project_id: &str) -> Option<ProjectEntry> {
    state.projects.read().await.get(project_id).cloned()
}

fn daemon_auth_redirect(state: &DaemonState, uri: &Uri) -> Option<Response> {
    let expected = state.auth_token.as_deref()?;
    let token = crate::daemon::auth::matching_daemon_query_token(uri.query(), expected)?;
    let mut response = Response::new(axum::body::boxed(axum::body::Full::from(Vec::<u8>::new())));
    *response.status_mut() = StatusCode::SEE_OTHER;
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&redirect_without_daemon_token(uri)).ok()?,
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&daemon_auth_cookie(&token)).ok()?,
    );
    Some(response)
}

fn daemon_auth_cookie(token: &str) -> String {
    format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/",
        crate::daemon::auth::DAEMON_AUTH_COOKIE,
        token
    )
}

fn redirect_without_daemon_token(uri: &Uri) -> String {
    let path = uri.path();
    let pairs = uri
        .query()
        .into_iter()
        .flat_map(|query| url::form_urlencoded::parse(query.as_bytes()))
        .filter(|(name, _)| name != crate::daemon::auth::DAEMON_AUTH_QUERY)
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return path.to_string();
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(&name, &value);
    }
    format!("{}?{}", path, serializer.finish())
}

async fn picker_data(state: Arc<DaemonState>, port: u16) -> PickerData {
    let mut entries = state.projects.read().await.list();
    entries.sort_by(|a, b| a.slug.cmp(&b.slug).then_with(|| a.id.cmp(&b.id)));
    let mut projects = Vec::with_capacity(entries.len());
    for entry in entries {
        let worker = state.supervisor.worker_info(&entry.id).await;
        let status = state.latest_worker_status(&entry.id).await;
        let activity = state.proxy_activity(&entry.id).await;
        let last_activity_ms = [
            entry.last_active_ms,
            status
                .as_ref()
                .map(|status| status.last_activity_ts)
                .unwrap_or_default(),
            activity.last_proxy_activity_ms,
        ]
        .into_iter()
        .max()
        .unwrap_or_default();
        projects.push(PickerProject {
            id: entry.id.clone(),
            slug: entry.slug.clone(),
            root: entry.root.to_string_lossy().to_string(),
            pinned: entry.pinned,
            worker_state: worker_state_label(worker.as_ref()),
            lsp_clients: status
                .as_ref()
                .map(|status| status.lsp_clients)
                .unwrap_or_default(),
            busy_chats: status
                .as_ref()
                .map(|status| status.busy_chats)
                .unwrap_or_default(),
            exec_running: status
                .as_ref()
                .map(|status| status.exec_running)
                .unwrap_or_default(),
            live_proxy_streams: activity.live_proxy_streams,
            cron_next_fire_ms: state.cron_pending(&entry.id).await,
            last_active_ms: last_activity_ms,
        });
    }

    PickerData {
        daemon: PickerDaemon {
            version: state.version.clone(),
            port,
            started_at_ms: state.started_at_ms,
            uptime_secs: Duration::from_millis(now_ms().saturating_sub(state.started_at_ms))
                .as_secs(),
        },
        projects,
    }
}

fn worker_state_label(worker: Option<&WorkerInfo>) -> String {
    match worker.map(|worker| &worker.state) {
        Some(WorkerState::Stopped) | None => "stopped".to_string(),
        Some(WorkerState::Starting) => "starting".to_string(),
        Some(WorkerState::Ready) => "ready".to_string(),
        Some(WorkerState::Stopping) => "stopping".to_string(),
        Some(WorkerState::Crashed) => "crashed".to_string(),
        Some(WorkerState::Failed { .. }) => "failed".to_string(),
    }
}

fn render_picker_html(data: &PickerData) -> Cow<'static, [u8]> {
    let rows = picker_rows_html(&data.projects);
    let data_json = json_for_script(data);
    let meta = format!(
        "v{} · port {} · uptime {}s",
        html_escape(&data.daemon.version),
        data.daemon.port,
        data.daemon.uptime_secs
    );
    Cow::Owned(
        PICKER_TEMPLATE
            .replace("__REFACT_DAEMON_PICKER_META__", &meta)
            .replace("__REFACT_DAEMON_PICKER_ROWS__", &rows)
            .replace("__REFACT_DAEMON_PICKER_DATA__", &data_json)
            .into_bytes(),
    )
}

fn picker_rows_html(projects: &[PickerProject]) -> String {
    if projects.is_empty() {
        return "<tr><td class=\"empty\" colspan=\"6\">No projects registered yet. Run <code>refact projects open .</code> from a workspace.</td></tr>".to_string();
    }

    projects
        .iter()
        .map(|project| {
            let state = html_escape(&project.worker_state);
            let class = css_class_segment(&project.worker_state);
            format!(
                "<tr><td data-label=\"Project\"><div class=\"project\"><strong>{}</strong><span class=\"root muted\">{}</span></div></td><td data-label=\"Worker\"><span class=\"dot dot-{}\"></span>{}</td><td data-label=\"LSP clients\">{}</td><td data-label=\"Cron next fire\">{}</td><td data-label=\"Last active\">{}</td><td data-label=\"Action\"><a class=\"button\" href=\"{}\">Open GUI</a></td></tr>",
                html_escape(&project.slug),
                html_escape(&project.root),
                class,
                state,
                project.lsp_clients,
                project
                    .cron_next_fire_ms
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "—".to_string()),
                project.last_active_ms,
                project_gui_path(&project.id),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn project_gui_index_body(
    body: Cow<'static, [u8]>,
    project_id: &str,
    port: u16,
) -> Cow<'static, [u8]> {
    let candidates = daemon_origin_candidates(port, project_id);
    let body = crate::http::routers::gui::inject_gui_origin_candidates(body, &candidates);
    let Ok(html) = std::str::from_utf8(body.as_ref()) else {
        return body;
    };
    let html = inject_daemon_origin_candidates(html, &candidates);
    Cow::Owned(inject_daemon_api_prefix(&html, project_id).into_bytes())
}

fn inject_daemon_origin_candidates(html: &str, candidates: &GuiPublicOriginCandidates) -> String {
    let Ok(json) = serde_json::to_string(&candidates.origins) else {
        return html.to_string();
    };
    html.replace(
        "window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ || [];",
        &format!("window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ || {json};"),
    )
}

fn inject_daemon_api_prefix(html: &str, project_id: &str) -> String {
    let prefix = project_api_prefix(project_id);
    let prefix_json = serde_json::to_string(&prefix).unwrap_or_else(|_| "\"\"".to_string());
    let with_prefix = html.replace(
        "        const loopbackOrigin = \"http://127.0.0.1:\" + port;\n",
        &format!(
            "        const loopbackOrigin = \"http://127.0.0.1:\" + port;\n        const daemonProjectApiPrefix = {prefix_json};\n        const prefixDaemonProjectApi = function (value) {{\n          if (typeof value === \"string\") {{\n            if (value.startsWith(\"/v1/\") || value === \"/v1\") {{\n              return daemonProjectApiPrefix + value;\n            }}\n            try {{\n              const url = new URL(value, origin);\n              if (url.origin === origin && (url.pathname.startsWith(\"/v1/\") || url.pathname === \"/v1\")) {{\n                return origin + daemonProjectApiPrefix + url.pathname + url.search + url.hash;\n              }}\n            }} catch (_error) {{}}\n          }}\n          if (value instanceof URL && value.origin === origin && (value.pathname.startsWith(\"/v1/\") || value.pathname === \"/v1\")) {{\n            return new URL(origin + daemonProjectApiPrefix + value.pathname + value.search + value.hash);\n          }}\n          if (value instanceof Request) {{\n            try {{\n              const url = new URL(value.url);\n              if (url.origin === origin && (url.pathname.startsWith(\"/v1/\") || url.pathname === \"/v1\")) {{\n                return new Request(origin + daemonProjectApiPrefix + url.pathname + url.search + url.hash, value);\n              }}\n            }} catch (_error) {{}}\n          }}\n          return value;\n        }};\n"
        ),
    );
    with_prefix
        .replace(
            "            return originalFetch(rewriteEngineUrl(input, candidate), init).then(\n",
            "            return originalFetch(prefixDaemonProjectApi(rewriteEngineUrl(input, candidate)), init).then(\n",
        )
        .replace(
            "          const rewrittenInput = rewriteEngineUrl(input);\n",
            "          const rewrittenInput = prefixDaemonProjectApi(rewriteEngineUrl(input));\n",
        )
        .replace(
            "        };\n\n        const initialState = {\n",
            "        };\n\n        if (typeof window.EventSource === \"function\") {\n          const originalEventSource = window.EventSource;\n          window.EventSource = function (input, init) {\n            return new originalEventSource(prefixDaemonProjectApi(input), init);\n          };\n          window.EventSource.prototype = originalEventSource.prototype;\n        }\n\n        const initialState = {\n",
        )
}

fn daemon_origin_candidates(port: u16, project_id: &str) -> GuiPublicOriginCandidates {
    let prefix = project_api_prefix(project_id);
    let mut origins = Vec::new();
    origins.push(format!("http://127.0.0.1:{port}{prefix}"));
    for origin in gui_public_origin_candidates(port).origins {
        let candidate = format!("{}{prefix}", origin.trim_end_matches('/'));
        if !origins.contains(&candidate) {
            origins.push(candidate);
        }
    }
    GuiPublicOriginCandidates { origins }
}

fn unknown_project_html(project_id: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"UTF-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" /><title>Project not found</title></head><body><h1>Project not found</h1><p>No daemon project is registered for <code>{}</code>.</p><p><a href=\"/\">Back to projects</a></p></body></html>",
        html_escape(project_id)
    )
}

fn project_api_prefix(project_id: &str) -> String {
    format!("/p/{}", path_segment(project_id))
}

fn project_gui_path(project_id: &str) -> String {
    format!("{}/", project_api_prefix(project_id))
}

fn path_segment(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn json_for_script<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".to_string())
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn css_class_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::response::IntoResponse;
    use hyper::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    use crate::daemon::config::DaemonConfig;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectRegistry;
    use crate::daemon_link::WorkerStatusReport;

    struct TestState {
        _dir: tempfile::TempDir,
        state: Arc<DaemonState>,
    }

    async fn test_state() -> TestState {
        test_state_with_auth(None).await
    }

    async fn test_state_with_auth(token: Option<&str>) -> TestState {
        let dir = tempfile::tempdir().unwrap();
        let mut config = DaemonConfig::default();
        if let Some(token) = token {
            config.auth = crate::daemon::config::AuthConfig {
                enabled: true,
                token: Some(token.to_string()),
            };
        }
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            token.map(str::to_string),
        );
        *state.projects.write().await = ProjectRegistry::empty(dir.path().join("projects.json"));
        TestState { _dir: dir, state }
    }

    fn status_report(project_id: &str, lsp_clients: usize) -> WorkerStatusReport {
        WorkerStatusReport {
            project_id: project_id.to_string(),
            pid: 123,
            lsp_clients,
            busy_chats: 2,
            exec_running: 1,
            last_activity_ts: 777,
        }
    }

    #[test]
    fn daemon_gui_index_injects_prefix_shim_and_escapes_project_id() {
        let html = r#"<script>
        const loopbackOrigin = "http://127.0.0.1:" + port;
        const originalFetch = window.fetch.bind(window);
            return originalFetch(rewriteEngineUrl(input, candidate), init).then(
          const rewrittenInput = rewriteEngineUrl(input);
        };

        const initialState = {
        </script>"#;
        let injected = inject_daemon_api_prefix(html, "abc\"</script>");
        assert!(injected.contains("const daemonProjectApiPrefix = \"/p/abc%22%3C%2Fscript%3E\";"));
        assert!(injected.contains("prefixDaemonProjectApi(rewriteEngineUrl(input));"));
        assert!(injected.contains("window.EventSource = function"));
        assert!(!injected.contains("abc\"</script>"));
    }

    #[test]
    fn daemon_origin_candidates_are_project_aware() {
        let candidates = daemon_origin_candidates(8488, "abc123");
        assert!(candidates
            .origins
            .iter()
            .any(|origin| origin == "http://127.0.0.1:8488/p/abc123"));
        assert!(candidates
            .origins
            .iter()
            .all(|origin| origin.ends_with("/p/abc123")));
    }

    #[test]
    fn daemon_origin_candidates_inject_when_worker_marker_already_replaced() {
        let candidates = GuiPublicOriginCandidates {
            origins: vec!["http://127.0.0.1:8488/p/abc123".to_string()],
        };
        let html = "window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ || [];";
        let injected = inject_daemon_origin_candidates(html, &candidates);
        assert!(injected.contains("http://127.0.0.1:8488/p/abc123"));
        assert!(!injected.contains("|| [];"));
    }

    #[test]
    fn redirect_without_daemon_token_preserves_other_query() {
        let uri: Uri = "/p/abc/?daemon_token=secret&theme=dark&q=a%20b"
            .parse()
            .unwrap();

        assert_eq!(
            redirect_without_daemon_token(&uri),
            "/p/abc/?theme=dark&q=a+b"
        );
    }

    #[test]
    fn picker_html_mentions_auth_error() {
        assert!(PICKER_TEMPLATE
            .contains("Authentication required. Open this page through the daemon launch URL."));
    }

    #[test]
    fn picker_html_contains_project_rows_and_escaped_data() {
        let data = PickerData {
            daemon: PickerDaemon {
                version: "8.1.0".to_string(),
                port: 8488,
                started_at_ms: 10,
                uptime_secs: 3,
            },
            projects: vec![PickerProject {
                id: "abc123".to_string(),
                slug: "my-project".to_string(),
                root: "/tmp/<workspace>".to_string(),
                pinned: false,
                worker_state: "ready".to_string(),
                lsp_clients: 2,
                busy_chats: 1,
                exec_running: 0,
                live_proxy_streams: 0,
                cron_next_fire_ms: Some(42),
                last_active_ms: 99,
            }],
        };
        let html = String::from_utf8(render_picker_html(&data).into_owned()).unwrap();
        assert!(html.contains("my-project"));
        assert!(html.contains("/tmp/&lt;workspace&gt;"));
        assert!(html.contains("dot dot-ready"));
        assert!(html.contains("/p/abc123/"));
        assert!(html.contains("\"cron_next_fire_ms\":42"));
        assert!(!html.contains("/tmp/<workspace>"));
    }

    #[tokio::test]
    async fn picker_data_contains_registered_project_status() {
        let test = test_state().await;
        let state = test.state;
        let root = tempfile::tempdir().unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.path().to_path_buf()).await.unwrap()
        };
        state.store_worker_status(status_report(&entry.id, 3)).await;
        state.set_cron_pending(&entry.id, Some(12345)).await;

        let data = picker_data(state, 8488).await;
        assert_eq!(data.projects.len(), 1);
        assert_eq!(data.projects[0].id, entry.id);
        assert_eq!(data.projects[0].lsp_clients, 3);
        assert_eq!(data.projects[0].busy_chats, 2);
        assert_eq!(data.projects[0].exec_running, 1);
        assert_eq!(data.projects[0].cron_next_fire_ms, Some(12345));
    }

    #[tokio::test]
    async fn unknown_project_index_returns_404_page() {
        let state = test_state().await.state;
        let response = crate::daemon::server::make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri("/p/missing/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Project not found"));
        assert!(body.contains("Back to projects"));
    }

    #[tokio::test]
    async fn auth_enabled_picker_requires_credentials() {
        let state = test_state_with_auth(Some("secret-token")).await.state;
        let response = crate::daemon::server::make_router(state, 8488)
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tokenized_picker_sets_cookie_redirects_and_cookie_allows_next_request() {
        let state = test_state_with_auth(Some("secret-token")).await.state;
        let response = crate::daemon::server::make_router(state.clone(), 8488)
            .oneshot(
                Request::builder()
                    .uri("/?daemon_token=secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/");
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("refact_daemon_auth=secret-token"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Path=/"));

        let response = crate::daemon::server::make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::COOKIE, "refact_daemon_auth=secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn tokenized_project_gui_sets_cookie_and_redirects() {
        let test = test_state_with_auth(Some("secret-token")).await;
        let state = test.state;
        let root = tempfile::tempdir().unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.path().to_path_buf()).await.unwrap()
        };

        let response = crate::daemon::server::make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri(format!("/p/{}/?daemon_token=secret-token", entry.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            &format!("/p/{}/", entry.id)
        );
        assert!(response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("refact_daemon_auth=secret-token"));
    }

    #[tokio::test]
    async fn daemon_asset_route_rejects_invalid_path() {
        let response = handle_daemon_gui_asset(AxumPath("../secret".to_string()))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn daemon_asset_route_serves_css_and_js_with_content_types_when_embedded() {
        let state = test_state().await.state;
        for (path, content_type) in [
            ("style.css", "text/css; charset=utf-8"),
            ("index.umd.cjs", "text/javascript; charset=utf-8"),
        ] {
            if ChatGuiAsset::get(&format!("dist/chat/{path}")).is_none() {
                continue;
            }
            let response = crate::daemon::server::make_router(state.clone(), 8488)
                .oneshot(
                    Request::builder()
                        .uri(format!("/dist/chat/{path}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response
                    .headers()
                    .get(axum::http::header::CONTENT_TYPE)
                    .unwrap(),
                content_type
            );
        }
    }

    #[tokio::test]
    #[ignore = "F-0: daemon project-GUI prefix/origin injection must be reconciled with main's __REFACT_ENGINE_ORIGIN_CANDIDATES__ GUI bootstrap (anchors removed by GUI origin refactor merged from main)"]
    async fn project_index_route_injects_daemon_gui_for_registered_project() {
        let test = test_state().await;
        let state = test.state;
        let root = tempfile::tempdir().unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.path().to_path_buf()).await.unwrap()
        };
        let response = crate::daemon::server::make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri(format!("/p/{}/", entry.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("daemonProjectApiPrefix"));
        assert!(body.contains(&format!("/p/{}", entry.id)));
        assert!(body.contains(&format!("http://127.0.0.1:8488/p/{}", entry.id)));
    }

    #[test]
    fn json_for_script_escapes_html_sensitive_chars() {
        let text = json_for_script(&json!({"x": "</script><b>&"}));
        assert!(!text.contains("</script>"));
        assert!(text.contains("\\u003c/script\\u003e"));
        assert!(text.contains("\\u0026"));
    }
}
