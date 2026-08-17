use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{Page, Runtime};
use hyper::body::Bytes;
use refact_core::image_policy::ImagePolicy;
use refact_lsp::call_validation::ChatContent;
use refact_lsp::chat::browser_context::maybe_insert_browser_context;
use refact_lsp::integrations::browser_controller::execute_steps as execute_fixture_steps_with_policy;
use refact_lsp::integrations::browser_controller::execute_steps as execute_steps_with_policy;
use refact_lsp::integrations::browser_controller::execute_request_with_runtime;
use refact_lsp::integrations::browser_controller::execute_steps_with_runtime;
use refact_lsp::integrations::browser_models::{
    AccessibilitySnapshotOptions, BrowserActionRequest, BrowserCookie, BrowserCookieSameSite,
    BrowserExpectedText, BrowserExpectation, BrowserLoadState, BrowserLocator, BrowserPdfOptions,
    BrowserScreenshotAnimations, BrowserScreenshotClip, BrowserScreenshotOptions, BrowserStep,
    BrowserStorageItem, BrowserStorageKind, FillStrategy, LocatorHandlerAction, LocatorRegex,
    RouteHandler, SessionPolicy, TabTarget, UrlPattern, WebSocketEventKind, WebSocketRouteMode,
};
use refact_lsp::refact_browser::{
    BrowserRuntime, CdpKeyboardDispatcher, CdpMouseDispatcher, CheckedState, HandleError, Keyboard,
    HitTargetController, HitTargetResult, Mouse, MouseButton, UTILITY_WORLD_NAME,
};
use serde::Deserialize;
use serde_json::json;
use structopt::StructOpt;
use tempfile::{tempdir, TempDir};

fn execute_fixture_steps(
    tab: &Tab,
    steps: &[BrowserStep],
) -> refact_lsp::integrations::browser_models::ExecutionReport {
    execute_fixture_steps_with_policy(tab, steps, &ImagePolicy::browser_capture())
}

const FIXTURE_PAGES: &[&str] = &[
    "delayed-button.html",
    "overlay.html",
    "moving-target.html",
    "states.html",
    "roles.html",
    "accname.html",
    "snapshot.html",
    "controlled-input.html",
    "form-actions.html",
    "iframe-form.html",
    "nested-iframe.html",
    "nested-iframe-outer.html",
    "nested-iframe-inner.html",
    "shadow-dom.html",
    "dialog.html",
    "fetch-after-click.html",
    "slow-network.html",
    "popup.html",
    "route-target.html",
    "context-probe.html",
    "upload.html",
    "download.html",
    "contenteditable.html",
    "hover-menu.html",
    "strict-multi.html",
    "compose.html",
    "hostile-globals.html",
    "hit-target.html",
    "selectors.html",
    "cookie-banner.html",
    "interstitial.html",
    "generator.html",
    "ws-echo.html",
    "har-target.html",
];

fn execute_steps(
    tab: &Tab,
    steps: &[BrowserStep],
) -> refact_lsp::integrations::browser_models::ExecutionReport {
    execute_steps_with_policy(tab, steps, &ImagePolicy::default())
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/browser_fixtures")
}

fn find_executable(name: &FsPath, path: Option<&OsString>) -> Option<PathBuf> {
    if name.components().count() > 1 || name.is_absolute() {
        return name.is_file().then(|| name.to_path_buf());
    }
    path.and_then(|value| {
        std::env::split_paths(value)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn discover_chrome_with(
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

fn discover_chrome() -> Option<PathBuf> {
    discover_chrome_with(std::env::var_os("CHROME"), std::env::var_os("PATH"))
}

fn e2e_enabled() -> bool {
    std::env::var("REFACT_BROWSER_E2E").as_deref() == Ok("1") && discover_chrome().is_some()
}

fn print_skip() {
    eprintln!(
        "skipped: set REFACT_BROWSER_E2E=1 and install Chrome, Chromium, google-chrome, or chromium-browser"
    );
}

#[derive(Deserialize)]
struct SlowEchoQuery {
    ms: Option<u64>,
}

async fn slow_echo(Query(query): Query<SlowEchoQuery>) -> impl IntoResponse {
    let delay_ms = query.ms.unwrap_or(0).min(5_000);
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    axum::Json(json!({"echo": "ok", "delay_ms": delay_ms}))
}

async fn download() -> Response<Body> {
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

async fn upload(headers: HeaderMap, body: Bytes) -> Response<Body> {
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

async fn route_data(headers: HeaderMap) -> impl IntoResponse {
    let source = headers
        .get("x-route-test")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("origin");
    axum::Json(json!({"source": source}))
}

async fn route_redirect() -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/api/data")
        .body(Body::empty())
        .unwrap()
}

fn content_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn static_fixture(
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

struct FixtureServer {
    base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start() -> Result<Self, String> {
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

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn launch_browser(profile: &TempDir) -> BrowserRuntime {
    BrowserRuntime::launch(
        profile.path().to_path_buf(),
        None,
        discover_chrome(),
        Some(Duration::from_secs(120)),
        true,
        true,
    )
    .expect("browser launch must succeed after e2e_enabled")
}

struct BrowserCase {
    runtime: BrowserRuntime,
    _profile: TempDir,
    server: FixtureServer,
    tab: Arc<Tab>,
}

impl BrowserCase {
    async fn start(page: &str) -> Option<Self> {
        if !e2e_enabled() {
            print_skip();
            return None;
        }
        let server = FixtureServer::start().await.unwrap();
        let profile = tempdir().unwrap();
        let mut runtime = launch_browser(&profile);
        let tab = runtime.browser.new_tab().unwrap();
        runtime.set_active_tab_target_id(tab.get_target_id().to_string());
        let report = execute_fixture_steps(
            &tab,
            &[BrowserStep::Navigate {
                url: server.url(page),
            }],
        );
        assert!(report.ok, "navigation failed: {report:?}");
        Some(Self {
            runtime,
            _profile: profile,
            server,
            tab,
        })
    }

    fn navigate(&self, page: &str) {
        let report = execute_fixture_steps(
            &self.tab,
            &[BrowserStep::Navigate {
                url: self.server.url(page),
            }],
        );
        assert!(report.ok, "navigation failed: {report:?}");
    }

    fn setup_world(&mut self) {
        refact_lsp::refact_browser::setup_recording_for_tab(&mut self.runtime, self.tab.clone())
            .unwrap();
    }

    fn call_version(&self) -> serde_json::Value {
        self.runtime
            .world_manager
            .call_injected(&self.tab, "version", json!([]))
            .unwrap()
    }
}

#[tokio::test]
async fn context_state_roundtrips_and_reaches_adopted_popup() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let server = FixtureServer::start().await.unwrap();
    let profile = tempdir().unwrap();
    let mut browser = launch_browser(&profile);
    let tab = browser.browser.new_tab().unwrap();
    browser.set_active_tab_target_id(tab.get_target_id().to_string());
    let mut case = BrowserCase {
        runtime: browser,
        _profile: profile,
        server,
        tab,
    };
    case.tab
        .call_method(Page::Navigate {
            url: case.server.url("context-probe.html"),
            referrer: None,
            transition_Type: None,
            frame_id: None,
            referrer_policy: None,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::SetViewport {
                    width: 412,
                    height: 732,
                    device_scale_factor: Some(2.0),
                    is_mobile: Some(true),
                    has_touch: Some(true),
                },
                BrowserStep::EmulateMedia {
                    color_scheme: Some("dark".to_string()),
                    reduced_motion: None,
                    forced_colors: None,
                    contrast: None,
                    media: None,
                },
                BrowserStep::SetLocale {
                    locale: "ja-JP".to_string(),
                },
                BrowserStep::SetTimezone {
                    timezone: "Asia/Tokyo".to_string(),
                },
                BrowserStep::SetCookies {
                    cookies: vec![BrowserCookie {
                        name: "session".to_string(),
                        value: "logged-in".to_string(),
                        domain: String::new(),
                        path: "/".to_string(),
                        expires: None,
                        http_only: false,
                        secure: false,
                        same_site: Some(BrowserCookieSameSite::Lax),
                        url: Some(case.server.url("context-probe.html")),
                    }],
                },
                BrowserStep::SetStorage {
                    kind: BrowserStorageKind::Local,
                    items: vec![BrowserStorageItem {
                        name: "logged_in".to_string(),
                        value: "true".to_string(),
                    }],
                },
                BrowserStep::Reload,
                BrowserStep::WaitForPopup {
                    timeout_ms: Some(5_000),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#popup"),
                },
                BrowserStep::Eval {
                    expression: "({language:navigator.language,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,dark:matchMedia('(prefers-color-scheme: dark)').matches,width:innerWidth,height:innerHeight,cookie:document.cookie,localStorage:Object.fromEntries(Object.entries(localStorage))})".to_string(),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(report.ok, "context steps failed: {report:?}");
    assert_eq!(report.new_tabs.len(), 1);
    let probe = report.steps.last().unwrap().data.as_ref().unwrap();
    assert_eq!(probe["width"], 412);
    assert_eq!(probe["dark"], true);
    assert_eq!(probe["language"], "ja-JP");
    assert_eq!(probe["timezone"], "Asia/Tokyo");
    assert_eq!(probe["localStorage"]["logged_in"], "true");
    assert!(probe["cookie"]
        .as_str()
        .unwrap()
        .contains("session=logged-in"));

    let state = {
        let runtime = runtime.lock().await;
        let tab = runtime.get_active_tab().unwrap();
        refact_lsp::refact_browser::context_state::storage_state(&tab).unwrap()
    };
    assert_eq!(
        state
            .cookies
            .iter()
            .find(|cookie| cookie.name == "session")
            .unwrap()
            .value,
        "logged-in"
    );
    assert_eq!(state.origins[0].local_storage[0].value, "true");

    let fresh_profile = tempdir().unwrap();
    let mut fresh_runtime = launch_browser(&fresh_profile);
    let fresh_tab = fresh_runtime.browser.new_tab().unwrap();
    fresh_runtime.set_active_tab_target_id(fresh_tab.get_target_id().to_string());
    refact_lsp::refact_browser::setup_recording_for_tab(&mut fresh_runtime, fresh_tab.clone())
        .unwrap();
    refact_lsp::refact_browser::context_state::set_storage_state(&fresh_tab, &state).unwrap();
    fresh_tab
        .navigate_to(&case.server.url("context-probe.html"))
        .and_then(|tab| tab.wait_until_navigated())
        .unwrap();
    let restored = fresh_tab
        .evaluate(
            "({cookie:document.cookie,loggedIn:localStorage.getItem('logged_in')})",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert!(restored["cookie"]
        .as_str()
        .unwrap()
        .contains("session=logged-in"));
    assert_eq!(restored["loggedIn"], "true");
}

fn text_step(selector: &str) -> BrowserStep {
    BrowserStep::GetText {
        locator: BrowserLocator::css(selector),
    }
}

fn returned_text(report: &refact_lsp::integrations::browser_models::ExecutionReport) -> &str {
    report
        .steps
        .last()
        .and_then(|step| step.data.as_ref())
        .and_then(|data| data.get("text"))
        .and_then(|text| text.as_str())
        .unwrap_or("")
}

fn returned_eval_string(
    report: &refact_lsp::integrations::browser_models::ExecutionReport,
) -> &str {
    report
        .steps
        .last()
        .and_then(|step| step.data.as_ref())
        .and_then(|data| data.get("value"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
}

#[tokio::test]
async fn fixture_server_starts_and_serves_page() {
    let server = FixtureServer::start().await.unwrap();
    let response = reqwest::get(server.url("delayed-button.html"))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
    assert!(response.text().await.unwrap().contains("Delayed button"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn transactional_report_settles_fetch_and_returns_console_once() {
    let Some(mut case) = BrowserCase::start("fetch-after-click.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let request = BrowserActionRequest {
        session: SessionPolicy::SharedDefault,
        target: TabTarget::Active,
        attach_screenshot: None,
        steps: vec![BrowserStep::Eval {
            expression: "document.querySelector('#fetch').click()".to_string(),
        }],
    };

    let report =
        execute_request_with_runtime(runtime.clone(), request, &ImagePolicy::browser_capture())
            .await
            .unwrap();
    assert!(report.ok, "click failed: {report:?}");
    assert!(report.stabilized);
    assert!(report
        .url
        .as_deref()
        .is_some_and(|url| url.ends_with("fetch-after-click.html")));
    assert!(report
        .console
        .iter()
        .any(|entry| entry.text.contains("slow echo settled after 400ms")));
    assert!(report.network.iter().any(|entry| {
        entry.url.contains("/slow-echo")
            && entry.status == Some(200)
            && entry.failure_text.is_none()
    }));
    assert_eq!(
        execute_request_with_runtime(
            runtime,
            BrowserActionRequest {
                session: SessionPolicy::SharedDefault,
                target: TabTarget::Active,
                attach_screenshot: None,
                steps: vec![],
            },
            &ImagePolicy::browser_capture(),
        )
        .await
        .unwrap()
        .console
        .iter()
        .filter(|entry| entry.text.contains("slow echo settled after 400ms"))
        .count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn network_waits_coexist_with_locator_handlers_and_dialogs_in_one_batch() {
    let Some(mut case) = BrowserCase::start("slow-network.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::RemoveLocatorHandler {
                    name: "dismiss_overlays".to_string(),
                },
                BrowserStep::AddLocatorHandler {
                    name: "network-overlay".to_string(),
                    locator: BrowserLocator::css("#accept-all"),
                    handler: LocatorHandlerAction::Click,
                    times: Some(1),
                    no_wait_after: false,
                },
                BrowserStep::HandleDialog {
                    accept: false,
                    prompt_text: None,
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#load"),
                },
                BrowserStep::WaitForResponse {
                    url_or_pattern: UrlPattern::Regex {
                        source: "/missing-network-resource$".to_string(),
                        flags: String::new(),
                    },
                    timeout_ms: Some(3_000),
                },
                BrowserStep::WaitForLoadState {
                    state: BrowserLoadState::Networkidle,
                    timeout_ms: Some(3_000),
                },
                BrowserStep::GetText {
                    locator: BrowserLocator::css("#result"),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "mixed browser batch failed: {report:?}");
    assert!(report.steps[4].summary.contains("Matched response"));
    let waited_response = report.steps[4].data.as_ref().unwrap();
    assert!(waited_response["url"]
        .as_str()
        .unwrap()
        .ends_with("/missing-network-resource"));
    assert_eq!(waited_response["status"], 404);
    assert!(report.steps[5].summary.contains("networkidle"));
    assert_eq!(returned_text(&report), "echo ok after 700ms:404");
    assert!(report
        .locator_handlers
        .iter()
        .any(|firing| firing.name == "network-overlay" && firing.ok));
    assert_eq!(report.dialogs.len(), 1);
    assert!(!report.dialogs[0].automatic);
    assert!(report
        .network
        .iter()
        .any(|entry| { entry.url.contains("/slow-echo") && entry.status == Some(200) }));
    assert!(report.network.iter().any(|entry| {
        entry.url.contains("/missing-network-resource") && entry.status == Some(404)
    }));
}

#[test]
fn chrome_discovery_returns_none_for_missing_candidates() {
    let empty_path = tempdir().unwrap();
    let found = discover_chrome_with(
        Some(OsString::from("missing-browser")),
        Some(empty_path.path().as_os_str().to_os_string()),
    );
    assert_eq!(found, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fixture_server_serves_every_page_in_browser() {
    let Some(case) = BrowserCase::start(FIXTURE_PAGES[0]).await else {
        return;
    };
    for page in &FIXTURE_PAGES[1..] {
        case.navigate(page);
        assert!(case.tab.get_url().ends_with(page));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn navigation_steps_wait_for_reload_history_and_open_tab() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    let initial_url = case.server.url("snapshot.html");
    let next_url = case.server.url("delayed-button.html");
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Reload,
            BrowserStep::Eval {
                expression: "history.pushState({}, '', '#same-document'); location.href"
                    .to_string(),
            },
            BrowserStep::GoBack,
            BrowserStep::GoForward,
            BrowserStep::Navigate {
                url: next_url.clone(),
            },
            BrowserStep::GoBack,
            BrowserStep::GoForward,
            BrowserStep::OpenTab {
                device: None,
                url: Some(initial_url.clone()),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "navigation steps failed: {report:?}");
    assert_eq!(report.steps.len(), 8);
    assert_eq!(report.steps[2].summary, "Navigated back");
    assert_eq!(report.steps[3].summary, "Navigated forward");
    assert_eq!(report.steps[5].summary, "Navigated back");
    assert_eq!(report.steps[6].summary, "Navigated forward");
    assert_eq!(report.url.as_deref(), Some(initial_url.as_str()));
    assert_eq!(report.new_tabs.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn navigated_fixture_context_contains_screenshot_image() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    const CHAT_ID: &str = "browser-context-screenshot-e2e";
    let cache_dir = tempdir().unwrap();
    let config_dir = tempdir().unwrap();
    let command_line = refact_lsp::global_context::CommandLine::from_iter_safe([
        "browser-e2e",
        "--http-port",
        "0",
        "--lsp-port",
        "0",
        "--no-scheduler",
    ])
    .unwrap();
    let (gcx, _) = refact_lsp::global_context::create_global_context(
        cache_dir.path().to_path_buf(),
        config_dir.path().to_path_buf(),
        command_line,
    )
    .await;
    let app = refact_lsp::app_state::AppState::from_gcx(gcx.clone()).await;
    case.runtime.reattach(CHAT_ID);
    refact_lsp::integrations::browser_runtime::register_browser_runtime(app, case.runtime).await;

    let (context, oversize) = maybe_insert_browser_context(gcx, CHAT_ID, true, true)
        .await
        .expect("navigated page must produce browser context");

    assert!(!oversize);
    assert_eq!(
        context.event.extra["event"]["payload"]["page_changed"],
        true
    );
    let ChatContent::Multimodal(elements) = context
        .screenshot
        .expect("enabled page change must attach a screenshot")
        .content
    else {
        panic!("expected screenshot context");
    };
    assert!(elements.iter().any(|element| element.is_image()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn artifacts_capture_page_clip_element_pdf_and_highlight_lifecycle() {
    let Some(mut case) = BrowserCase::start("moving-target.html").await else {
        return;
    };
    case.tab
        .evaluate("document.body.style.minHeight = '1800px'", false)
        .unwrap();
    case.setup_world();
    let snapshot = case
        .runtime
        .world_manager
        .aria_snapshot(
            &case.tab,
            None,
            refact_lsp::refact_browser::SnapshotOptions {
                mode: refact_lsp::refact_browser::SnapshotMode::Ai,
                refs: true,
                ..Default::default()
            },
        )
        .unwrap();
    let moving_ref = snapshot
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Moving button"))
        .and_then(|node| node.reference.clone())
        .unwrap();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Screenshot {
                    options: BrowserScreenshotOptions {
                        full_page: true,
                        animations: Some(BrowserScreenshotAnimations::Disabled),
                        ..Default::default()
                    },
                },
                BrowserStep::Screenshot {
                    options: BrowserScreenshotOptions {
                        clip: Some(BrowserScreenshotClip {
                            x: 0.0,
                            y: 0.0,
                            width: 120.0,
                            height: 80.0,
                        }),
                        ..Default::default()
                    },
                },
                BrowserStep::ScreenshotElement {
                    locator: BrowserLocator::reference(&moving_ref),
                    options: BrowserScreenshotOptions::default(),
                },
                BrowserStep::Screenshot {
                    options: BrowserScreenshotOptions {
                        mask: vec![BrowserLocator::css("#moving")],
                        mask_color: Some("#ff00ff".to_string()),
                        ..Default::default()
                    },
                },
                BrowserStep::Screenshot {
                    options: BrowserScreenshotOptions {
                        clip: Some(BrowserScreenshotClip {
                            x: 500.0,
                            y: 500.0,
                            width: 40.0,
                            height: 40.0,
                        }),
                        omit_background: true,
                        ..Default::default()
                    },
                },
                BrowserStep::Screenshot {
                    options: BrowserScreenshotOptions {
                        animations: Some(BrowserScreenshotAnimations::Disabled),
                        ..Default::default()
                    },
                },
                BrowserStep::Screenshot {
                    options: BrowserScreenshotOptions {
                        animations: Some(BrowserScreenshotAnimations::Disabled),
                        ..Default::default()
                    },
                },
                BrowserStep::Highlight {
                    locator: BrowserLocator::css("#moving"),
                    style: None,
                    label: Some("Target".to_string()),
                },
                BrowserStep::Annotate {
                    locator: BrowserLocator::css("#moving"),
                    text: "Review".to_string(),
                },
                BrowserStep::HideHighlight,
                BrowserStep::Pdf {
                    options: BrowserPdfOptions::default(),
                },
            ],
        },
        &ImagePolicy::default(),
    )
    .await
    .unwrap();

    assert!(report.ok, "artifact batch failed: {report:?}");
    let image_at = |step: usize| {
        let data = report.steps[step].data.as_ref().unwrap();
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(data["artifact"]["data"].as_str().unwrap())
            .unwrap();
        image::load_from_memory(&bytes).unwrap()
    };
    let full = image_at(0);
    let clip = image_at(1);
    let element = image_at(2);
    assert!(full.height() > clip.height());
    assert!(full.width().max(full.height()) <= ImagePolicy::default().preferred_side);
    assert!(clip.width() <= 120 && clip.height() <= 80);
    assert!(element.width() > 0 && element.height() > 0);
    assert!(image_at(3)
        .to_rgba8()
        .pixels()
        .any(|pixel| pixel.0[0] > 240 && pixel.0[1] < 20 && pixel.0[2] > 240));
    assert!(image_at(4).to_rgba8().pixels().any(|pixel| pixel.0[3] == 0));
    assert_eq!(
        report.steps[5].data.as_ref().unwrap()["artifact"]["data"],
        report.steps[6].data.as_ref().unwrap()["artifact"]["data"]
    );
    let pdf = report.steps[10].data.as_ref().unwrap();
    assert!(pdf["artifact"]["bytes"].as_u64().unwrap() > 0);
    assert!(FsPath::new(pdf["artifact"]["path"].as_str().unwrap()).is_file());
    assert_eq!(
        case.tab
            .evaluate(
                "document.querySelector('[data-refact-highlight]') === null",
                false
            )
            .unwrap()
            .value,
        Some(json!(true))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn design_tools_measure_marks_contrast_and_visual_changes() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.querySelector('nav').id='probe-target';document.querySelector('nav').style.cssText='width:50vw;overflow:hidden;color:#777;background:#888';true",
            false,
        )
        .unwrap();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::SetViewport {
                width: 375,
                height: 800,
                device_scale_factor: Some(1.0),
                is_mobile: Some(false),
                has_touch: Some(false),
            },
            BrowserStep::EmulateMedia {
                color_scheme: Some("dark".to_string()),
                reduced_motion: None,
                forced_colors: None,
                contrast: None,
                media: None,
            },
            BrowserStep::Styles {
                locator: BrowserLocator::css("#probe-target"),
                property_filter: Some("color|width|overflow".to_string()),
            },
            BrowserStep::AccessibilitySnapshot {
                options: AccessibilitySnapshotOptions {
                    mode: Default::default(),
                    refs: Some(true),
                    boxes: true,
                    root: None,
                    max_chars: None,
                },
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "design probe failed: {report:?}");
    let nodes = report.steps[3].data.as_ref().unwrap()["nodes"]
        .as_array()
        .unwrap();
    assert!(nodes.iter().any(|node| {
        node["ref"].as_str().is_some()
            && node["box"]["width"].as_i64().is_some_and(|width| width > 0)
    }));

    let contrast = case
        .tab
        .evaluate(
            "(()=>{const s=getComputedStyle(document.querySelector('#probe-target'));return s.color!==s.backgroundColor})()",
            false,
        )
        .unwrap();
    assert_eq!(contrast.value, Some(json!(true)));

    let baseline = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Screenshot {
            options: Default::default(),
        }],
    );
    case.tab
        .evaluate(
            "document.querySelector('#probe-target').style.background='magenta';true",
            false,
        )
        .unwrap();
    let changed = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Screenshot {
            options: Default::default(),
        }],
    );
    assert_ne!(
        baseline.steps[0].data.as_ref().unwrap()["data"],
        changed.steps[0].data.as_ref().unwrap()["data"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn world_utility_survives_hostile_globals() {
    let Some(mut case) = BrowserCase::start("hostile-globals.html").await else {
        return;
    };
    case.setup_world();
    assert_eq!(case.call_version(), "playwright-1.63.0-next-refact-1");
    let main_world = case
        .tab
        .evaluate(
            "[Array.from(), JSON.stringify({}), document.elementFromPoint(0, 0)].join('|')",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(main_world, "hostile-array|hostile-json|hostile-element");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn world_utility_reinjects_after_navigation() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    case.setup_world();
    assert_eq!(case.call_version(), "playwright-1.63.0-next-refact-1");
    case.navigate("hostile-globals.html");
    assert_eq!(case.call_version(), "playwright-1.63.0-next-refact-1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn world_stealth_stays_in_main_world() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    case.setup_world();
    assert_eq!(
        case.tab
            .evaluate(
                "globalThis.__refact_stealth_installed === true && navigator.webdriver === undefined",
                false,
            )
            .unwrap()
            .value,
        Some(json!(true))
    );
    let frame_id = case
        .tab
        .call_method(Page::GetFrameTree(None))
        .unwrap()
        .frame_tree
        .frame
        .id;
    let context_id = case
        .tab
        .call_method(Page::CreateIsolatedWorld {
            frame_id,
            world_name: Some(UTILITY_WORLD_NAME.to_string()),
            grant_univeral_access: Some(true),
        })
        .unwrap()
        .execution_context_id;
    let isolated = case
        .tab
        .call_method(Runtime::Evaluate {
            expression: "globalThis.__refact_stealth_installed".to_string(),
            object_group: None,
            include_command_line_api: None,
            silent: None,
            context_id: Some(context_id),
            return_by_value: Some(true),
            generate_preview: None,
            user_gesture: None,
            await_promise: None,
            throw_on_side_effect: None,
            timeout: None,
            disable_breaks: None,
            repl_mode: None,
            allow_unsafe_eval_blocked_by_csp: None,
            unique_context_id: None,
            serialization_options: None,
        })
        .unwrap();
    assert_eq!(isolated.result.value, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn click_delayed_button_without_wait_seconds() {
    let Some(case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#delayed"),
            },
            BrowserStep::WaitForText {
                text: "delayed clicked".to_string(),
                timeout_ms: Some(3_000),
            },
        ],
    );
    assert!(report.ok, "click should auto-wait: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn expect_retries_matchers_report_received_and_soft_failure_continues() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.body.insertAdjacentHTML('beforeend', '<p id=assertion-text>Alpha   Beta 42</p><ul><li class=assertion-item>One</li><li class=assertion-item>Two</li><li class=assertion-item>Three</li></ul>')",
            false,
        )
        .unwrap();
    case.tab
        .evaluate(
            "const delayed = document.querySelector('#delayed'); delayed.style.display = 'none'; setTimeout(() => delayed.style.display = 'inline-block', 1500)",
            false,
        )
        .unwrap();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#delayed")),
                matcher: BrowserExpectation::ToBeVisible,
                timeout_ms: Some(3_000),
                soft: false,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#assertion-text")),
                matcher: BrowserExpectation::ToContainText {
                    expected: BrowserExpectedText::Text("beta 42".to_string()),
                    ignore_case: true,
                },
                timeout_ms: None,
                soft: false,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#assertion-text")),
                matcher: BrowserExpectation::ToHaveText {
                    expected: BrowserExpectedText::Regex(LocatorRegex {
                        source: r"Alpha\s+Beta\s+\d+".to_string(),
                        flags: String::new(),
                    }),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css(".assertion-item")),
                matcher: BrowserExpectation::ToHaveCount { expected: 3 },
                timeout_ms: None,
                soft: false,
            },
            BrowserStep::Navigate {
                url: case.server.url("snapshot.html"),
            },
            BrowserStep::Expect {
                locator: None,
                matcher: BrowserExpectation::ToHaveUrl {
                    expected: BrowserExpectedText::Regex(LocatorRegex {
                        source: r"/snapshot\.html$".to_string(),
                        flags: String::new(),
                    }),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::role("navigation", Some("Primary"))),
                matcher: BrowserExpectation::ToMatchAriaSnapshot {
                    expected: "- navigation:\n  - link\n  - button \"Save\"".to_string(),
                },
                timeout_ms: None,
                soft: false,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("h1")),
                matcher: BrowserExpectation::ToHaveText {
                    expected: BrowserExpectedText::Text("Missing heading".to_string()),
                    ignore_case: false,
                },
                timeout_ms: Some(50),
                soft: true,
            },
            BrowserStep::Expect {
                locator: None,
                matcher: BrowserExpectation::ToHaveTitle {
                    expected: BrowserExpectedText::Text("ARIA snapshot fixture".to_string()),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(
        report.ok,
        "soft assertion should not fail the batch: {report:?}"
    );
    assert_eq!(report.steps.len(), 9);
    assert!(report.steps[0].retries > 0);
    let failed = report.steps[7].assertion.as_ref().unwrap();
    assert!(!failed.passed);
    assert!(failed.soft);
    assert_eq!(failed.expected, json!("Missing heading"));
    assert_eq!(failed.received, json!("Snapshot page"));
    assert!(report.steps[8].ok, "batch did not continue: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn click_obscured_waits_for_overlay() {
    let Some(mut case) = BrowserCase::start("overlay.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "const overlay = document.createElement('div'); overlay.id = 'overlay'; overlay.textContent = 'blocking overlay'; overlay.style.cssText = 'position:fixed;inset:0;z-index:10'; document.body.append(overlay); setTimeout(() => overlay.remove(), 2500)",
            false,
        )
        .unwrap();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#target"),
            },
            BrowserStep::WaitForText {
                text: "clicked after overlay".to_string(),
                timeout_ms: Some(2_000),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(
        report.ok,
        "click should wait for overlay removal: {report:?}"
    );
    let diagnostics = report.steps[0]
        .actionability
        .as_ref()
        .expect("retried click should include actionability diagnostics");
    assert!(diagnostics.attempts.unwrap_or_default() > 0);
    assert_eq!(diagnostics.receives_events, Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn click_permanently_obscured_reports_intercepting_element() {
    let Some(mut case) = BrowserCase::start("overlay.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "const overlay = document.createElement('div'); overlay.id = 'permanent-overlay'; overlay.textContent = 'blocking overlay'; overlay.style.cssText = 'position:fixed;inset:0;z-index:10'; document.body.append(overlay)",
            false,
        )
        .unwrap();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Click {
            locator: BrowserLocator::css("#target"),
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(!report.ok, "obscured click should time out: {report:?}");
    let diagnostics = report.steps[0]
        .actionability
        .as_ref()
        .expect("failed click should include actionability diagnostics");
    assert!(diagnostics.timed_out);
    assert_eq!(diagnostics.receives_events, Some(false));
    assert!(diagnostics
        .intercepting_element
        .as_deref()
        .is_some_and(|preview| preview.contains("permanent-overlay")));
    assert!(diagnostics
        .call_log
        .last()
        .is_some_and(|entry| entry.contains("intercepts pointer events")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fill_controlled_input_react() {
    let Some(case) = BrowserCase::start("controlled-input.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Fill {
            locator: BrowserLocator::css("#controlled"),
            text: "typed by browser".to_string(),
            clear_first: true,
            verify: true,
        }],
    );
    assert!(
        report.ok,
        "controlled fill should use trusted input: {report:?}"
    );
    let output = case
        .tab
        .evaluate("document.querySelector('#state').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(output, "typed by browser");
    assert_eq!(
        report.steps[0].fill_strategy,
        Some(FillStrategy::NativeTyping)
    );
    assert_eq!(report.steps[0].verified, Some(true));
    assert_eq!(report.steps[0].retries, 0);
    let diagnostics = report.steps[0]
        .actionability
        .as_ref()
        .expect("controlled fill should include actionability diagnostics");
    assert_eq!(diagnostics.visible, Some(true));
    assert_eq!(diagnostics.enabled, Some(true));
    assert_eq!(diagnostics.editable, Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fill_falls_back_when_cdp_input_is_rejected() {
    let Some(case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Fill {
            locator: BrowserLocator::css("#fallback"),
            text: "fallback value".to_string(),
            clear_first: true,
            verify: true,
        }],
    );
    assert!(report.ok, "fallback fill failed: {report:?}");
    assert_ne!(
        report.steps[0].fill_strategy,
        Some(FillStrategy::NativeTyping)
    );
    assert_eq!(report.steps[0].verified, Some(true));
    assert!(report.steps[0].retries > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn disabled_fill_reports_enabled_actionability_state() {
    let Some(case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Fill {
            locator: BrowserLocator::css("#disabled"),
            text: "blocked".to_string(),
            clear_first: true,
            verify: true,
        }],
    );

    assert!(!report.ok, "disabled fill should fail: {report:?}");
    let diagnostics = report.steps[0]
        .actionability
        .as_ref()
        .expect("disabled fill should include actionability diagnostics");
    assert!(diagnostics.timed_out);
    assert_eq!(diagnostics.enabled, Some(false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn checkbox_actions_are_verified_and_idempotent() {
    let Some(case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    let locator = BrowserLocator::css("#checkbox");
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Check {
                locator: locator.clone(),
            },
            BrowserStep::Check {
                locator: locator.clone(),
            },
            BrowserStep::Uncheck {
                locator: locator.clone(),
            },
            BrowserStep::Uncheck { locator },
        ],
    );
    assert!(report.ok, "checkbox actions failed: {report:?}");
    let changed = report
        .steps
        .iter()
        .map(|step| step.data.as_ref().unwrap()["changed"].as_bool().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(changed, vec![true, false, true, false]);
    assert!(report
        .steps
        .iter()
        .all(|step| step.data.as_ref().unwrap()["verified"] == true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn radio_uncheck_returns_playwright_error() {
    let Some(case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Uncheck {
            locator: BrowserLocator::css("#radio"),
        }],
    );
    assert!(!report.ok);
    assert!(report.steps[0]
        .error
        .as_deref()
        .unwrap()
        .contains("Cannot uncheck radio button"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn select_option_selects_from_multiple_select() {
    let Some(case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::SelectOption {
            locator: BrowserLocator::css("#multiple"),
            value: "Beta option".to_string(),
        }],
    );
    assert!(report.ok, "select option failed: {report:?}");
    assert_eq!(
        report.steps[0].data.as_ref().unwrap()["selected"],
        json!(["beta"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn keyboard_types_unicode_into_controlled_input() {
    let Some(case) = BrowserCase::start("controlled-input.html").await else {
        return;
    };
    case.tab
        .evaluate("document.querySelector('#controlled').focus()", false)
        .unwrap();
    let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(&case.tab));
    keyboard.type_text("hello é🙂", None).unwrap();
    let value = case
        .tab
        .evaluate("document.querySelector('#state').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(value, "hello é🙂");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn keyboard_shortcut_selects_and_deletes_controlled_input() {
    let Some(case) = BrowserCase::start("controlled-input.html").await else {
        return;
    };
    case.tab
        .evaluate("document.querySelector('#controlled').focus()", false)
        .unwrap();
    let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(&case.tab));
    keyboard.insert_text("remove me").unwrap();
    keyboard.press("Control+A", None).unwrap();
    keyboard.press("Delete", None).unwrap();
    let value = case
        .tab
        .evaluate("document.querySelector('#state').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(value, "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn hover_reveals_css_menu() {
    let Some(case) = BrowserCase::start("hover-menu.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Hover {
                locator: BrowserLocator::css("#trigger"),
            },
            BrowserStep::WaitForSelector {
                locator: BrowserLocator::css("#menu a"),
                timeout_ms: Some(2_000),
            },
        ],
    );
    assert!(report.ok, "hover should use real pointer input: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn cdp_mouse_hover_reveals_css_only_menu() {
    let Some(mut case) = BrowserCase::start("hover-menu.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#trigger"}]),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let point = CdpMouseDispatcher::new(&case.tab)
        .clickable_point(&handle)
        .unwrap();
    let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(&case.tab));
    let mut mouse = Mouse::new(CdpMouseDispatcher::new(&case.tab), &keyboard);
    mouse.hover(point.x, point.y).unwrap();
    let display = case
        .tab
        .evaluate(
            "getComputedStyle(document.querySelector('#menu')).display",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(display, "block");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn hit_target_preliminary_check_names_covering_overlay() {
    let Some(mut case) = BrowserCase::start("hit-target.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#target"}]),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let point = CdpMouseDispatcher::new(&case.tab)
        .clickable_point(&handle)
        .unwrap();
    case.tab.evaluate("window.addOverlay()", false).unwrap();
    let result = HitTargetController::default()
        .expect_hit_target(
            &case.tab,
            &case.runtime.world_manager,
            &handle,
            refact_lsp::refact_browser::HitTargetPoint {
                x: point.x,
                y: point.y,
            },
        )
        .unwrap();
    let HitTargetResult::Intercepted { description } = result else {
        panic!("expected overlay interception, got {result:?}");
    };
    assert!(description.contains("<div class=\"overlay\"></div>"));
    assert!(description.ends_with("intercepts pointer events"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn hit_target_preliminary_check_walks_open_shadow_root() {
    let Some(mut case) = BrowserCase::start("hit-target.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .resolve_expression_handle(
            &case.tab,
            "document.querySelector('#shadow-host').shadowRoot.querySelector('#shadow-target')",
        )
        .unwrap();
    let point = CdpMouseDispatcher::new(&case.tab)
        .clickable_point(&handle)
        .unwrap();
    assert_eq!(
        HitTargetController::default()
            .expect_hit_target(
                &case.tab,
                &case.runtime.world_manager,
                &handle,
                refact_lsp::refact_browser::HitTargetPoint {
                    x: point.x,
                    y: point.y,
                },
            )
            .unwrap(),
        HitTargetResult::Done
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn hit_target_interceptor_suppresses_overlay_appearing_after_precheck() {
    let Some(mut case) = BrowserCase::start("hit-target.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#target"}]),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let point = CdpMouseDispatcher::new(&case.tab)
        .clickable_point(&handle)
        .unwrap();
    let controller = HitTargetController::default();
    let token = controller
        .install_interceptor(
            &case.tab,
            &case.runtime.world_manager,
            &handle,
            refact_lsp::refact_browser::ActionKind::Click,
            Some(refact_lsp::refact_browser::HitTargetPoint {
                x: point.x,
                y: point.y,
            }),
        )
        .unwrap();
    case.tab.evaluate("window.addLateOverlay()", false).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(&case.tab));
    let mut mouse = Mouse::new(CdpMouseDispatcher::new(&case.tab), &keyboard);
    mouse.click(point.x, point.y, MouseButton::Left).unwrap();
    let result = controller
        .take_result(&case.tab, &case.runtime.world_manager, token)
        .unwrap();
    let HitTargetResult::Intercepted { description } = result else {
        panic!("expected event-time overlay interception, got {result:?}");
    };
    assert!(description.contains("class=\"overlay\""));
    let output = case
        .tab
        .evaluate("document.querySelector('#result').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(output, "idle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn strict_multi_click_errors() {
    let Some(case) = BrowserCase::start("strict-multi.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Click {
            locator: BrowserLocator::css(".duplicate"),
        }],
    );
    assert!(
        !report.ok,
        "strict click must reject multiple matches: {report:?}"
    );
    let error = report.steps[0].error.as_deref().unwrap_or_default();
    assert!(error.contains("css=.duplicate"), "{error}");
    assert!(error.contains("resolved to 3 elements"), "{error}");
    assert!(error.contains("<button class=\"duplicate\""), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn wait_for_selector_accepts_multiple_matches() {
    let Some(case) = BrowserCase::start("strict-multi.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::WaitForSelector {
            locator: BrowserLocator::css(".duplicate"),
            timeout_ms: Some(2_000),
        }],
    );
    assert!(
        report.ok,
        "wait for selector must accept multiple matches: {report:?}"
    );
    assert!(
        report.steps[0].summary.contains("3 match(es)"),
        "{:?}",
        report.steps[0]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn click_if_exists_skips_attached_invisible_element_without_retry_budget() {
    let Some(case) = BrowserCase::start("states.html").await else {
        return;
    };
    let prepared = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Eval {
            expression: "document.body.insertAdjacentHTML('beforeend', '<button id=\"invisible-button\" style=\"visibility:hidden;width:60px;height:24px\">Invisible</button>')".to_string(),
        }],
    );
    assert!(prepared.ok, "fixture preparation failed: {prepared:?}");

    let baseline_started = std::time::Instant::now();
    let baseline = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::ClickIfExists {
            locator: BrowserLocator::css("#never-rendered"),
        }],
    );
    let baseline_elapsed = baseline_started.elapsed();
    assert!(baseline.ok, "absent element must skip: {baseline:?}");

    let started = std::time::Instant::now();
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::ClickIfExists {
            locator: BrowserLocator::css("#invisible-button"),
        }],
    );
    let elapsed = started.elapsed();

    assert!(report.ok, "click_if_exists must skip: {report:?}");
    assert!(
        report.steps[0]
            .summary
            .contains("Skipped click_if_exists (css=#invisible-button)"),
        "{:?}",
        report.steps[0]
    );
    assert!(
        report.steps[0].summary.contains("element is not visible"),
        "{:?}",
        report.steps[0]
    );
    assert!(
        elapsed < baseline_elapsed + Duration::from_secs(2),
        "skip burned the retry budget: {elapsed:?} against a {baseline_elapsed:?} baseline"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "skip did not stay below the action retry budget: {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn click_if_exists_skips_missing_element() {
    let Some(case) = BrowserCase::start("states.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::ClickIfExists {
            locator: BrowserLocator::css("#never-rendered"),
        }],
    );
    assert!(report.ok, "missing element must skip: {report:?}");
    assert!(
        report.steps[0].summary.contains("Skipped click_if_exists"),
        "{:?}",
        report.steps[0]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn handle_clicks_second_strict_match() {
    let Some(case) = BrowserCase::start("strict-multi.html").await else {
        return;
    };
    let locator = BrowserLocator {
        strategy: refact_lsp::integrations::browser_models::LocatorStrategy::Css {
            value: ".duplicate".to_string(),
        },
        frames: Vec::new(),
        nth: Some(1),
        within: None,
        locator: None,
        filter: None,
        and: None,
        or: None,
        first: None,
        last: None,
    };
    let report = execute_steps(
        &case.tab,
        &[BrowserStep::Click { locator }, text_step("#result")],
    );
    assert!(report.ok, "nth handle click failed: {report:?}");
    assert_eq!(returned_text(&report), "second");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn handle_is_invalidated_after_navigation() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#delayed"}]),
        )
        .unwrap();
    let handle = handle.into_iter().next().unwrap();
    case.navigate("strict-multi.html");
    let error = case
        .runtime
        .world_manager
        .call_function_on(
            &case.tab,
            &handle,
            "function() { return this.tagName; }",
            vec![],
        )
        .unwrap_err();
    assert!(matches!(error, HandleError::Invalidated { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn element_states_match_playwright_predicates() {
    let Some(mut case) = BrowserCase::start("states.html").await else {
        return;
    };
    case.setup_world();
    let selectors = [
        "#disabled-button",
        "#aria-disabled",
        "#fieldset-disabled",
        "#readonly-input",
        "#aria-readonly",
        "#contenteditable-false",
        "#checked",
        "#unchecked",
        "#mixed",
        "#opacity-zero",
        "#display-none",
        "#readonly-select",
    ];
    let mut states = Vec::new();
    for selector in selectors {
        let handle = case
            .runtime
            .world_manager
            .call_injected_handles(
                &case.tab,
                "resolveAll",
                json!([{"by":"css","value":selector}]),
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        states.push(
            case.runtime
                .world_manager
                .element_states(&case.tab, &handle)
                .unwrap(),
        );
    }

    assert!(!states[0].enabled);
    assert!(!states[1].enabled);
    assert!(!states[2].enabled);
    assert_eq!(states[3].editable, Some(false));
    assert_eq!(states[4].editable, Some(false));
    assert_eq!(states[5].editable, Some(false));
    assert_eq!(states[6].checked, Some(CheckedState::Checked));
    assert_eq!(states[7].checked, Some(CheckedState::Unchecked));
    assert_eq!(states[8].checked, Some(CheckedState::Mixed));
    assert!(states[9].visible);
    assert!(!states[10].visible);
    assert_eq!(states[11].editable, Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn computed_roles_match_playwright_html_aam() {
    let Some(mut case) = BrowserCase::start("roles.html").await else {
        return;
    };
    case.setup_world();
    let handles = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"[data-computed-role]"}]),
        )
        .unwrap();
    assert_eq!(handles.len(), 30);
    for handle in handles {
        let actual = case
            .runtime
            .world_manager
            .call_function_on(
                &case.tab,
                &handle,
                "function() { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return { id: this.id, expectedComputed: this.dataset.computedRole, expectedImplicit: this.dataset.implicitRole || this.dataset.computedRole, computed: instance.computeRole(this), implicit: instance.getImplicitRole(this) }; }",
                Vec::new(),
            )
            .unwrap();
        assert_eq!(
            actual["computed"], actual["expectedComputed"],
            "computed role mismatch for {}",
            actual["id"]
        );
        assert_eq!(
            actual["implicit"], actual["expectedImplicit"],
            "implicit role mismatch for {}",
            actual["id"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn accessible_text_matches_playwright_accname_and_html_aam() {
    let Some(mut case) = BrowserCase::start("accname.html").await else {
        return;
    };
    case.setup_world();
    let handles = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"[data-expected-name]"}]),
        )
        .unwrap();
    assert_eq!(handles.len(), 27);
    for handle in handles {
        let expected = case
            .runtime
            .world_manager
            .call_function_on(
                &case.tab,
                &handle,
                "function() { return { id: this.id, expected: this.dataset.expectedName }; }",
                Vec::new(),
            )
            .unwrap();
        let actual = case
            .runtime
            .world_manager
            .get_accessible_name(&case.tab, &handle, false)
            .unwrap();
        assert_eq!(
            actual, expected["expected"],
            "accessible name mismatch for {}",
            expected["id"]
        );
    }

    let description_handles = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"[data-expected-description]"}]),
        )
        .unwrap();
    for handle in description_handles {
        let expected = case
            .runtime
            .world_manager
            .call_function_on(
                &case.tab,
                &handle,
                "function() { return { id: this.id, expected: this.dataset.expectedDescription }; }",
                Vec::new(),
            )
            .unwrap();
        let actual = case
            .runtime
            .world_manager
            .get_accessible_description(&case.tab, &handle)
            .unwrap();
        assert_eq!(
            actual, expected["expected"],
            "accessible description mismatch for {}",
            expected["id"]
        );
    }

    let hidden = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#hidden-self"}]),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        case.runtime
            .world_manager
            .get_accessible_name(&case.tab, &hidden, false)
            .unwrap(),
        ""
    );
    assert_eq!(
        case.runtime
            .world_manager
            .get_accessible_name(&case.tab, &hidden, true)
            .unwrap(),
        "Hidden self"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn moving_target_stability_predicate_reports_each_probe() {
    let Some(mut case) = BrowserCase::start("moving-target.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#moving"}]),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    case.tab
        .evaluate(
            "const element = document.querySelector('#moving'); element.style.animation = 'none'; void element.offsetWidth; element.style.animation = 'travel 1.5s linear forwards';",
            false,
        )
        .unwrap();

    let moving = case
        .runtime
        .world_manager
        .element_states(&case.tab, &handle)
        .unwrap();
    assert!(!moving.stable);
    tokio::time::sleep(Duration::from_millis(1_700)).await;
    let settled = case
        .runtime
        .world_manager
        .element_states(&case.tab, &handle)
        .unwrap();
    assert!(settled.stable);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn moving_target_waits_until_stable() {
    let Some(case) = BrowserCase::start("moving-target.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::WaitForElementStable {
                locator: BrowserLocator::css("#moving"),
                timeout_ms: Some(4_000),
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#moving"),
            },
            text_step("#result"),
        ],
    );
    assert!(report.ok, "stable wait and click failed: {report:?}");
    assert_eq!(returned_text(&report), "moving clicked");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn moving_target_click_reports_successful_retries() {
    let Some(mut case) = BrowserCase::start("moving-target.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate("globalThis.armMovingTargetForClick()", false)
        .unwrap();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#moving"),
            },
            text_step("#result"),
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "moving click should settle: {report:?}");
    assert_eq!(returned_text(&report), "moving clicked");
    let diagnostics = report.steps[0]
        .actionability
        .as_ref()
        .expect("retried click should include actionability diagnostics");
    assert!(diagnostics.attempts.unwrap_or_default() > 0);
    assert_eq!(diagnostics.stable, Some(true));
    assert_eq!(diagnostics.receives_events, Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn contenteditable_fill_updates_output() {
    let Some(case) = BrowserCase::start("contenteditable.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Fill {
            locator: BrowserLocator::css("#editor"),
            text: "editable text".to_string(),
            clear_first: true,
            verify: true,
        }],
    );
    assert!(report.ok, "contenteditable fill failed: {report:?}");
    let output = case
        .tab
        .evaluate("document.querySelector('#result').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(output, "editable text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn keyboard_types_unicode_into_contenteditable() {
    let Some(case) = BrowserCase::start("contenteditable.html").await else {
        return;
    };
    case.tab
        .evaluate("document.querySelector('#editor').focus()", false)
        .unwrap();
    let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(&case.tab));
    keyboard.press_sequentially("editable é🙂", None).unwrap();
    let value = case
        .tab
        .evaluate("document.querySelector('#result').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(value, "editable é🙂");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fetch_after_click_renders_slow_echo() {
    let Some(case) = BrowserCase::start("fetch-after-click.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#fetch"),
            },
            BrowserStep::WaitForText {
                text: "echo ok after 400ms".to_string(),
                timeout_ms: Some(3_000),
            },
            text_step("#result"),
        ],
    );
    assert!(report.ok, "fetch result did not render: {report:?}");
    assert_eq!(returned_text(&report), "echo ok after 400ms");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn locator_handler_clears_cookie_banner_before_click_and_records_firing() {
    let Some(case) = BrowserCase::start("cookie-banner.html").await else {
        return;
    };
    let tab = case.tab.clone();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::RemoveLocatorHandler {
                    name: "dismiss_overlays".to_string(),
                },
                BrowserStep::AddLocatorHandler {
                    name: "cookie-banner".to_string(),
                    locator: BrowserLocator::css("#accept-all"),
                    handler: LocatorHandlerAction::Click,
                    times: Some(1),
                    no_wait_after: false,
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#target"),
                },
            ],
        },
        &ImagePolicy::default(),
    )
    .await
    .unwrap();

    assert!(report.ok, "handler-assisted click failed: {report:?}");
    let text = tab
        .evaluate("document.querySelector('#target').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(text, "clicked");
    assert!(report
        .locator_handlers
        .iter()
        .any(|firing| firing.name == "cookie-banner" && firing.ok));
    let trusted_click = tab
        .evaluate("globalThis.handlerClickTrusted", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(trusted_click, json!(true));
    drop(runtime);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn locator_handler_clears_interstitial_that_appears_between_actions() {
    let Some(case) = BrowserCase::start("interstitial.html").await else {
        return;
    };
    let tab = case.tab.clone();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::RemoveLocatorHandler {
                    name: "dismiss_overlays".to_string(),
                },
                BrowserStep::AddLocatorHandler {
                    name: "interstitial".to_string(),
                    locator: BrowserLocator::css("#close"),
                    handler: LocatorHandlerAction::Click,
                    times: Some(1),
                    no_wait_after: false,
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#show"),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#target"),
                },
            ],
        },
        &ImagePolicy::default(),
    )
    .await
    .unwrap();

    assert!(report.ok, "interstitial handler failed: {report:?}");
    let text = tab
        .evaluate("document.querySelector('#target').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(text, "clicked");
    assert!(report
        .locator_handlers
        .iter()
        .any(|firing| firing.name == "interstitial" && firing.ok));
    drop(runtime);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn legacy_dismiss_overlays_step_still_clears_cookie_banner() {
    let Some(case) = BrowserCase::start("cookie-banner.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::DismissOverlays,
            BrowserStep::Click {
                locator: BrowserLocator::css("#target"),
            },
        ],
    );

    assert!(report.ok, "legacy dismiss step failed: {report:?}");
    let text = case
        .tab
        .evaluate("document.querySelector('#target').textContent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(text, "clicked");
    assert!(report
        .locator_handlers
        .iter()
        .any(|firing| firing.name == "dismiss_overlays" && firing.ok));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn popup_click_opens_second_tab() {
    let Some(mut case) = BrowserCase::start("popup.html").await else {
        return;
    };
    case.setup_world();
    let before = case.runtime.browser.get_tabs().lock().unwrap().len();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#open"),
            },
            BrowserStep::ListTabs,
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "popup click failed: {report:?}");
    let after = case.runtime.browser.get_tabs().lock().unwrap().len();
    assert_eq!(after, before + 1);
    assert_eq!(report.new_tabs.len(), 1);
    assert_eq!(
        report.new_tabs[0].opener.as_ref().unwrap().tab_id,
        case.tab.get_target_id().as_str()
    );
    let listed_tabs = report.steps[1].data.as_ref().unwrap()["tabs"]
        .as_array()
        .unwrap();
    assert_eq!(listed_tabs.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn switch_tab_retargets_later_steps() {
    let Some(mut case) = BrowserCase::start("popup.html").await else {
        return;
    };
    case.setup_world();
    let popup_report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Click {
            locator: BrowserLocator::css("#open"),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(popup_report.ok, "popup click failed: {popup_report:?}");
    let popup_id = popup_report.new_tabs[0].id.clone();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::SwitchTab {
                tab: TabTarget::Id { id: popup_id },
            },
            BrowserStep::Eval {
                expression: "document.querySelector('h1').textContent".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "switch-tab batch failed: {report:?}");
    assert_eq!(
        report.steps[1].data.as_ref().unwrap()["value"],
        "Popup child"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn wait_for_popup_click_and_popup_action_share_one_batch() {
    let Some(mut case) = BrowserCase::start("popup.html").await else {
        return;
    };
    case.setup_world();
    let primary_target_id = case.tab.get_target_id().to_string();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::WaitForPopup {
                    timeout_ms: Some(5_000),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#open"),
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('h1').textContent".to_string(),
                },
                BrowserStep::CloseTab { tab: None },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "popup batch failed: {report:?}");
    assert_eq!(
        report.steps[0].data.as_ref().unwrap()["tab_id"],
        report.new_tabs[0].id
    );
    assert_eq!(
        report.steps[2].data.as_ref().unwrap()["value"],
        "Popup child"
    );
    assert_eq!(
        runtime.lock().await.active_tab_target_id(),
        Some(primary_target_id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn network_routes_fulfill_abort_modify_redirects_unroute_and_reach_popups() {
    let Some(mut case) = BrowserCase::start("route-target.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let data_pattern = UrlPattern::Text(case.server.url("api/data"));
    let api_pattern = UrlPattern::Text(format!("{}/api/**", case.server.base_url));

    let fulfill = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::Fulfill {
                        status: 200,
                        headers: BTreeMap::new(),
                        body: Some(json!({"source": "mocked"}).to_string()),
                        content_type: Some("application/json".to_string()),
                        body_base64: false,
                    },
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "mocked".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(fulfill.ok, "fulfill route failed: {fulfill:?}");
    assert_eq!(fulfill.intercepted_requests[0].action, "fulfill");
    assert_eq!(fulfill.intercepted_requests[0].status, Some(200));
    assert!(fulfill
        .network
        .iter()
        .any(|request| request.url.ends_with("/api/data") && request.status == Some(200)));

    let abort = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Unroute {
                    pattern: Some(data_pattern.clone()),
                },
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::Abort {
                        reason: "blockedbyclient".to_string(),
                    },
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent = 'idle'"
                        .to_string(),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "fetch failed".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(abort.ok, "abort route failed: {abort:?}");
    assert_eq!(abort.intercepted_requests[0].action, "abort");
    assert_eq!(
        abort.intercepted_requests[0].reason.as_deref(),
        Some("blockedbyclient")
    );

    let modified = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Unroute { pattern: None },
                BrowserStep::Route {
                    pattern: api_pattern.clone(),
                    handler: RouteHandler::Continue {
                        url: None,
                        method: None,
                        headers: Some(BTreeMap::from([(
                            "X-Route-Test".to_string(),
                            "modified".to_string(),
                        )])),
                        post_data: None,
                    },
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent = 'idle'"
                        .to_string(),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-redirect"),
                },
                BrowserStep::WaitForText {
                    text: "modified".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(modified.ok, "continue route failed: {modified:?}");
    assert_eq!(modified.intercepted_requests.len(), 2);
    assert!(modified
        .intercepted_requests
        .iter()
        .any(|request| request.redirect_hop));
    assert!(modified
        .network
        .iter()
        .any(|request| request.url.ends_with("/api/redirect") && request.status == Some(302)));
    assert!(modified
        .network
        .iter()
        .any(|request| request.url.ends_with("/api/data") && request.status == Some(200)));

    let unroute = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Unroute { pattern: None },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent = 'idle'"
                        .to_string(),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "origin".to_string(),
                    timeout_ms: Some(5_000),
                },
                BrowserStep::ListRoutes,
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(unroute.ok, "unroute failed: {unroute:?}");
    assert!(unroute.active_routes.is_empty());
    assert!(unroute.intercepted_requests.is_empty());

    let popup = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Route {
                    pattern: data_pattern,
                    handler: RouteHandler::Fulfill {
                        status: 200,
                        headers: BTreeMap::new(),
                        body: Some(json!({"source": "popup-mocked"}).to_string()),
                        content_type: Some("application/json".to_string()),
                        body_base64: false,
                    },
                },
                BrowserStep::WaitForPopup {
                    timeout_ms: Some(5_000),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#open-popup"),
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "popup-mocked".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(popup.ok, "popup route inheritance failed: {popup:?}");
    assert_eq!(popup.new_tabs.len(), 1);
    assert!(popup
        .intercepted_requests
        .iter()
        .any(|request| request.action == "fulfill"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn websocket_route_registers_page_socket_and_delivers_mock_frame() {
    let Some(mut case) = BrowserCase::start("states.html").await else {
        return;
    };
    case.setup_world();
    let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
    let page = case.server.url("ws-echo.html");
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: false,
            steps: vec![
                BrowserStep::RouteWebSocket {
                    pattern: pattern.clone(),
                    mode: WebSocketRouteMode::Mock,
                },
                BrowserStep::Navigate { url: page },
                BrowserStep::SendWebSocketMessage {
                    url_pattern: pattern.clone(),
                    data: "mocked-frame".to_string(),
                },
                BrowserStep::WaitForText {
                    text: "mocked-frame".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "websocket routing failed: {report:?}");
    assert!(
        report.page_errors.is_empty(),
        "page errors while constructing the routed WebSocket: {:?}",
        report.page_errors
    );
    assert_eq!(
        report.steps[2].summary,
        "Sent WebSocket message to 1 socket(s)"
    );
    assert!(
        report.websockets.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::Created) && event.routed
        }),
        "no routed socket recorded: {:?}",
        report.websockets
    );
    assert!(
        report.websockets.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::FrameSent)
                && event.data.as_deref() == Some("hello")
        }),
        "page frame not observed: {:?}",
        report.websockets
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn iframe_form_submit_reaches_same_origin_frame() {
    let Some(case) = BrowserCase::start("iframe-form.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::role("textbox", Some("Name"))
                    .in_frames(vec![BrowserLocator::css("#form-frame")]),
                text: "Frame User".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::WaitForSelector {
                locator: BrowserLocator::role("button", Some("Submit"))
                    .in_frames(vec![BrowserLocator::css("#form-frame")]),
                timeout_ms: Some(2_000),
            },
            BrowserStep::Click {
                locator: BrowserLocator::role("button", Some("Submit"))
                    .in_frames(vec![BrowserLocator::css("#form-frame")]),
            },
            BrowserStep::WaitForText {
                text: "submitted Frame User".to_string(),
                timeout_ms: Some(2_000),
            },
        ],
    );
    assert!(report.ok, "same-origin iframe action failed: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn nested_iframe_click_and_hidden_wait_use_frame_chain() {
    let Some(case) = BrowserCase::start("nested-iframe.html").await else {
        return;
    };
    let frames = vec![
        BrowserLocator::css("#outer-frame"),
        BrowserLocator::css("#inner-frame"),
    ];
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::role("button", Some("Hide status"))
                    .in_frames(frames.clone()),
            },
            BrowserStep::WaitForElementHidden {
                locator: BrowserLocator::role("status", None).in_frames(frames),
                timeout_ms: Some(2_000),
            },
        ],
    );

    assert!(report.ok, "nested iframe action or wait failed: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn ambiguous_iframe_owner_reports_strict_violation() {
    let Some(case) = BrowserCase::start("nested-iframe.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::WaitForSelector {
            locator: BrowserLocator::role("button", Some("Hide status"))
                .in_frames(vec![BrowserLocator::css("iframe.owner")]),
            timeout_ms: Some(2_000),
        }],
    );

    assert!(!report.ok, "ambiguous frame owner must fail: {report:?}");
    assert!(report.steps[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("frame locator resolved to 2 iframe elements"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn nested_shadow_dom_button_and_input_are_actionable() {
    let Some(case) = BrowserCase::start("shadow-dom.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::css("#shadow-input"),
                text: "shadow value".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#shadow-button"),
            },
            BrowserStep::WaitForText {
                text: "shadow value".to_string(),
                timeout_ms: Some(2_000),
            },
        ],
    );
    assert!(report.ok, "nested shadow action failed: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn dialog_fixture_auto_dismisses_confirm_and_reports_it() {
    let Some(mut case) = BrowserCase::start("dialog.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::Eval {
                    expression: "document.querySelector('#confirm').click(); 'clicked'".to_string(),
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent".to_string(),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(report.ok, "dialog fixture click failed: {report:?}");
    assert_eq!(returned_eval_string(&report), "dismissed");
    assert_eq!(report.dialogs.len(), 1);
    assert!(report.dialogs[0].automatic);
    assert_eq!(
        report.dialogs[0].action,
        refact_lsp::integrations::browser_models::DialogAction::Dismissed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn dialog_fixture_uses_armed_accept_and_prompt_text() {
    let Some(mut case) = BrowserCase::start("dialog.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let accept_report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::HandleDialog {
                    accept: true,
                    prompt_text: None,
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#confirm').click(); 'clicked'".to_string(),
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent".to_string(),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(
        accept_report.ok,
        "accepted dialog failed: {accept_report:?}"
    );
    assert_eq!(returned_eval_string(&accept_report), "confirmed");
    assert!(!accept_report.dialogs[0].automatic);

    let prompt_report = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            steps: vec![
                BrowserStep::HandleDialog {
                    accept: true,
                    prompt_text: Some("Pixel".to_string()),
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#prompt').click(); 'clicked'".to_string(),
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent".to_string(),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(prompt_report.ok, "prompt dialog failed: {prompt_report:?}");
    assert_eq!(returned_eval_string(&prompt_report), "Pixel");
    assert_eq!(prompt_report.dialogs.len(), 1);
    assert_eq!(prompt_report.dialogs[0].default_value, "visitor");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn upload_route_accepts_multipart_body() {
    let Some(case) = BrowserCase::start("upload.html").await else {
        return;
    };
    let boundary = "refact-browser-fixture";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"fixture.txt\"\r\nContent-Type: text/plain\r\n\r\nfixture upload\r\n--{boundary}--\r\n"
    );
    let response = reqwest::Client::new()
        .post(case.server.url("upload"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["accepted"],
        true
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn download_route_sets_attachment_headers() {
    let Some(case) = BrowserCase::start("download.html").await else {
        return;
    };
    let response = reqwest::get(case.server.url("download")).await.unwrap();
    assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
    assert_eq!(
        response.headers().get("content-disposition").unwrap(),
        "attachment; filename=browser-fixture.txt"
    );
    assert_eq!(response.text().await.unwrap(), "browser fixture download\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn uploads_visible_hidden_and_file_chooser_inputs() {
    let Some(mut case) = BrowserCase::start("upload.html").await else {
        return;
    };
    case.setup_world();
    let file = case._profile.path().join("upload-fixture.txt");
    std::fs::write(&file, "browser upload fixture\n").unwrap();
    let path = file.to_string_lossy().into_owned();

    let direct = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::SetInputFiles {
                locator: BrowserLocator::label("Visible upload"),
                paths: vec![path.clone()],
            },
            BrowserStep::SetInputFiles {
                locator: BrowserLocator::css("#hidden-file"),
                paths: vec![path.clone()],
            },
            BrowserStep::Eval {
                expression: "JSON.stringify([document.querySelector('#visible-file').files.length, document.querySelector('#hidden-file').files.length])".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(direct.ok, "direct upload failed: {direct:?}");
    assert_eq!(direct.uploads.len(), 2);
    assert_eq!(direct.steps[2].data.as_ref().unwrap()["value"], "[1,1]");

    let chooser = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::ExpectFileChooser {
                paths: vec![path.clone()],
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#visible-file"),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(chooser.ok, "file chooser upload failed: {chooser:?}");
    assert_eq!(chooser.uploads.len(), 1);
    assert_eq!(chooser.uploads[0].source, "file_chooser");
    assert_eq!(chooser.uploads[0].paths, vec![path]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn eval_invokes_function_expressions_and_keeps_plain_expressions() {
    let Some(case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::Eval {
                expression: "() => 42".to_string(),
            },
            BrowserStep::Eval {
                expression: "(() => 42)()".to_string(),
            },
            BrowserStep::Eval {
                expression: "async () => 7".to_string(),
            },
            BrowserStep::Eval {
                expression: "(function () { return 5; })".to_string(),
            },
            BrowserStep::Eval {
                expression: "1+1".to_string(),
            },
            BrowserStep::Eval {
                expression: "document.title".to_string(),
            },
        ],
    );

    assert!(report.ok, "eval batch failed: {report:?}");
    let value = |index: usize| report.steps[index].data.as_ref().unwrap()["value"].clone();
    assert_eq!(value(0), json!(42));
    assert_eq!(value(1), json!(42));
    assert_eq!(value(2), json!(7));
    assert_eq!(value(3), json!(5));
    assert_eq!(value(4), json!(2));
    assert_eq!(value(5), json!("ARIA snapshot fixture"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn eval_surfaces_errors_thrown_by_invoked_functions() {
    let Some(case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[BrowserStep::Eval {
            expression: "() => { throw new Error(\"eval-boom\") }".to_string(),
        }],
    );

    assert!(!report.ok, "throwing eval unexpectedly passed: {report:?}");
    let error = report.steps[0].error.as_deref().unwrap_or_default();
    assert!(
        error.contains("eval-boom"),
        "unexpected eval error: {error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn captures_download_with_suggested_filename_and_runtime_path() {
    let Some(mut case) = BrowserCase::start("download.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#download"),
            },
            BrowserStep::WaitForDownload {
                timeout_ms: Some(10_000),
                save_as: Some("saved-fixture.txt".to_string()),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "download capture failed: {report:?}");
    assert_eq!(report.downloads.len(), 1);
    let download = &report.downloads[0];
    assert_eq!(download.suggested_filename, "browser-fixture.txt");
    assert!(download.received_bytes > 0);
    assert!(FsPath::new(&download.local_path).is_file());
    assert_eq!(
        std::fs::read_to_string(&download.local_path).unwrap(),
        "browser fixture download\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn element_states_absorb_detachment_between_animation_frames() {
    let Some(mut case) = BrowserCase::start("states.html").await else {
        return;
    };
    case.setup_world();
    let handle = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "resolveAll",
            json!([{"by":"css","value":"#readonly-input"}]),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let states = case
        .runtime
        .world_manager
        .call_function_on(
            &case.tab,
            &handle,
            "function() { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); const states = instance.elementStates(this); requestAnimationFrame(() => this.remove()); return states; }",
            Vec::new(),
        )
        .unwrap();
    assert_eq!(states["stable"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn aria_snapshot_serializes_composed_tree_and_distills_generics() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let default_snapshot = case
        .runtime
        .world_manager
        .aria_snapshot(
            &case.tab,
            None,
            refact_lsp::refact_browser::SnapshotOptions::default(),
        )
        .unwrap();
    assert!(!default_snapshot.yaml.contains("Hidden snapshot text"));

    let snapshot = case
        .runtime
        .world_manager
        .aria_snapshot(
            &case.tab,
            None,
            refact_lsp::refact_browser::SnapshotOptions {
                mode: refact_lsp::refact_browser::SnapshotMode::Ai,
                ..Default::default()
            },
        )
        .unwrap();
    let yaml = snapshot.yaml;
    for expected in [
        "- navigation \"Primary\"",
        "- link \"Guide\"",
        "- /url: /guide",
        "- button \"Save\"",
        "- heading \"Snapshot page\" [level=1]",
        "- heading \"Controls\" [level=2]",
        "- textbox \"Search\"",
        "- /placeholder: Find docs",
        "- checkbox \"Subscribe\" [checked]",
        "Before Center After",
        "- button \"Shadow action\"",
        "- group \"Owned group\"",
        "- button \"Owned action\"",
    ] {
        assert!(yaml.contains(expected), "missing {expected:?} in:\n{yaml}");
    }
    assert!(!yaml.contains("Hidden snapshot text"));
    assert!(!yaml.contains("- generic:\n    - link \"Guide\""));

    let boxed = case
        .runtime
        .world_manager
        .aria_snapshot(
            &case.tab,
            None,
            refact_lsp::refact_browser::SnapshotOptions {
                mode: refact_lsp::refact_browser::SnapshotMode::Ai,
                boxes: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(boxed.yaml.contains("[box="));
    assert!(boxed.nodes.iter().any(|node| node.role == "button"));

    let positions = [
        "navigation \"Primary\"",
        "link \"Guide\"",
        "button \"Save\"",
        "heading \"Snapshot page\"",
        "button \"Shadow action\"",
        "group \"Owned group\"",
        "button \"Owned action\"",
    ]
    .map(|line| yaml.find(line).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn selector_evaluator_matches_text_visibility_and_global_nth() {
    let Some(mut case) = BrowserCase::start("selectors.html").await else {
        return;
    };
    case.setup_world();
    let selectors = [
        "css=:text(\"Needle text\")",
        "css=article:has-text(\"ancestor needle\")",
        "css=:nth-match(.global-button, 2)",
        "css=button:visible",
        "css=body >> css=#second",
    ];
    let expected = [
        vec!["text-leaf"],
        vec!["has-text-parent"],
        vec!["second"],
        vec![
            "first",
            "second",
            "visible",
            "above",
            "near",
            "anchor",
            "below",
            "far",
            "shadow-button",
        ],
        vec!["second"],
    ];
    for (selector, expected_ids) in selectors.into_iter().zip(expected) {
        let handles = case
            .runtime
            .world_manager
            .call_injected_handles(&case.tab, "querySelectorAll", json!([selector]))
            .unwrap();
        let ids = handles
            .iter()
            .map(|handle| {
                case.runtime
                    .world_manager
                    .call_function_on(&case.tab, handle, "function() { return this.id; }", vec![])
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, expected_ids, "selector {selector}");
    }
    let native_nth_child_count = case
        .tab
        .evaluate(
            "document.querySelectorAll('.global-button:nth-child(2)').length",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(native_nth_child_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn selector_evaluator_preserves_shadow_and_xpath_boundaries() {
    let Some(mut case) = BrowserCase::start("selectors.html").await else {
        return;
    };
    case.setup_world();
    let queries = [
        ("css=#shadow-button", vec!["shadow-button"]),
        ("css=:light(#shadow-button)", vec![]),
    ];
    for (selector, expected_ids) in queries {
        let handles = case
            .runtime
            .world_manager
            .call_injected_handles(&case.tab, "querySelectorAll", json!([selector]))
            .unwrap();
        let ids = handles
            .iter()
            .map(|handle| {
                case.runtime
                    .world_manager
                    .call_function_on(&case.tab, handle, "function() { return this.id; }", vec![])
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, expected_ids, "selector {selector}");
    }
    let xpath = case
        .runtime
        .world_manager
        .call_injected_handles(&case.tab, "querySelectorAll", json!(["//button"]))
        .unwrap();
    assert_eq!(xpath.len(), 9);
    let ids = xpath
        .iter()
        .map(|handle| {
            case.runtime
                .world_manager
                .call_function_on(&case.tab, handle, "function() { return this.id; }", vec![])
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(!ids.iter().any(|id| id == "shadow-button"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "headless_chrome 1.0.20 drops Input.dragIntercepted on its non-flattened target transport"]
async fn html5_drag_and_drop_reaches_page_handler() {
    let Some(mut case) = BrowserCase::start("drag-drop.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::WaitForSelector {
                locator: BrowserLocator::css("#source"),
                timeout_ms: Some(2_000),
            },
            BrowserStep::DragAndDrop {
                source: BrowserLocator::css("#source"),
                target: BrowserLocator::css("#target"),
                source_position: None,
                target_position: None,
            },
            BrowserStep::Eval {
                expression: "({dragged:document.querySelector('#target').dataset.dropped,files:document.querySelector('#files').dataset.files})".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "drag/drop report: {report:?}");
    assert_eq!(
        report.steps[2].data.as_ref().unwrap()["value"]["dragged"],
        "dragged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn file_drop_and_coordinate_mouse_reach_page_handlers() {
    let Some(mut case) = BrowserCase::start("drag-drop.html").await else {
        return;
    };
    case.setup_world();
    let file = case._profile.path().join("drop.txt");
    std::fs::write(&file, "dropped").unwrap();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::WaitForSelector {
                locator: BrowserLocator::css("#files"),
                timeout_ms: Some(2_000),
            },
            BrowserStep::DropFiles {
                target: BrowserLocator::css("#files"),
                paths: vec![file.to_string_lossy().into_owned()],
            },
            BrowserStep::Eval {
                expression: "({dragged:document.querySelector('#target').dataset.dropped,files:document.querySelector('#files').dataset.files})".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "file drop report: {report:?}");
    assert_eq!(
        report.steps[2].data.as_ref().unwrap()["value"]["files"],
        "drop.txt",
        "file drop report: {report:?}"
    );

    case.navigate("canvas-draw.html");
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::MouseDragXy {
                start_x: 10.0,
                start_y: 10.0,
                end_x: 80.0,
                end_y: 60.0,
            },
            BrowserStep::Eval {
                expression: "document.querySelector('#canvas').dataset.drawn".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "coordinate mouse report: {report:?}");
    assert_eq!(report.steps[1].data.as_ref().unwrap()["value"], "yes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn selector_evaluator_matches_and_sorts_layout_relations() {
    let Some(mut case) = BrowserCase::start("selectors.html").await else {
        return;
    };
    case.setup_world();
    let selectors = [
        ("css=#layout button:above(#anchor)", vec!["above"]),
        ("css=#layout button:below(#anchor)", vec!["below", "far"]),
        ("css=#layout button:near(#anchor)", vec!["near"]),
    ];
    for (selector, expected_ids) in selectors {
        let handles = case
            .runtime
            .world_manager
            .call_injected_handles(&case.tab, "querySelectorAll", json!([selector]))
            .unwrap();
        let ids = handles
            .iter()
            .map(|handle| {
                case.runtime
                    .world_manager
                    .call_function_on(&case.tab, handle, "function() { return this.id; }", vec![])
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, expected_ids, "selector {selector}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn aria_refs_reuse_invalidate_and_resolve_latest_snapshot_elements() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.querySelector('button').addEventListener('click', () => document.body.dataset.saved = 'yes')",
            false,
        )
        .unwrap();
    let options = refact_lsp::refact_browser::SnapshotOptions {
        mode: refact_lsp::refact_browser::SnapshotMode::Ai,
        refs: true,
        ..Default::default()
    };
    let first = case
        .runtime
        .world_manager
        .aria_snapshot(&case.tab, None, options.clone())
        .unwrap();
    assert!(first.yaml.contains("[ref=e"));
    let save_ref = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Save"))
        .and_then(|node| node.reference.as_deref())
        .unwrap()
        .parse::<refact_lsp::refact_browser::Ref>()
        .unwrap();
    let search_ref = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Search"))
        .and_then(|node| node.reference.as_deref())
        .unwrap()
        .parse::<refact_lsp::refact_browser::Ref>()
        .unwrap();
    let save_handle = case
        .runtime
        .world_manager
        .resolve_ref(&case.tab, &save_ref)
        .unwrap();
    case.runtime
        .world_manager
        .call_function_on(
            &case.tab,
            &save_handle,
            "function() { this.click(); }",
            vec![],
        )
        .unwrap();
    assert_eq!(
        case.tab
            .evaluate("document.body.dataset.saved", false)
            .unwrap()
            .value
            .unwrap(),
        json!("yes")
    );
    assert!(case
        .runtime
        .world_manager
        .resolve_ref(&case.tab, &search_ref)
        .is_ok());

    let unchanged = case
        .runtime
        .world_manager
        .aria_snapshot(&case.tab, None, options.clone())
        .unwrap();
    let reused = unchanged
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Save"))
        .and_then(|node| node.reference.as_deref())
        .unwrap();
    assert_eq!(reused, save_ref.to_string());

    case.tab
        .evaluate(
            "document.querySelector('button').setAttribute('aria-label', 'Save changed')",
            false,
        )
        .unwrap();
    let renamed = case
        .runtime
        .world_manager
        .aria_snapshot(&case.tab, None, options.clone())
        .unwrap();
    let renamed_ref = renamed
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Save changed"))
        .and_then(|node| node.reference.as_deref())
        .unwrap()
        .parse::<refact_lsp::refact_browser::Ref>()
        .unwrap();
    assert_ne!(renamed_ref, save_ref);
    assert!(matches!(
        case.runtime.world_manager.resolve_ref(&case.tab, &save_ref),
        Err(refact_lsp::refact_browser::RefError::Stale { .. })
    ));

    case.tab
        .evaluate("document.querySelector('button').remove()", false)
        .unwrap();
    assert!(matches!(
        case.runtime
            .world_manager
            .resolve_ref(&case.tab, &renamed_ref),
        Err(refact_lsp::refact_browser::RefError::Detached { .. })
    ));

    case.navigate("delayed-button.html");
    assert!(matches!(
        case.runtime
            .world_manager
            .resolve_ref(&case.tab, &search_ref),
        Err(refact_lsp::refact_browser::RefError::GenerationMismatch { .. })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn production_snapshot_and_ref_actions_share_one_batch() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.querySelector('button').addEventListener('click', () => document.body.dataset.saved = 'yes')",
            false,
        )
        .unwrap();
    let snapshot = case
        .runtime
        .world_manager
        .aria_snapshot(
            &case.tab,
            None,
            refact_lsp::refact_browser::SnapshotOptions {
                mode: refact_lsp::refact_browser::SnapshotMode::Ai,
                refs: true,
                ..Default::default()
            },
        )
        .unwrap();
    let reference = |name: &str| {
        snapshot
            .nodes
            .iter()
            .find(|node| node.name.as_deref() == Some(name))
            .and_then(|node| node.reference.clone())
            .unwrap()
    };
    let save_ref = reference("Save");
    let search_ref = reference("Search");

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::AccessibilitySnapshot {
                options: AccessibilitySnapshotOptions::default(),
            },
            BrowserStep::Click {
                locator: BrowserLocator::reference(&save_ref),
            },
            BrowserStep::Fill {
                locator: BrowserLocator::reference(&search_ref),
                text: "ref batch".to_string(),
                clear_first: true,
                verify: true,
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "ref batch failed: {report:?}");
    assert_eq!(report.steps.len(), 3);
    let snapshot_data = report.steps[0].data.as_ref().unwrap();
    assert!(snapshot_data["yaml"].as_str().unwrap().contains("[ref=e"));
    assert_eq!(
        case.tab
            .evaluate("document.body.dataset.saved", false)
            .unwrap()
            .value
            .unwrap(),
        json!("yes")
    );
    assert_eq!(
        case.tab
            .evaluate("document.querySelector('[aria-label=Search]').value", false)
            .unwrap()
            .value
            .unwrap(),
        json!("ref batch")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn tool_contract_canonical_ref_batch_executes_end_to_end() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.querySelector('button').addEventListener('click', () => document.body.dataset.saved = 'yes')",
            false,
        )
        .unwrap();
    let snapshot = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::AccessibilitySnapshot {
            options: AccessibilitySnapshotOptions::default(),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(snapshot.ok, "snapshot failed: {snapshot:?}");
    let nodes = snapshot.steps[0].data.as_ref().unwrap()["nodes"]
        .as_array()
        .unwrap();
    let reference = |name: &str| {
        nodes
            .iter()
            .find(|node| node["name"].as_str() == Some(name))
            .and_then(|node| node["ref"].as_str())
            .unwrap_or_else(|| panic!("snapshot lacks ref for {name}: {nodes:?}"))
            .to_string()
    };
    let request: BrowserActionRequest = serde_json::from_value(json!({
        "steps": [
            {"action":"accessibility_snapshot"},
            {"action":"click","locator":{"by":"ref","value":reference("Save")}},
            {"action":"fill","locator":{"by":"ref","value":reference("Search")},"text":"hi"}
        ]
    }))
    .unwrap();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(runtime, request, &ImagePolicy::browser_capture())
        .await
        .unwrap();

    assert!(report.ok, "canonical ref batch failed: {report:?}");
    assert_eq!(report.steps.len(), 3);
    assert!(
        report.steps[1]
            .summary
            .to_ascii_lowercase()
            .contains("click"),
        "unexpected click summary: {:?}",
        report.steps[1]
    );
    assert!(
        report.steps[2]
            .summary
            .to_ascii_lowercase()
            .contains("fill"),
        "unexpected fill summary: {:?}",
        report.steps[2]
    );
    assert_eq!(report.steps[2].verified, Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn production_ref_errors_distinguish_stale_and_navigation_generations() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let options = AccessibilitySnapshotOptions::default();
    let first = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::AccessibilitySnapshot {
            options: options.clone(),
        }],
        &ImagePolicy::browser_capture(),
    );
    let save_ref = first.steps[0].data.as_ref().unwrap()["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "Save")
        .and_then(|node| node["ref"].as_str())
        .unwrap()
        .to_string();
    case.tab
        .evaluate(
            "document.querySelector('button').setAttribute('aria-label', 'Save changed')",
            false,
        )
        .unwrap();
    let stale = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::AccessibilitySnapshot { options },
            BrowserStep::Click {
                locator: BrowserLocator::reference(&save_ref),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(!stale.ok);
    assert!(stale.steps[1].error.as_deref().unwrap().contains("stale"));
    assert!(stale.steps[1].error.as_deref().unwrap().contains(&save_ref));

    let current_ref = stale.steps[0].data.as_ref().unwrap()["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "Save changed")
        .and_then(|node| node["ref"].as_str())
        .unwrap()
        .to_string();
    case.navigate("delayed-button.html");
    let navigated = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Click {
            locator: BrowserLocator::reference(&current_ref),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(!navigated.ok);
    assert!(navigated.steps[0]
        .error
        .as_deref()
        .unwrap()
        .contains("earlier document generation"));
    assert!(navigated.steps[0]
        .error
        .as_deref()
        .unwrap()
        .contains(&current_ref));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn generated_locators_follow_preferences_and_round_trip() {
    let Some(mut case) = BrowserCase::start("generator.html").await else {
        return;
    };
    case.setup_world();
    let cases = [
        ("test-id", Some("internal:testid=[data-qa=")),
        ("text-button", Some("internal:role=button[name=")),
        ("chained", Some(">>")),
        ("label", Some("internal:role=textbox[name=")),
        ("structural", Some("nth-child")),
    ];
    for (case_name, expected_fragment) in cases {
        let target = case
            .runtime
            .world_manager
            .call_injected_handles(
                &case.tab,
                "querySelectorAll",
                json!([format!("css=[data-generator-case={case_name:?}]")]),
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let locator = case
            .runtime
            .world_manager
            .generate_locator(
                &case.tab,
                &target,
                refact_lsp::refact_browser::LocatorGenerationOptions {
                    test_id_attribute_name: "data-qa".to_string(),
                },
            )
            .unwrap();
        if let Some(expected_fragment) = expected_fragment {
            assert!(
                locator.contains(expected_fragment),
                "locator {locator:?} for {case_name} did not contain {expected_fragment:?}"
            );
        }
        let resolved = case
            .runtime
            .world_manager
            .call_injected_handles(&case.tab, "querySelectorAll", json!([locator.clone()]))
            .unwrap();
        assert_eq!(resolved.len(), 1, "locator {locator:?} must be unique");
        assert_eq!(
            resolved[0].backend_node_id, target.backend_node_id,
            "locator {locator:?} must resolve to its source element"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn get_by_locators_match_playwright_semantics() {
    use refact_lsp::integrations::browser_models::LocatorStrategy;
    let Some(mut case) = BrowserCase::start("getby.html").await else {
        return;
    };
    case.setup_world();

    let resolve = |locator: serde_json::Value| {
        case.runtime
            .world_manager
            .call_injected_handles(&case.tab, "resolveAll", json!([locator]))
            .unwrap()
    };

    assert_eq!(
        resolve(json!({"by":"role","role":"button","name":"save item"})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"role","role":"button","name":"Save Item","exact":true})).len(),
        1
    );
    assert!(
        resolve(json!({"by":"role","role":"button","name":"save item","exact":true})).is_empty()
    );
    assert_eq!(
        resolve(json!({"by":"role","role":"button","description":"primary ACTION"})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"role","role":"heading","name":"account","level":2})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"role","role":"checkbox","checked":true})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"role","role":"button","disabled":true})).len(),
        1
    );
    assert!(resolve(json!({"by":"role","role":"button","name":"hidden"})).is_empty());
    assert_eq!(
        resolve(json!({"by":"role","role":"button","name":"hidden","include_hidden":true})).len(),
        1
    );

    let smallest = resolve(json!({"by":"text","value":"unique text"}));
    assert_eq!(smallest.len(), 1);
    let id = case
        .runtime
        .world_manager
        .call_function_on(
            &case.tab,
            &smallest[0],
            "function() { return this.id; }",
            vec![],
        )
        .unwrap();
    assert_eq!(id, json!("smallest-text"));
    assert_eq!(
        resolve(json!({"by":"text","value":"send request"})).len(),
        1
    );

    for value in [
        "Wrapping Label",
        "For Label",
        "ARIA Label",
        "Referenced Label",
    ] {
        assert_eq!(
            resolve(json!({"by":"label","value":value,"exact":true})).len(),
            1
        );
    }
    assert_eq!(
        resolve(json!({"by":"placeholder","value":"search WORKSPACE"})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"placeholder","value":"  Search   workspace  ","exact":true})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"alt_text","value":"product logo"})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"title","value":"More Information","exact":true})).len(),
        1
    );
    assert_eq!(
        resolve(json!({"by":"test_id","value":"save-card"})).len(),
        1
    );

    let custom = refact_lsp::refact_browser::test_id_locator("custom-card", "data-qa");
    assert_eq!(resolve(serde_json::to_value(custom).unwrap()).len(), 1);

    let regex = LocatorStrategy::Text {
        value: "does not matter".to_string(),
        exact: true,
        regex: Some(refact_lsp::integrations::browser_models::LocatorRegex {
            source: "unique\\s+text".to_string(),
            flags: "i".to_string(),
        }),
    };
    assert_eq!(
        resolve(
            serde_json::to_value(BrowserLocator {
                strategy: regex,
                frames: Vec::new(),
                nth: None,
                within: None,
                locator: None,
                filter: None,
                and: None,
                or: None,
                first: None,
                last: None
            })
            .unwrap()
        )
        .len(),
        1
    );
    let regex: BrowserLocator = serde_json::from_value(json!({
        "by": "text",
        "value": "does not matter",
        "exact": true,
        "regex": {"source": "unique\\s+text", "flags": "i"}
    }))
    .unwrap();
    assert_eq!(resolve(serde_json::to_value(regex).unwrap()).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn composed_locators_match_playwright_semantics() {
    let Some(mut case) = BrowserCase::start("compose.html").await else {
        return;
    };
    case.setup_world();

    let resolve_ids = |locator: serde_json::Value| {
        case.runtime
            .world_manager
            .call_injected_handles(&case.tab, "resolveAll", json!([locator]))
            .unwrap()
            .iter()
            .map(|handle| {
                case.runtime
                    .world_manager
                    .call_function_on(
                        &case.tab,
                        handle,
                        "function() { return this.id || this.getAttribute('data-testid'); }",
                        vec![],
                    )
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        resolve_ids(
            json!({"by":"css","value":"[data-testid=alpha-card]","locator":{"by":"role","role":"button","name":"Open"}})
        ),
        vec!["alpha-open".to_string()]
    );
    assert_eq!(
        resolve_ids(
            json!({"by":"css","value":".card","filter":{"has_text":"Beta archived"},"locator":{"by":"css","value":"button"}})
        ),
        vec!["beta-open".to_string()]
    );
    assert_eq!(
        resolve_ids(
            json!({"by":"css","value":".card","filter":{"has":{"by":"css","value":".ready"}}})
        ),
        vec!["alpha-card".to_string()]
    );
    assert_eq!(
        resolve_ids(
            json!({"by":"css","value":".card","filter":{"has_not":{"by":"css","value":".ready"},"has_not_text":{"source":"archived","flags":"i"},"visible":true}})
        ),
        vec!["gamma-card".to_string()]
    );
    assert_eq!(
        resolve_ids(json!({"by":"css","value":".card","and":{"by":"css","value":".featured"}})),
        vec!["alpha-card".to_string()]
    );
    assert_eq!(
        resolve_ids(
            json!({"by":"test_id","value":"gamma-card","or":{"by":"test_id","value":"alpha-card"}})
        ),
        vec!["alpha-card".to_string(), "gamma-card".to_string()]
    );
    assert_eq!(
        resolve_ids(json!({"by":"css","value":".card-action","first":true})),
        vec!["alpha-open".to_string()]
    );
    assert_eq!(
        resolve_ids(json!({"by":"css","value":".card-action","last":true})),
        vec!["hidden-open".to_string()]
    );
    assert_eq!(
        resolve_ids(json!({"by":"css","value":".card-action","nth":1})),
        vec!["beta-open".to_string()]
    );

    let nth_match = case
        .runtime
        .world_manager
        .call_injected_handles(
            &case.tab,
            "querySelectorAll",
            json!(["css=:nth-match(.card-action, 1)"]),
        )
        .unwrap();
    let nth_match_id = case
        .runtime
        .world_manager
        .call_function_on(
            &case.tab,
            &nth_match[0],
            "function() { return this.id; }",
            vec![],
        )
        .unwrap();
    assert_eq!(nth_match_id, json!("alpha-open"));

    let ambiguous_or = execute_steps(&case.tab, &[BrowserStep::Click {
        locator: serde_json::from_value(json!({"by":"test_id","value":"alpha-card","locator":{"by":"css","value":"button"},"or":{"by":"test_id","value":"fallback"}})).unwrap(),
    }]);
    assert!(!ambiguous_or.ok);
    assert!(ambiguous_or.steps[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("resolved to 2 elements"));

    assert_eq!(
        resolve_ids(json!({"by":"css","value":".card-action"})).len(),
        4
    );

    let multi = execute_steps(
        &case.tab,
        &[BrowserStep::ExtractLinks {
            locator: Some(BrowserLocator::css(".card")),
            limit: None,
        }],
    );
    assert!(multi.ok, "multi-element extract_links failed: {multi:?}");
    assert_eq!(multi.steps[0].data.as_ref().unwrap()["total"], json!(2));
}
