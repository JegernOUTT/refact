use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{Page, Runtime};
use refact_core::image_policy::ImagePolicy;
use refact_lsp::call_validation::ChatContent;
use refact_lsp::chat::browser_context::maybe_insert_browser_context;
use refact_lsp::integrations::browser_controller::execute_steps as execute_fixture_steps_with_policy;
use refact_lsp::integrations::browser_controller::execute_steps as execute_steps_with_policy;
use refact_lsp::integrations::browser_controller::execute_request_with_runtime;
use refact_lsp::integrations::browser_controller::execute_steps_with_runtime;
use refact_lsp::integrations::browser_models::{
    AccessibilitySnapshotOptions, BrowserActionRequest, BrowserComposeMode, BrowserConsoleLevel,
    BrowserElementState, BrowserExpectation, BrowserExpectedText, BrowserExpectedTextOrList,
    BrowserHttpRequest, BrowserLoadState, BrowserLocator, BrowserPdfOptions, BrowserPollMatcher,
    BrowserPseudoElement, BrowserScreenshotAnimations, BrowserScreenshotClip,
    BrowserScreenshotOptions, BrowserStep, BrowserTextMode, CdpTarget, ClockTicks, ClockTime,
    ExecutionReport, FillStrategy, LocatorHandlerAction, LocatorRegex, NetworkReportMode,
    PageContextMode, RouteHandler, SessionPolicy, TabTarget, UrlPattern, WebSocketEventKind,
    WebSocketFrameDisposition, WebSocketMessageAction, WebSocketRouteMode,
};
use refact_lsp::refact_browser::devices;
use refact_lsp::refact_browser::{
    BrowserLaunchOptions, BrowserRuntime, CdpKeyboardDispatcher, CdpMouseDispatcher, CheckedState,
    HandleError, Keyboard, HitTargetController, HitTargetResult, Mouse, MouseButton,
    UTILITY_WORLD_NAME,
};
use serde_json::json;
use structopt::StructOpt;
use tempfile::{tempdir, TempDir};

mod browser_common;

use browser_common::{
    discover_chrome, discover_chrome_with, e2e_enabled, e2e_launch_options, print_skip,
    FixtureServer,
};

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
    "animation.html",
    "states.html",
    "roles.html",
    "accname.html",
    "snapshot.html",
    "controlled-input.html",
    "input-events.html",
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
    "readouts.html",
    "compose.html",
    "hostile-globals.html",
    "hit-target.html",
    "selectors.html",
    "cookie-banner.html",
    "interstitial.html",
    "generator.html",
    "ws-echo.html",
    "ws-intercept.html",
    "har-target.html",
    "clock.html",
    "visual-states.html",
    "poll-state.html",
    "aria-shadow.html",
];

fn execute_steps(
    tab: &Tab,
    steps: &[BrowserStep],
) -> refact_lsp::integrations::browser_models::ExecutionReport {
    execute_steps_with_policy(tab, steps, &ImagePolicy::default())
}

struct WsEchoServer {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

fn negotiate_subprotocol(
    request: &tokio_tungstenite::tungstenite::handshake::server::Request,
    mut response: tokio_tungstenite::tungstenite::handshake::server::Response,
) -> Result<
    tokio_tungstenite::tungstenite::handshake::server::Response,
    tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
> {
    const PROTOCOL: &str = "sec-websocket-protocol";
    let selected = request
        .headers()
        .get(PROTOCOL)
        .and_then(|offered| offered.to_str().ok())
        .and_then(|offered| offered.split(',').next())
        .and_then(|offered| offered.trim().parse().ok());
    if let Some(selected) = selected {
        response.headers_mut().insert(PROTOCOL, selected);
    }
    Ok(response)
}

impl WsEchoServer {
    async fn start() -> Result<Self, String> {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut socket) =
                        tokio_tungstenite::accept_hdr_async(stream, negotiate_subprotocol).await
                    else {
                        return;
                    };
                    while let Some(Ok(message)) = socket.next().await {
                        let Message::Text(text) = message else {
                            continue;
                        };
                        if socket
                            .send(Message::Text(format!("echo:{text}")))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Ok(Self { address, task })
    }

    fn url(&self) -> String {
        format!("ws://{}/ws-echo", self.address)
    }
}

impl Drop for WsEchoServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn launch_browser(profile: &TempDir) -> BrowserRuntime {
    BrowserRuntime::launch(
        profile.path().to_path_buf(),
        e2e_launch_options(discover_chrome()),
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
                timeout_ms: None,
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
                timeout_ms: None,
            }],
        );
        assert!(report.ok, "navigation failed: {report:?}");
    }

    fn setup_world(&mut self) {
        refact_lsp::refact_browser::setup_recording_for_tab(&mut self.runtime, self.tab.clone())
            .unwrap();
    }

    fn probe_utility_world(&self) -> String {
        self.runtime
            .world_manager
            .aria_snapshot(
                &self.tab,
                None,
                refact_lsp::refact_browser::SnapshotOptions::default(),
            )
            .unwrap()
            .yaml
    }
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
async fn a_default_navigate_batch_returns_a_snapshot_context_and_zero_images() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let target = case.server.url("getby.html");
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let report = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![BrowserStep::Navigate {
                url: target.clone(),
                timeout_ms: None,
            }],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "navigate failed: {report:?}");
    assert!(
        report.screenshot.is_none(),
        "the default page context must not attach an image"
    );
    let page = report
        .page
        .clone()
        .expect("a navigate batch must return a page block");
    let snapshot = page
        .snapshot
        .expect("a page-changing batch must attach the aria snapshot");
    assert!(snapshot.bytes > 0);
    assert!(snapshot.lines > 0);
    assert!(
        snapshot.yaml.contains("[ref="),
        "the attached snapshot must carry actionable refs: {}",
        snapshot.yaml
    );
    assert_eq!(snapshot.artifact.is_some(), snapshot.truncated);
    assert_eq!(page.status, None, "a 200 document is not surfaced");
    assert_eq!(report.url.as_deref(), Some(target.as_str()));
    assert!(report.title.is_some());

    let mut envelope = serde_json::to_value(&report).unwrap();
    envelope["page"]["snapshot"]["yaml"] = serde_json::Value::String(String::new());
    let header = serde_json::to_string(&envelope["page"]).unwrap();
    assert!(
        header.len() <= 600,
        "page header was {} chars: {header}",
        header.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn page_context_screenshot_returns_a_png_instead_of_a_snapshot() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let target = case.server.url("getby.html");
    let snapshot_page = case.server.url("snapshot.html");
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: Some(PageContextMode::Screenshot),
            network: NetworkReportMode::default(),
            steps: vec![BrowserStep::Navigate {
                url: target,
                timeout_ms: None,
            }],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "navigate failed: {report:?}");
    let screenshot = report
        .screenshot
        .expect("screenshot mode must attach an image");
    assert!(screenshot.mime.starts_with("image/"));
    assert!(!screenshot.data.is_empty());
    assert!(
        report.page.and_then(|page| page.snapshot).is_none(),
        "screenshot mode must not attach the aria snapshot"
    );

    let both = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: Some(PageContextMode::Both),
            network: NetworkReportMode::default(),
            steps: vec![BrowserStep::Navigate {
                url: snapshot_page,
                timeout_ms: None,
            }],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(both.screenshot.is_some());
    assert!(both.page.and_then(|page| page.snapshot).is_some());
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
        page_context: None,
        network: NetworkReportMode::default(),
        steps: vec![BrowserStep::Eval {
            expression: "document.querySelector('#fetch').click()".to_string(),
        }],
        block_service_workers: None,
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
    assert!(report.network.is_empty());
    assert!(report
        .network_summary
        .iter()
        .any(|line| { line.contains("/slow-echo") && line.contains(" 200 ") }));
    assert_eq!(
        execute_request_with_runtime(
            runtime,
            BrowserActionRequest {
                session: SessionPolicy::SharedDefault,
                target: TabTarget::Active,
                attach_screenshot: None,
                page_context: None,
                network: NetworkReportMode::default(),
                steps: vec![],
                block_service_workers: None,
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
async fn status_filtered_response_wait_skips_the_first_failure_and_matches_the_retry() {
    let Some(mut case) = BrowserCase::start("fetch-after-click.html").await else {
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
            page_context: None,
            network: NetworkReportMode::Full,
            steps: vec![
                BrowserStep::Eval {
                    expression: "fetch('/probe-status-missing').then(function() { return fetch('/slow-echo?probe-status=retry&ms=200'); }); 'started'".to_string(),
                },
                BrowserStep::WaitForResponse {
                    pattern: UrlPattern::Regex {
                        source: "probe-status".to_string(),
                        flags: String::new(),
                    },
                    method: Some("GET".to_string()),
                    status: Some(200),
                    timeout_ms: Some(5_000),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(
        report.ok,
        "status-filtered response wait failed: {report:?}"
    );
    let matched = report.steps[1].data.as_ref().unwrap();
    assert_eq!(matched["status"], 200);
    assert!(
        matched["url"].as_str().unwrap().contains("slow-echo"),
        "status filter matched the 404 instead of the retry: {matched}"
    );
    assert!(
        report
            .network
            .iter()
            .any(|entry| entry.url.contains("probe-status-missing") && entry.status == Some(404)),
        "the skipped 404 should still be reported: {report:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn console_wait_catches_a_delayed_error_and_ignores_quieter_levels() {
    let Some(mut case) = BrowserCase::start("fetch-after-click.html").await else {
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
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::Eval {
                    expression: "console.log('console-probe quiet'); setTimeout(function() { console.error('console-probe late boom'); }, 400); 'started'".to_string(),
                },
                BrowserStep::WaitForConsoleMessage {
                    contains: Some("console-probe".to_string()),
                    level: Some(BrowserConsoleLevel::Error),
                    timeout_ms: Some(5_000),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "console wait failed: {report:?}");
    let matched = report.steps[1].data.as_ref().unwrap();
    assert!(
        matched["text"].as_str().unwrap().contains("late boom"),
        "console wait returned the quiet log instead of the delayed error: {matched}"
    );
    assert!(
        matched["level"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case("error"),
        "unexpected console level: {matched}"
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
            page_context: None,
            network: NetworkReportMode::Full,
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
                    pattern: UrlPattern::Regex {
                        source: "/missing-network-resource$".to_string(),
                        flags: String::new(),
                    },
                    method: None,
                    status: None,
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
            block_service_workers: None,
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
async fn http_request_shares_the_page_session_cookie_in_both_directions() {
    let Some(mut case) = BrowserCase::start("login").await else {
        return;
    };
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::HttpRequest {
            options: BrowserHttpRequest {
                url: case.server.url("api/session"),
                fail_on_status: Some(true),
                ..Default::default()
            },
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "http_request failed: {report:?}");

    let data = &report.steps[0].data.as_ref().unwrap()["http_request"];
    assert_eq!(data["status"], 200);
    assert_eq!(data["method"], "GET");
    assert_eq!(data["redirects"], 0);
    assert_eq!(data["headers"]["content-type"], "application/json");
    assert!(
        data["body"].as_str().unwrap().contains("logged-in"),
        "the API did not see the page session cookie: {data}"
    );
    assert_eq!(data["set_cookies"]["count"], 1);
    assert_eq!(data["set_cookies"]["names"][0], "api_issued");
    assert!(data.get("artifact").is_none());
    assert!(!serde_json::to_string(data).unwrap().contains("from-api"));

    case.navigate("context-probe.html");
    let cookies = case
        .tab
        .evaluate("document.cookie", false)
        .unwrap()
        .value
        .unwrap();
    let cookies = cookies.as_str().unwrap();
    assert!(cookies.contains("api_issued=from-api"), "{cookies}");
    assert!(cookies.contains("fixture_session=logged-in"), "{cookies}");
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
                timeout_ms: None,
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
async fn an_idle_session_outlives_its_own_idle_eviction_window() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let server = FixtureServer::start().await.unwrap();
    let profile = tempdir().unwrap();
    let idle_window = Duration::from_secs(2);
    let mut runtime = BrowserRuntime::launch(
        profile.path().to_path_buf(),
        BrowserLaunchOptions {
            headless: true,
            chrome_path: discover_chrome(),
            idle_timeout: Some(idle_window),
            ..BrowserLaunchOptions::default()
        },
    )
    .expect("browser launch must succeed after e2e_enabled");
    let tab = runtime.browser.new_tab().unwrap();
    runtime.set_active_tab_target_id(tab.get_target_id().to_string());

    assert_eq!(runtime.idle_timeout, idle_window);
    tokio::time::sleep(idle_window * 3).await;

    assert!(
        runtime.check_connection(),
        "cdp transport must survive silence longer than the idle eviction window"
    );
    assert!(
        runtime.is_idle_expired(),
        "our own idle knob must still measure inactivity"
    );

    let runtime = Arc::new(tokio::sync::Mutex::new(runtime));
    let report = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            block_service_workers: None,
            steps: vec![BrowserStep::Navigate {
                url: server.url("delayed-button.html"),
                timeout_ms: None,
            }],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .expect("idle session must still accept a batch");
    assert!(report.ok, "idle session batch failed: {report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn dead_cdp_transport_is_relaunched_and_the_batch_is_retried_once() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    const CHAT_ID: &str = "browser-dead-transport-e2e";
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
    let dead_runtime_id = refact_lsp::integrations::browser_runtime::register_browser_runtime(
        app.clone(),
        case.runtime,
    )
    .await;

    let (_, runtime_arc) =
        refact_lsp::integrations::browser_runtime::find_runtime_by_chat_id(app.clone(), CHAT_ID)
            .await
            .expect("registered runtime must resolve by chat id");

    {
        let mut rt = runtime_arc.lock().await;
        let session = rt.cdp_session().expect("raw cdp session must connect");
        let _ = session.send("Browser.close", None, None);
    }
    for _ in 0..50 {
        let dead = {
            let mut rt = runtime_arc.lock().await;
            !rt.check_connection()
        };
        if dead {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    {
        let mut rt = runtime_arc.lock().await;
        assert!(
            !rt.check_connection(),
            "closing the browser must kill the cdp transport"
        );
    }

    let report =
        refact_lsp::integrations::browser_controller::execute_request_with_runtime_validated(
            runtime_arc,
            BrowserActionRequest {
                session: SessionPolicy::SharedDefault,
                target: TabTarget::Active,
                attach_screenshot: None,
                page_context: None,
                network: NetworkReportMode::default(),
                block_service_workers: None,
                steps: vec![BrowserStep::Navigate {
                    url: case.server.url("delayed-button.html"),
                    timeout_ms: None,
                }],
            },
            &ImagePolicy::browser_capture(),
            gcx.clone(),
        )
        .await
        .expect("dead transport must be recovered, not surfaced");

    assert!(report.ok, "relaunched batch must succeed: {report:?}");
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning == refact_lsp::integrations::browser_runtime::RELAUNCH_WARNING),
        "recovery must be visible in the report: {:?}",
        report.warnings
    );

    let (live_runtime_id, live_runtime) =
        refact_lsp::integrations::browser_runtime::find_runtime_by_chat_id(app.clone(), CHAT_ID)
            .await
            .expect("chat must own a live runtime after recovery");
    assert_ne!(live_runtime_id, dead_runtime_id);
    {
        let mut rt = live_runtime.lock().await;
        assert!(rt.check_connection());
        assert_eq!(rt.attached_chat_id.as_deref(), Some(CHAT_ID));
        assert_eq!(rt.profile_dir, case._profile.path());
    }

    refact_lsp::integrations::browser_runtime::remove_browser_runtime(app, &live_runtime_id).await;
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
            page_context: None,
            network: NetworkReportMode::default(),
            block_service_workers: None,
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
                    locator: None,
                    depth: None,
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
    assert!(
        case.probe_utility_world()
            .contains("Hostile globals fixture"),
        "utility world should answer despite hostile page globals"
    );
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
    assert!(
        !case.probe_utility_world().trim().is_empty(),
        "utility world should answer before navigation"
    );
    case.navigate("hostile-globals.html");
    assert!(
        case.probe_utility_world()
            .contains("Hostile globals fixture"),
        "utility world should be reinjected into the new document"
    );
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
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#assertion-text")),
                matcher: BrowserExpectation::ToContainText {
                    expected: BrowserExpectedTextOrList::One(BrowserExpectedText::Text(
                        "beta 42".to_string(),
                    )),
                    ignore_case: true,
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#assertion-text")),
                matcher: BrowserExpectation::ToHaveText {
                    expected: BrowserExpectedTextOrList::One(BrowserExpectedText::Regex(
                        LocatorRegex {
                            source: r"Alpha\s+Beta\s+\d+".to_string(),
                            flags: String::new(),
                        },
                    )),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css(".assertion-item")),
                matcher: BrowserExpectation::ToHaveCount { expected: 3 },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Navigate {
                url: case.server.url("snapshot.html"),
                timeout_ms: None,
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
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::role("navigation", Some("Primary"))),
                matcher: BrowserExpectation::ToMatchAriaSnapshot {
                    expected: "- navigation:\n  - link\n  - button \"Save\"".to_string(),
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("h1")),
                matcher: BrowserExpectation::ToHaveText {
                    expected: BrowserExpectedTextOrList::One(BrowserExpectedText::Text(
                        "Missing heading".to_string(),
                    )),
                    ignore_case: false,
                },
                timeout_ms: Some(50),
                soft: true,
                not: None,
            },
            BrowserStep::Expect {
                locator: None,
                matcher: BrowserExpectation::ToHaveTitle {
                    expected: BrowserExpectedText::Text("ARIA snapshot fixture".to_string()),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
                not: None,
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
async fn expect_option_parity_covers_negation_indeterminate_pseudo_presence_and_lists() {
    let Some(mut case) = BrowserCase::start("delayed-button.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.head.insertAdjacentHTML('beforeend', '<style>#badge::before { content: \"done\"; }</style>'); document.body.insertAdjacentHTML('beforeend', '<p id=status>Loading</p><span id=badge data-ready></span><input id=mixed type=checkbox><div id=aria-mixed role=checkbox aria-checked=mixed></div><ul><li class=row>Alpha one</li><li class=row>Beta two</li><li class=row>Gamma three</li></ul>'); document.querySelector('#mixed').indeterminate = true; setTimeout(() => document.querySelector('#status').textContent = 'Ready', 2000)",
            false,
        )
        .unwrap();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#status")),
                matcher: BrowserExpectation::ToHaveText {
                    expected: BrowserExpectedTextOrList::One(BrowserExpectedText::Text(
                        "Loading".to_string(),
                    )),
                    ignore_case: false,
                },
                timeout_ms: Some(8_000),
                soft: false,
                not: Some(true),
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#mixed")),
                matcher: BrowserExpectation::ToBeChecked {
                    checked: None,
                    indeterminate: Some(true),
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#mixed")),
                matcher: BrowserExpectation::ToBeChecked {
                    checked: None,
                    indeterminate: None,
                },
                timeout_ms: Some(100),
                soft: false,
                not: Some(true),
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#badge")),
                matcher: BrowserExpectation::ToHaveCss {
                    name: "content".to_string(),
                    expected: BrowserExpectedText::Text("\"done\"".to_string()),
                    ignore_case: false,
                    pseudo: Some(BrowserPseudoElement::Before),
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#badge")),
                matcher: BrowserExpectation::ToHaveAttribute {
                    name: "data-ready".to_string(),
                    expected: None,
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#badge")),
                matcher: BrowserExpectation::ToHaveAttribute {
                    name: "data-missing".to_string(),
                    expected: None,
                    ignore_case: false,
                },
                timeout_ms: Some(100),
                soft: false,
                not: Some(true),
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css(".row")),
                matcher: BrowserExpectation::ToHaveText {
                    expected: BrowserExpectedTextOrList::Many(vec![
                        BrowserExpectedText::Text("Alpha one".to_string()),
                        BrowserExpectedText::Text("Beta two".to_string()),
                        BrowserExpectedText::Text("Gamma three".to_string()),
                    ]),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css(".row")),
                matcher: BrowserExpectation::ToContainText {
                    expected: BrowserExpectedTextOrList::Many(vec![
                        BrowserExpectedText::Text("Alpha".to_string()),
                        BrowserExpectedText::Text("Gamma".to_string()),
                    ]),
                    ignore_case: false,
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css(".row")),
                matcher: BrowserExpectation::ToContainText {
                    expected: BrowserExpectedTextOrList::Many(vec![
                        BrowserExpectedText::Text("Gamma".to_string()),
                        BrowserExpectedText::Text("Alpha".to_string()),
                    ]),
                    ignore_case: false,
                },
                timeout_ms: Some(100),
                soft: false,
                not: Some(true),
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#status")),
                matcher: BrowserExpectation::ToBeInViewport { ratio: Some(1.0) },
                timeout_ms: None,
                soft: false,
                not: None,
            },
            BrowserStep::Expect {
                locator: Some(BrowserLocator::css("#aria-mixed")),
                matcher: BrowserExpectation::ToBeChecked {
                    checked: None,
                    indeterminate: Some(true),
                },
                timeout_ms: None,
                soft: false,
                not: None,
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "expect option parity batch failed: {report:?}");
    let negated = report.steps[0].assertion.as_ref().unwrap();
    assert_eq!(negated.matcher, "not to_have_text");
    assert!(negated.passed);
    assert_eq!(
        negated.received,
        json!("Ready"),
        "negation must report the state that stopped matching: {report:?}"
    );
    assert_eq!(
        report.steps[1].assertion.as_ref().unwrap().received,
        json!(true)
    );
    assert_eq!(
        report.steps[3].assertion.as_ref().unwrap().received,
        json!("\"done\"")
    );
    assert_eq!(
        report.steps[4].assertion.as_ref().unwrap().expected,
        json!("<present>")
    );
    assert_eq!(
        report.steps[6].assertion.as_ref().unwrap().received,
        json!(["Alpha one", "Beta two", "Gamma three"])
    );
    assert!(
        report.steps[10].ok,
        "aria-checked=mixed must satisfy indeterminate: {report:?}"
    );
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
async fn tap_requires_touch_emulation_then_fires_touch_events() {
    let Some(mut case) = BrowserCase::start("input-events.html").await else {
        return;
    };

    let without_touch = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Tap {
            locator: Some(BrowserLocator::css("#tap-target")),
            x: None,
            y: None,
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(
        !without_touch.ok,
        "tap must require touch: {without_touch:?}"
    );
    let error = without_touch.steps[0].error.clone().unwrap();
    assert!(error.contains("set_viewport"), "unexpected error: {error}");
    assert!(error.contains("has_touch"), "unexpected error: {error}");

    let with_touch = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::SetViewport {
                width: 390,
                height: 844,
                device_scale_factor: Some(1.0),
                is_mobile: Some(true),
                has_touch: Some(true),
            },
            BrowserStep::Tap {
                locator: Some(BrowserLocator::css("#tap-target")),
                x: None,
                y: None,
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(with_touch.ok, "tap failed: {with_touch:?}");

    let recorded = case
        .tab
        .evaluate("window.recorded.tap.join(',')", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(recorded, json!("touchstart,touchend,click"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn emulate_device_changes_reported_viewport_and_user_agent() {
    let Some(mut case) = BrowserCase::start("input-events.html").await else {
        return;
    };

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::EmulateDevice {
            name: "Pixel 7".to_string(),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "emulate_device failed: {report:?}");
    assert!(report.steps[0].summary.contains("Pixel 7"));

    let metrics: serde_json::Value = serde_json::from_str(
        case.tab
            .evaluate(
                "JSON.stringify({inner: innerWidth, screen: screen.width, dpr: devicePixelRatio})",
                false,
            )
            .unwrap()
            .value
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(metrics["inner"], json!(412), "viewport metrics: {metrics}");
    assert_eq!(metrics["screen"], json!(412), "viewport metrics: {metrics}");
    assert_eq!(metrics["dpr"], json!(2.625), "viewport metrics: {metrics}");
    let user_agent = case
        .tab
        .evaluate("navigator.userAgent", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(
        user_agent,
        json!(devices::lookup("Pixel 7").unwrap().user_agent)
    );
    let touch_points = case
        .tab
        .evaluate("navigator.maxTouchPoints > 0", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(touch_points, json!(true));

    let unknown = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::EmulateDevice {
            name: "Pixel 777".to_string(),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(!unknown.ok, "unknown device must fail: {unknown:?}");
    let error = unknown.steps[0].error.clone().unwrap();
    assert!(error.contains("Unknown device 'Pixel 777'"), "{error}");
    assert!(error.contains("Pixel 7"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn network_and_cpu_throttling_apply_and_reset_clears_them() {
    let Some(mut case) = BrowserCase::start("input-events.html").await else {
        return;
    };

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::SetNetworkConditions {
                offline: None,
                latency_ms: None,
                download_kbps: None,
                upload_kbps: None,
                preset: Some("slow-3g".to_string()),
            },
            BrowserStep::SetCpuThrottling { rate: 4.0 },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "throttling failed: {report:?}");
    assert!(report.steps[0].summary.contains("2000ms latency"));
    assert!(report.steps[1].summary.contains("4"));
    assert_eq!(
        case.runtime.context_state.network_conditions,
        Some(refact_lsp::refact_browser::NetworkConditions::preset("slow-3g").unwrap())
    );
    assert_eq!(case.runtime.context_state.cpu_throttling_rate, Some(4.0));

    let reset = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Reset],
        &ImagePolicy::browser_capture(),
    );
    assert!(reset.ok, "reset failed: {reset:?}");
    assert!(case.runtime.context_state.network_conditions.is_none());
    assert!(case.runtime.context_state.cpu_throttling_rate.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn insert_text_produces_input_event_without_key_events() {
    let Some(case) = BrowserCase::start("input-events.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::InsertText {
            locator: Some(BrowserLocator::css("#ime")),
            text: "こんにちは".to_string(),
        }],
    );
    assert!(report.ok, "insert_text failed: {report:?}");

    let recorded = case
        .tab
        .evaluate("window.recorded.ime.join(',')", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(recorded, json!("input"));

    let value = case
        .tab
        .evaluate("document.querySelector('#ime').value", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(value, json!("こんにちは"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn press_sequentially_triggers_per_character_key_handlers() {
    let Some(case) = BrowserCase::start("input-events.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::PressSequentially {
            locator: BrowserLocator::css("#autocomplete"),
            text: "abc".to_string(),
            delay_ms: Some(5),
        }],
    );
    assert!(report.ok, "press_sequentially failed: {report:?}");

    let keys = case
        .tab
        .evaluate("window.recorded.suggestions.join(',')", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(keys, json!("a,b,c"));

    let suggestions = case
        .tab
        .evaluate("document.querySelectorAll('#suggestions li').length", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(suggestions, json!(3));

    let value = case
        .tab
        .evaluate("document.querySelector('#autocomplete').value", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(value, json!("abc"));
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
                state: None,
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
async fn wait_for_function_resolves_a_delayed_main_world_flag() {
    let Some(case) = BrowserCase::start("poll-state.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::WaitForFunction {
            expression: "() => globalThis.__refactReady".to_string(),
            locator: None,
            timeout_ms: Some(5_000),
            polling_ms: None,
        }],
    );

    assert!(report.ok, "delayed flag must settle: {report:?}");
    let data = report.steps[0].data.as_ref().unwrap();
    assert_eq!(data["value"], json!(true));
    assert!(data["attempts"].as_u64().unwrap() >= 1);
    assert!(data["elapsed_ms"].as_u64().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn wait_for_function_re_resolves_its_locator_across_re_renders() {
    let Some(case) = BrowserCase::start("poll-state.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::WaitForFunction {
            expression: "el => el.dataset.state === 'ready' && el.dataset.render".to_string(),
            locator: Some(BrowserLocator::css("#row")),
            timeout_ms: Some(15_000),
            polling_ms: Some(60),
        }],
    );

    assert!(
        report.ok,
        "re-rendered element must be re-resolved: {report:?}"
    );
    let data = report.steps[0].data.as_ref().unwrap();
    assert!(
        data["attempts"].as_u64().unwrap() > 1,
        "expected retries across re-renders: {data}"
    );
    let render = data["value"]
        .as_str()
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or_default();
    assert!(
        render > 1,
        "predicate must have run against a replacement node, not the original: {data}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn expect_poll_waits_for_a_numeric_threshold_and_reports_attempts() {
    let Some(case) = BrowserCase::start("poll-state.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::ExpectPoll {
            expression: "globalThis.__refactCounter".to_string(),
            expected: json!(4),
            matcher: BrowserPollMatcher::Gt,
            timeout_ms: Some(5_000),
            soft: None,
        }],
    );

    assert!(report.ok, "counter must exceed the threshold: {report:?}");
    let assertion = report.steps[0].assertion.as_ref().unwrap();
    assert_eq!(assertion.matcher, "gt");
    assert!(assertion.passed);
    assert!(!assertion.soft);
    assert_eq!(assertion.expected, json!(4));
    assert!(assertion.received.as_f64().unwrap() > 4.0);
    assert!(assertion.attempts > 1, "expected polling: {assertion:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn a_throwing_predicate_fails_immediately_instead_of_retrying() {
    let Some(case) = BrowserCase::start("poll-state.html").await else {
        return;
    };
    let started = std::time::Instant::now();
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::WaitForFunction {
            expression: "() => globalThis.__refactExplode()".to_string(),
            locator: None,
            timeout_ms: Some(5_000),
            polling_ms: None,
        }],
    );

    assert!(!report.ok, "a throwing predicate must fail: {report:?}");
    let error = report.steps[0].error.as_deref().unwrap_or_default();
    assert!(error.contains("predicate exploded"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "a throw must not consume the whole timeout"
    );
    assert_eq!(report.steps[0].retries, 0);
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
async fn input_value_reads_the_typed_property_not_the_stale_attribute() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::css("#stale"),
                text: "typed-value".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::InputValue {
                locator: BrowserLocator::css("#stale"),
            },
            BrowserStep::GetAttribute {
                locator: BrowserLocator::css("#stale"),
                attribute: "value".to_string(),
            },
            BrowserStep::InputValue {
                locator: BrowserLocator::css("#notes"),
            },
            BrowserStep::InputValue {
                locator: BrowserLocator::css("#pick"),
            },
        ],
    );

    assert!(report.ok, "input_value batch failed: {report:?}");
    assert_eq!(
        report.steps[1].data.as_ref().unwrap()["value"],
        json!("typed-value")
    );
    assert_eq!(
        report.steps[2].data.as_ref().unwrap()["value"],
        json!("attribute-value")
    );
    assert_eq!(
        report.steps[3].data.as_ref().unwrap()["value"],
        json!("textarea-value")
    );
    assert_eq!(
        report.steps[4].data.as_ref().unwrap()["value"],
        json!("beta")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn input_value_rejects_non_field_elements() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::InputValue {
            locator: BrowserLocator::css("#not-a-field"),
        }],
    );

    assert!(
        !report.ok,
        "input_value must reject a plain div: {report:?}"
    );
    let error = report.steps[0].error.as_deref().unwrap_or_default();
    assert!(error.contains("<input>"), "{error}");
    assert!(error.contains("<select>"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn count_reports_every_match_without_strictness() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::Count {
                locator: BrowserLocator::css(".row"),
            },
            BrowserStep::Count {
                locator: BrowserLocator::css("#never-rendered"),
            },
        ],
    );

    assert!(report.ok, "count must not be strict: {report:?}");
    assert_eq!(report.steps[0].data.as_ref().unwrap()["count"], json!(4));
    assert_eq!(report.steps[1].data.as_ref().unwrap()["count"], json!(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn all_texts_honours_the_limit_and_reports_the_true_total() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::AllTexts {
                locator: BrowserLocator::css(".row"),
                mode: BrowserTextMode::InnerText,
                limit: Some(2),
            },
            BrowserStep::AllTexts {
                locator: BrowserLocator::css(".row"),
                mode: BrowserTextMode::TextContent,
                limit: None,
            },
        ],
    );

    assert!(report.ok, "all_texts batch failed: {report:?}");
    let limited = report.steps[0].data.as_ref().unwrap();
    assert_eq!(limited["texts"], json!(["First", "Second"]));
    assert_eq!(limited["total"], json!(4));

    let full = report.steps[1].data.as_ref().unwrap();
    assert_eq!(
        full["texts"],
        json!(["First clipped", "Second", "Third", "Fourth"])
    );
    assert_eq!(full["total"], json!(4));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn bounding_box_measures_visible_elements_and_is_null_when_hidden() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::BoundingBox {
                locator: BrowserLocator::css("#measured"),
            },
            BrowserStep::BoundingBox {
                locator: BrowserLocator::css("#display-none"),
            },
        ],
    );

    assert!(report.ok, "bounding_box batch failed: {report:?}");
    let measured = &report.steps[0].data.as_ref().unwrap()["bounding_box"];
    assert_eq!(measured["x"], json!(40.0));
    assert_eq!(measured["y"], json!(60.0));
    assert_eq!(measured["width"], json!(220.0));
    assert_eq!(measured["height"], json!(30.0));
    assert_eq!(
        report.steps[1].data.as_ref().unwrap()["bounding_box"],
        json!(null)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn element_state_reports_every_flag_in_one_read() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[
            BrowserStep::ElementState {
                locator: BrowserLocator::css("#ticked"),
            },
            BrowserStep::ElementState {
                locator: BrowserLocator::css("#display-none"),
            },
        ],
    );

    assert!(report.ok, "element_state batch failed: {report:?}");
    let ticked = &report.steps[0].data.as_ref().unwrap()["state"];
    assert_eq!(ticked["visible"], json!(true));
    assert_eq!(ticked["enabled"], json!(true));
    assert_eq!(ticked["checked"], json!("checked"));
    assert_eq!(ticked["stable"], json!(true));
    assert_eq!(
        report.steps[1].data.as_ref().unwrap()["state"]["visible"],
        json!(false)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn readouts_stay_strict_for_single_element_steps() {
    let Some(case) = BrowserCase::start("readouts.html").await else {
        return;
    };
    let report = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::BoundingBox {
            locator: BrowserLocator::css(".row"),
        }],
    );

    assert!(
        !report.ok,
        "bounding_box must reject multiple matches: {report:?}"
    );
    let error = report.steps[0].error.as_deref().unwrap_or_default();
    assert!(error.contains("resolved to 4 elements"), "{error}");
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
            state: None,
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
async fn expectation_values_follow_aria_checked_and_aria_disabled() {
    let Some(mut case) = BrowserCase::start("aria-shadow.html").await else {
        return;
    };
    case.setup_world();
    let mut values = Vec::new();
    for selector in ["#aria-checked", "#aria-switch", "#aria-disabled-input"] {
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
        values.push(
            case.runtime
                .world_manager
                .expectation_values(&case.tab, &handle)
                .unwrap(),
        );
    }

    assert_eq!(values[0]["checked"], json!(true));
    assert_eq!(values[0]["indeterminate"], json!(false));
    assert_eq!(values[1]["checked"], json!(false));
    assert_eq!(values[2]["enabled"], json!(false));
    assert_eq!(values[2]["editable"], json!(false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn extract_links_pierces_open_shadow_roots() {
    let Some(case) = BrowserCase::start("aria-shadow.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[BrowserStep::ExtractLinks {
            locator: Some(BrowserLocator::css("body")),
            limit: None,
        }],
    );
    assert!(report.ok, "shadow extract_links failed: {report:?}");
    let data = report.steps[0].data.as_ref().unwrap();
    assert_eq!(data["total"], json!(3));
    let urls = data["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| link["url"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(urls.contains(&"https://example.com/shadow".to_string()));
    assert!(urls.contains(&"https://example.com/nested".to_string()));
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
            page_context: None,
            network: NetworkReportMode::default(),
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
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
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
            block_service_workers: None,
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
            BrowserStep::DismissOverlays { aggressive: false },
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
async fn wait_for_popup_defers_until_a_later_step_in_the_runtime_batch_path() {
    let Some(mut case) = BrowserCase::start("popup.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::WaitForPopup {
                timeout_ms: Some(5_000),
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#open"),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "runtime popup batch failed: {report:?}");
    assert_eq!(report.new_tabs.len(), 1);
    assert_eq!(
        report.steps[0].data.as_ref().unwrap()["tab_id"],
        report.new_tabs[0].id
    );
    assert_eq!(
        report.steps[0].summary,
        format!("Popup opened: {}", report.new_tabs[0].id)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn arming_a_second_popup_wait_fails_in_the_runtime_batch_path() {
    let Some(mut case) = BrowserCase::start("popup.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::WaitForPopup {
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForPopup {
                timeout_ms: Some(1_000),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(!report.ok, "second popup arm should fail: {report:?}");
    assert_eq!(
        report.steps[1].error.as_deref(),
        Some("a popup wait is already armed at step 0")
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
            page_context: None,
            network: NetworkReportMode::default(),
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
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::Full,
            steps: vec![
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::Fulfill {
                        status: 200,
                        headers: BTreeMap::new(),
                        body: Some(json!({"source": "mocked"}).to_string()),
                        path: None,
                        json: None,
                        content_type: Some("application/json".to_string()),
                        body_base64: false,
                    },
                    times: None,
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "mocked".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::Unroute {
                    pattern: Some(data_pattern.clone()),
                },
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::Abort {
                        reason: "blockedbyclient".to_string(),
                    },
                    times: None,
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
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::Full,
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
                    times: None,
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
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
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
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::Route {
                    pattern: data_pattern,
                    handler: RouteHandler::Fulfill {
                        status: 200,
                        headers: BTreeMap::new(),
                        body: Some(json!({"source": "popup-mocked"}).to_string()),
                        path: None,
                        json: None,
                        content_type: Some("application/json".to_string()),
                        body_base64: false,
                    },
                    times: None,
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
            block_service_workers: None,
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
async fn newest_route_falls_back_to_the_older_handler_and_times_expires_it() {
    let Some(mut case) = BrowserCase::start("route-target.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let data_pattern = UrlPattern::Text(case.server.url("api/data"));

    let chained = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::Fulfill {
                        status: 200,
                        headers: BTreeMap::new(),
                        body: None,
                        path: None,
                        json: Some(json!({"source": "oldest"})),
                        content_type: None,
                        body_base64: false,
                    },
                    times: None,
                },
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::Fallback,
                    times: Some(1),
                },
                BrowserStep::ListRoutes,
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "oldest".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(chained.ok, "fallback chain failed: {chained:?}");
    assert_eq!(chained.intercepted_requests[0].action, "fulfill");
    assert_eq!(chained.intercepted_requests[0].status, Some(200));

    let listed = chained.steps[2]
        .data
        .as_ref()
        .and_then(|data| data.get("routes"))
        .and_then(|routes| routes.as_array())
        .expect("list_routes payload");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["order"], json!(0));
    assert_eq!(listed[0]["handler"]["type"], json!("fallback"));
    assert_eq!(listed[0]["times_remaining"], json!(1));
    assert_eq!(listed[1]["order"], json!(1));

    let expired = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![BrowserStep::ListRoutes],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(expired.ok, "list_routes failed: {expired:?}");
    assert_eq!(expired.active_routes.len(), 1);
    assert!(matches!(
        expired.active_routes[0].handler,
        RouteHandler::Fulfill { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fetch_and_fulfill_replays_the_real_response_with_overrides() {
    let Some(mut case) = BrowserCase::start("route-target.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let data_pattern = UrlPattern::Text(case.server.url("api/data"));

    let fetched = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::Route {
                    pattern: data_pattern.clone(),
                    handler: RouteHandler::FetchAndFulfill {
                        url: None,
                        method: None,
                        headers: Some(BTreeMap::from([(
                            "X-Route-Test".to_string(),
                            "fetched".to_string(),
                        )])),
                        post_data: None,
                        status: Some(201),
                        response_headers: BTreeMap::from([(
                            "X-Mutated".to_string(),
                            "yes".to_string(),
                        )]),
                        body: None,
                        body_base64: false,
                    },
                    times: None,
                },
                BrowserStep::Click {
                    locator: BrowserLocator::css("#fetch-data"),
                },
                BrowserStep::WaitForText {
                    text: "fetched".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(fetched.ok, "fetch_and_fulfill failed: {fetched:?}");
    assert_eq!(fetched.intercepted_requests[0].action, "fetch_and_fulfill");
    assert_eq!(fetched.intercepted_requests[0].status, Some(201));
    assert!(fetched.intercepted_requests[0]
        .response_body_preview
        .as_deref()
        .is_some_and(|preview| preview.contains("fetched")));
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
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::RouteWebSocket {
                    pattern: pattern.clone(),
                    mode: WebSocketRouteMode::Mock,
                    on_page_message: WebSocketMessageAction::Forward,
                    on_server_message: WebSocketMessageAction::Forward,
                },
                BrowserStep::Navigate {
                    url: page,
                    timeout_ms: None,
                },
                BrowserStep::SendWebSocketMessage {
                    pattern: pattern.clone(),
                    text: "mocked-frame".to_string(),
                },
                BrowserStep::WaitForText {
                    text: "mocked-frame".to_string(),
                    timeout_ms: Some(5_000),
                },
            ],
            block_service_workers: None,
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
        report
            .websockets
            .iter()
            .any(|event| { matches!(event.kind, WebSocketEventKind::Created) && event.routed }),
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

struct InterceptCase {
    report: ExecutionReport,
    tab: Arc<Tab>,
    _echo: WsEchoServer,
    _runtime: Arc<tokio::sync::Mutex<BrowserRuntime>>,
    _server: FixtureServer,
    _profile: TempDir,
}

async fn intercept_case(
    on_page_message: WebSocketMessageAction,
    on_server_message: WebSocketMessageAction,
    extra: Vec<BrowserStep>,
) -> Option<InterceptCase> {
    let mut case = BrowserCase::start("states.html").await?;
    case.setup_world();
    let echo = WsEchoServer::start().await.unwrap();
    let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
    let page = format!("{}?ws={}", case.server.url("ws-intercept.html"), echo.url());
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let mut steps = vec![
        BrowserStep::RouteWebSocket {
            pattern,
            mode: WebSocketRouteMode::Intercept,
            on_page_message,
            on_server_message,
        },
        BrowserStep::Navigate {
            url: page,
            timeout_ms: None,
        },
    ];
    steps.extend(extra);

    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::default(),
            steps,
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    Some(InterceptCase {
        report,
        tab: case.tab,
        _echo: echo,
        _runtime: runtime,
        _server: case.server,
        _profile: case._profile,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn websocket_intercept_mode_round_trips_through_the_real_server() {
    let Some(case) = intercept_case(
        WebSocketMessageAction::Forward,
        WebSocketMessageAction::Forward,
        vec![BrowserStep::WaitForText {
            text: "echo:hello".to_string(),
            timeout_ms: Some(5_000),
        }],
    )
    .await
    else {
        return;
    };
    let report = &case.report;

    assert!(report.ok, "intercept round-trip failed: {report:?}");
    assert!(
        report.page_errors.is_empty(),
        "page errors during intercept: {:?}",
        report.page_errors
    );
    assert!(
        report.websockets.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::FrameSent)
                && event.data.as_deref() == Some("hello")
                && event.disposition == Some(WebSocketFrameDisposition::Forwarded)
        }),
        "page frame was not forwarded to the server: {:?}",
        report.websockets
    );
    assert!(
        report.websockets.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::FrameReceived)
                && event.data.as_deref() == Some("echo:hello")
                && event.disposition == Some(WebSocketFrameDisposition::Forwarded)
        }),
        "server frame was not forwarded to the page: {:?}",
        report.websockets
    );
    assert!(
        report
            .websockets
            .iter()
            .any(|event| { event.protocols == vec!["chat-v1".to_string(), "chat-v2".to_string()] }),
        "page-requested subprotocols missing from the report: {:?}",
        report.websockets
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn websocket_drop_blocks_the_page_frame_from_reaching_the_server() {
    let Some(case) = intercept_case(
        WebSocketMessageAction::Drop,
        WebSocketMessageAction::Forward,
        vec![BrowserStep::WaitForWebSocketFrame {
            pattern: None,
            timeout_ms: Some(1_500),
        }],
    )
    .await
    else {
        return;
    };
    let report = &case.report;

    assert!(
        !report.steps[2].ok,
        "a dropped frame must not satisfy wait_for_web_socket_frame: {:?}",
        report.steps[2]
    );
    let echoed = execute_fixture_steps(
        &case.tab,
        &[BrowserStep::Expect {
            locator: Some(BrowserLocator::css("#received")),
            matcher: BrowserExpectation::ToHaveText {
                expected: refact_lsp::integrations::browser_models::BrowserExpectedTextOrList::One(
                    BrowserExpectedText::Text("waiting".to_string()),
                ),
                ignore_case: false,
            },
            not: None,
            timeout_ms: Some(1_000),
            soft: false,
        }],
    );
    assert!(
        echoed.ok,
        "the server never echoed, so #received must still read 'waiting': {echoed:?}"
    );
    assert!(
        report.websockets.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::FrameSent)
                && event.data.as_deref() == Some("hello")
                && event.disposition == Some(WebSocketFrameDisposition::Dropped)
        }),
        "the dropped frame should still be reported as dropped: {:?}",
        report.websockets
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn websocket_close_delivers_code_and_reason_to_the_page() {
    let Some(case) = intercept_case(
        WebSocketMessageAction::Forward,
        WebSocketMessageAction::Forward,
        vec![
            BrowserStep::WaitForText {
                text: "echo:hello".to_string(),
                timeout_ms: Some(5_000),
            },
            BrowserStep::CloseWebSocket {
                pattern: UrlPattern::Text("ws://**/ws-echo".to_string()),
                code: Some(4002),
                reason: Some("server restarting".to_string()),
            },
            BrowserStep::WaitForText {
                text: "closed:4002:server restarting".to_string(),
                timeout_ms: Some(5_000),
            },
        ],
    )
    .await
    else {
        return;
    };

    let report = &case.report;

    assert!(report.ok, "close simulation failed: {report:?}");
    assert_eq!(report.steps[3].summary, "Closed 1 WebSocket(s)");
    assert!(
        report.websockets.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::Closed)
                && event.close_code == Some(4002)
                && event.close_reason.as_deref() == Some("server restarting")
        }),
        "close code and reason missing from the report: {:?}",
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
                state: None,
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
            state: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::Eval {
                    expression: "document.querySelector('#confirm').click(); 'clicked'".to_string(),
                },
                BrowserStep::Eval {
                    expression: "document.querySelector('#result').textContent".to_string(),
                },
            ],
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
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
            block_service_workers: None,
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
            page_context: None,
            network: NetworkReportMode::default(),
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
            block_service_workers: None,
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
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
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
                state: None,
                timeout_ms: Some(2_000),
            },
            BrowserStep::DragAndDrop {
                source: BrowserLocator::css("#source"),
                target: BrowserLocator::css("#target"),
                source_position: None,
                target_position: None,
            },
            BrowserStep::Eval {
                expression: "String(document.querySelector('#target').dataset.dropped)".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "drag/drop report: {report:?}");
    assert_eq!(report.steps[2].data.as_ref().unwrap()["value"], "dragged");
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
                state: None,
                timeout_ms: Some(2_000),
            },
            BrowserStep::DropFiles {
                target: BrowserLocator::css("#files"),
                paths: vec![file.to_string_lossy().into_owned()],
            },
            BrowserStep::Eval {
                expression: "String(document.querySelector('#files').dataset.files)".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "file drop report: {report:?}");
    assert_eq!(report.steps[2].data.as_ref().unwrap()["value"], "drop.txt");

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
async fn scoped_accessibility_snapshot_returns_only_the_target_subtree() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.querySelector('nav button').addEventListener('click', () => document.body.dataset.saved = 'yes')",
            false,
        )
        .unwrap();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::AccessibilitySnapshot {
            options: AccessibilitySnapshotOptions {
                locator: Some(BrowserLocator::css("nav")),
                boxes: true,
                ..Default::default()
            },
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "scoped snapshot failed: {report:?}");
    let data = report.steps[0].data.as_ref().unwrap();
    let yaml = data["yaml"].as_str().unwrap();
    assert!(yaml.contains("Save"), "subtree content missing: {yaml}");
    assert!(yaml.contains("Guide"), "subtree content missing: {yaml}");
    assert!(
        !yaml.contains("Snapshot page"),
        "scoped snapshot leaked content outside the subtree: {yaml}"
    );
    assert!(
        !yaml.contains("Search"),
        "scoped snapshot leaked content outside the subtree: {yaml}"
    );
    assert!(
        yaml.contains("[box="),
        "boxes must compose with scoping: {yaml}"
    );

    let save_ref = data["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"].as_str() == Some("Save"))
        .and_then(|node| node["ref"].as_str())
        .unwrap()
        .to_string();
    let click = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Click {
            locator: BrowserLocator::reference(&save_ref),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(
        click.ok,
        "a ref minted by a scoped snapshot must stay actionable: {click:?}"
    );
    assert_eq!(
        case.tab
            .evaluate("document.body.dataset.saved", false)
            .unwrap()
            .value
            .unwrap(),
        json!("yes")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn depth_limited_accessibility_snapshot_marks_truncated_children() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::AccessibilitySnapshot {
                options: AccessibilitySnapshotOptions::default(),
            },
            BrowserStep::AccessibilitySnapshot {
                options: AccessibilitySnapshotOptions {
                    depth: Some(1),
                    ..Default::default()
                },
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "snapshots failed: {report:?}");
    let full = report.steps[0].data.as_ref().unwrap()["yaml"]
        .as_str()
        .unwrap();
    let limited = report.steps[1].data.as_ref().unwrap()["yaml"]
        .as_str()
        .unwrap();

    assert!(
        !full.contains("truncated"),
        "an unlimited snapshot must not gain markers: {full}"
    );
    assert!(
        limited.contains("… (") && limited.contains("truncated)"),
        "depth limit must emit a counted marker: {limited}"
    );
    assert!(
        limited.lines().count() < full.lines().count(),
        "depth limit must shrink the snapshot"
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
    assert!(resolve(json!({"by":"test_id","value":"save"})).is_empty());
    assert_eq!(
        resolve(json!({"by":"test_id","value":"save","exact":false})).len(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn capture_frames_records_an_animation_as_a_labelled_filmstrip() {
    let Some(mut case) = BrowserCase::start("animation.html").await else {
        return;
    };
    let policy = ImagePolicy::browser_capture();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Eval {
                expression: "playAnimation()".to_string(),
            },
            BrowserStep::CaptureFrames {
                duration_ms: Some(900),
                frame_count: Some(6),
                interval_ms: None,
                locator: None,
                full_page: None,
            },
        ],
        &policy,
    );

    assert!(report.ok, "capture_frames failed: {report:?}");
    let data = report.steps[1].data.as_ref().unwrap();
    assert_eq!(data["frame_count"], json!(6));
    assert_eq!(data["artifact"]["kind"], json!("filmstrip"));
    assert!(data["data"].as_str().is_some_and(|data| !data.is_empty()));
    assert_eq!(data["columns"], json!(4));
    assert_eq!(data["rows"], json!(2));

    let frames = data["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 6);
    assert!(frames[0].get("changed_percent").is_none());
    for frame in frames {
        let path = FsPath::new(frame["artifact"]["path"].as_str().unwrap());
        assert!(path.exists(), "missing frame artifact {path:?}");
        assert_eq!(frame["artifact"]["kind"], json!("frame"));
    }
    let moved = frames
        .iter()
        .skip(1)
        .filter_map(|frame| frame["changed_percent"].as_f64())
        .any(|changed| changed > 0.0);
    assert!(moved, "animation produced no measurable motion: {frames:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn capture_frames_scopes_to_an_element_and_to_the_full_page() {
    let Some(mut case) = BrowserCase::start("animation.html").await else {
        return;
    };
    let policy = ImagePolicy::browser_capture();

    let scoped = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Eval {
                expression: "playAnimation()".to_string(),
            },
            BrowserStep::CaptureFrames {
                duration_ms: Some(600),
                frame_count: Some(3),
                interval_ms: None,
                locator: Some(BrowserLocator::css("#stage")),
                full_page: None,
            },
        ],
        &policy,
    );
    assert!(scoped.ok, "element-scoped capture failed: {scoped:?}");
    let scoped_data = scoped.steps[1].data.as_ref().unwrap();
    let scoped_frame = &scoped_data["frames"][0]["artifact"];
    assert_eq!(scoped_frame["width"], json!(480));
    assert_eq!(scoped_frame["height"], json!(240));
    assert_eq!(
        scoped_data["warnings"],
        json!(["element-scoped frames use timed screenshots"])
    );

    let full = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::CaptureFrames {
            duration_ms: Some(400),
            frame_count: Some(2),
            interval_ms: None,
            locator: None,
            full_page: Some(true),
        }],
        &policy,
    );
    assert!(full.ok, "full-page capture failed: {full:?}");
    assert_eq!(
        full.steps[0].data.as_ref().unwrap()["warnings"],
        json!(["full-page frames use timed screenshots"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn capture_frames_rejects_requests_beyond_the_hard_caps() {
    let Some(mut case) = BrowserCase::start("animation.html").await else {
        return;
    };
    let policy = ImagePolicy::browser_capture();

    for (step, expected) in [
        (
            BrowserStep::CaptureFrames {
                duration_ms: Some(10_001),
                frame_count: None,
                interval_ms: None,
                locator: None,
                full_page: None,
            },
            "duration_ms 10001 exceeds the 10000ms capture cap",
        ),
        (
            BrowserStep::CaptureFrames {
                duration_ms: Some(1_000),
                frame_count: Some(25),
                interval_ms: None,
                locator: None,
                full_page: None,
            },
            "frame_count 25, outside the supported 2..=24 range",
        ),
    ] {
        let report = execute_steps_with_runtime(&mut case.runtime, &[step], &policy);
        assert!(!report.ok, "cap was not enforced: {report:?}");
        assert_eq!(report.steps[0].error.as_deref(), Some(expected));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn set_window_bounds_moves_and_resizes_the_real_headed_window() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let profile = tempdir().unwrap();
    let mut runtime = BrowserRuntime::launch(
        profile.path().to_path_buf(),
        BrowserLaunchOptions {
            headless: false,
            chrome_path: discover_chrome(),
            idle_timeout: Some(Duration::from_secs(120)),
            window_bounds: Some(refact_lsp::refact_chat_api::WindowBounds {
                x: 30,
                y: 40,
                width: 900,
                height: 700,
            }),
            ..BrowserLaunchOptions::default()
        },
    )
    .expect("headed browser launch must succeed");
    let tab = runtime.browser.new_tab().unwrap();
    runtime.set_active_tab_target_id(tab.get_target_id().to_string());

    let report = execute_steps_with_runtime(
        &mut runtime,
        &[BrowserStep::SetWindowBounds {
            x: Some(60),
            y: Some(80),
            width: Some(1024),
            height: Some(768),
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "set_window_bounds failed: {report:?}");
    let data = report.steps[0].data.as_ref().unwrap();
    assert_eq!(data["applied"], json!(true));
    assert_eq!(data["headless"], json!(false));

    let read_back = tab.get_bounds().unwrap();
    assert_eq!(read_back.width as u32, 1024);
    assert_eq!(read_back.height as u32, 768);
    assert_eq!(read_back.left, 60);
    assert_eq!(read_back.top, 80);
    assert_eq!(
        runtime.window_bounds().map(|bounds| bounds.width),
        Some(1024)
    );

    let partial = execute_steps_with_runtime(
        &mut runtime,
        &[BrowserStep::SetWindowBounds {
            x: None,
            y: None,
            width: Some(800),
            height: None,
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(partial.ok, "partial set_window_bounds failed: {partial:?}");
    let after_partial = tab.get_bounds().unwrap();
    assert_eq!(after_partial.width as u32, 800);
    assert_eq!(after_partial.height as u32, 768);
    assert_eq!(after_partial.left, 60);
}

struct ClockCase {
    _profile: TempDir,
    _server: FixtureServer,
    runtime: Arc<tokio::sync::Mutex<BrowserRuntime>>,
}

fn clock_request(steps: Vec<BrowserStep>) -> BrowserActionRequest {
    BrowserActionRequest {
        session: SessionPolicy::SharedDefault,
        target: TabTarget::Active,
        attach_screenshot: None,
        page_context: None,
        network: NetworkReportMode::default(),
        steps,
        block_service_workers: None,
    }
}

fn clock_time(text: &str) -> ClockTime {
    ClockTime::Text(text.to_string())
}

fn probe_state() -> BrowserStep {
    BrowserStep::Eval {
        expression: "window.__state()".to_string(),
    }
}

fn last_value(
    report: &refact_lsp::integrations::browser_models::ExecutionReport,
) -> serde_json::Value {
    let raw = report
        .steps
        .last()
        .and_then(|step| step.data.as_ref())
        .and_then(|data| data.get("value"))
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    serde_json::from_str(raw).unwrap_or_else(|error| {
        panic!("clock probe did not return JSON ({error}): {raw:?} in {report:?}")
    })
}

impl ClockCase {
    async fn start() -> Option<Self> {
        let mut case = BrowserCase::start("clock.html").await?;
        case.setup_world();
        let page_url = case.server.url("clock.html");
        let this = Self {
            _profile: case._profile,
            _server: case.server,
            runtime: Arc::new(tokio::sync::Mutex::new(case.runtime)),
        };
        let report = this
            .run(vec![
                BrowserStep::ClockInstall {
                    time: Some(clock_time("2020-02-02T00:00:00Z")),
                },
                BrowserStep::Navigate {
                    url: page_url,
                    timeout_ms: None,
                },
                BrowserStep::ClockPauseAt {
                    time: clock_time("2020-02-02T00:10:00Z"),
                },
                BrowserStep::Eval {
                    expression: "window.__reset()".to_string(),
                },
                probe_state(),
            ])
            .await;
        assert!(report.ok, "clock setup failed: {report:?}");
        let state = last_value(&report);
        assert_eq!(state["isFake"], json!(true), "clock was not installed");
        assert_eq!(state["interval"], json!(0), "counters were not reset");
        assert_eq!(
            state["now"],
            json!(1_580_602_200_000i64),
            "pause_at did not land on the requested instant"
        );
        Some(this)
    }

    async fn run(
        &self,
        steps: Vec<BrowserStep>,
    ) -> refact_lsp::integrations::browser_models::ExecutionReport {
        execute_request_with_runtime(
            self.runtime.clone(),
            clock_request(steps),
            &ImagePolicy::browser_capture(),
        )
        .await
        .unwrap()
    }
}

fn decode_step_image(
    report: &refact_lsp::integrations::browser_models::ExecutionReport,
) -> Vec<u8> {
    let data = report.steps[0]
        .data
        .as_ref()
        .expect("step must return data");
    let encoded = data["images"][0]["data"]
        .as_str()
        .expect("step must return an image");
    base64::prelude::BASE64_STANDARD.decode(encoded).unwrap()
}

fn dominant_color(bytes: &[u8]) -> (u8, u8, u8) {
    let image = image::load_from_memory(bytes).unwrap().to_rgb8();
    let mut counts: BTreeMap<(u8, u8, u8), usize> = BTreeMap::new();
    for pixel in image.pixels() {
        *counts.entry((pixel[0], pixel[1], pixel[2])).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(color, _)| color)
        .unwrap()
}

fn channel_leader(color: (u8, u8, u8)) -> &'static str {
    if color.0 > color.1 && color.0 > color.2 {
        "red"
    } else if color.1 > color.0 && color.1 > color.2 {
        "green"
    } else if color.2 > color.0 && color.2 > color.1 {
        "blue"
    } else if color.0 > 128 && color.1 > 128 && color.2 < 128 {
        "yellow"
    } else {
        "neutral"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn screencast_sessions_compose_a_filmstrip_on_stop() {
    let Some(mut case) = BrowserCase::start("animation.html").await else {
        return;
    };
    let policy = ImagePolicy::browser_capture();

    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::ScreencastStart {
                quality: Some(70),
                max_width: Some(640),
                max_height: Some(480),
            },
            BrowserStep::Eval {
                expression: "playAnimation()".to_string(),
            },
            BrowserStep::WaitSeconds { seconds: 1.5 },
            BrowserStep::ScreencastStop {
                compose: Some(true),
            },
        ],
        &policy,
    );

    assert!(report.ok, "screencast session failed: {report:?}");
    assert!(report.steps[0].summary.starts_with("Started screencast"));
    let data = report.steps[3].data.as_ref().unwrap();
    assert_eq!(data["artifact"]["kind"], json!("filmstrip"));
    assert!(data["frame_count"].as_u64().is_some_and(|count| count >= 2));

    let already_stopped = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::ScreencastStop { compose: None }],
        &policy,
    );
    assert!(!already_stopped.ok);
    assert_eq!(
        already_stopped.steps[0].error.as_deref(),
        Some("No screencast session is running")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn clock_fast_forward_fires_each_due_timer_at_most_once() {
    let Some(case) = ClockCase::start().await else {
        return;
    };

    let report = case
        .run(vec![
            BrowserStep::ClockFastForward {
                ticks: ClockTicks::Human("01:00".to_string()),
            },
            probe_state(),
        ])
        .await;
    assert!(report.ok, "fast_forward failed: {report:?}");

    let state = last_value(&report);
    assert_eq!(state["interval"], json!(1), "interval fired more than once");
    assert_eq!(
        state["chain"],
        json!(1),
        "timeout chain advanced past one link"
    );
    assert_eq!(
        state["timeouts"],
        json!([1000, 2000, 3000]),
        "independent timeouts did not each fire once"
    );
    assert_eq!(state["now"], json!(1_580_602_260_000i64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn screenshot_mask_hides_a_fixture_element_and_restores_the_dom() {
    let Some(case) = BrowserCase::start("visual-states.html").await else {
        return;
    };

    let unmasked = execute_steps(
        &case.tab,
        &[BrowserStep::ScreenshotElement {
            locator: BrowserLocator::css("#secret"),
            options: BrowserScreenshotOptions::default(),
        }],
    );
    assert!(unmasked.ok, "unmasked capture failed: {unmasked:?}");

    let masked = execute_steps(
        &case.tab,
        &[BrowserStep::ScreenshotElement {
            locator: BrowserLocator::css("#secret"),
            options: BrowserScreenshotOptions {
                mask: vec![BrowserLocator::css("#secret")],
                mask_color: Some("#FF00FF".to_string()),
                ..Default::default()
            },
        }],
    );
    assert!(masked.ok, "masked capture failed: {masked:?}");

    let unmasked_data = unmasked.steps[0].data.as_ref().unwrap();
    let masked_data = masked.steps[0].data.as_ref().unwrap();
    let unmasked_bytes = base64::prelude::BASE64_STANDARD
        .decode(unmasked_data["data"].as_str().unwrap())
        .unwrap();
    let masked_bytes = base64::prelude::BASE64_STANDARD
        .decode(masked_data["data"].as_str().unwrap())
        .unwrap();

    assert_eq!(dominant_color(&unmasked_bytes), (0, 0, 128));
    assert_eq!(dominant_color(&masked_bytes), (255, 0, 255));

    let leftovers = case
        .tab
        .evaluate(
            "document.querySelectorAll('[data-refact-screenshot-mask],[data-refact-screenshot]').length",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(leftovers, json!(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn set_window_bounds_on_headless_succeeds_without_applying() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let profile = tempdir().unwrap();
    let mut runtime = launch_browser(&profile);
    let tab = runtime.browser.new_tab().unwrap();
    runtime.set_active_tab_target_id(tab.get_target_id().to_string());

    let report = execute_steps_with_runtime(
        &mut runtime,
        &[BrowserStep::SetWindowBounds {
            x: None,
            y: None,
            width: Some(1024),
            height: Some(768),
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "headless set_window_bounds must not fail");
    let step = &report.steps[0];
    assert!(step.summary.contains("headless"));
    assert!(step.summary.contains("set_viewport"));
    let data = step.data.as_ref().unwrap();
    assert_eq!(data["applied"], json!(false));
    assert_eq!(data["headless"], json!(true));
    assert!(runtime.window_bounds().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn clock_run_for_fires_every_callback_along_the_way() {
    let Some(case) = ClockCase::start().await else {
        return;
    };

    let report = case
        .run(vec![
            BrowserStep::ClockRunFor {
                ticks: ClockTicks::Human("01:00".to_string()),
            },
            probe_state(),
        ])
        .await;
    assert!(report.ok, "run_for failed: {report:?}");

    let state = last_value(&report);
    assert_eq!(
        state["interval"],
        json!(60),
        "interval did not fire per second"
    );
    assert_eq!(
        state["chain"],
        json!(60),
        "timeout chain did not keep chaining"
    );
    assert_eq!(state["timeouts"], json!([1000, 2000, 3000]));
    assert_eq!(state["now"], json!(1_580_602_260_000i64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn hover_state_capture_differs_from_the_default_state_capture() {
    let Some(case) = BrowserCase::start("visual-states.html").await else {
        return;
    };

    let default_only = execute_steps(
        &case.tab,
        &[BrowserStep::CaptureElementStates {
            locator: BrowserLocator::css("#swatch"),
            states: vec![BrowserElementState::Default],
            labels: Some(false),
            options: BrowserScreenshotOptions::default(),
        }],
    );
    assert!(default_only.ok, "default capture failed: {default_only:?}");

    let hover_only = execute_steps(
        &case.tab,
        &[BrowserStep::CaptureElementStates {
            locator: BrowserLocator::css("#swatch"),
            states: vec![BrowserElementState::Hover],
            labels: Some(false),
            options: BrowserScreenshotOptions::default(),
        }],
    );
    assert!(hover_only.ok, "hover capture failed: {hover_only:?}");

    let default_bytes = decode_step_image(&default_only);
    let hover_bytes = decode_step_image(&hover_only);
    assert_eq!(channel_leader(dominant_color(&default_bytes)), "blue");
    assert_eq!(channel_leader(dominant_color(&hover_bytes)), "green");
    assert_ne!(default_bytes, hover_bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn clock_set_fixed_time_freezes_date_while_timers_keep_running() {
    let Some(case) = ClockCase::start().await else {
        return;
    };

    let report = case
        .run(vec![
            BrowserStep::ClockSetFixedTime {
                time: clock_time("2021-03-03T00:00:00Z"),
            },
            BrowserStep::ClockRunFor {
                ticks: ClockTicks::Millis(3000),
            },
            probe_state(),
        ])
        .await;
    assert!(report.ok, "set_fixed_time failed: {report:?}");

    let state = last_value(&report);
    assert_eq!(
        state["interval"],
        json!(3),
        "timers stopped under a fixed time"
    );
    assert_eq!(
        state["dates"],
        json!([
            1_614_729_600_000i64,
            1_614_729_600_000i64,
            1_614_729_600_000i64
        ]),
        "Date.now moved while the clock was fixed"
    );
    assert_eq!(state["now"], json!(1_614_729_600_000i64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn element_state_strip_captures_every_state_and_returns_the_element_to_rest() {
    let Some(case) = BrowserCase::start("visual-states.html").await else {
        return;
    };

    let report = execute_steps(
        &case.tab,
        &[BrowserStep::CaptureElementStates {
            locator: BrowserLocator::css("#swatch"),
            states: Vec::new(),
            labels: Some(true),
            options: BrowserScreenshotOptions::default(),
        }],
    );
    assert!(report.ok, "state strip failed: {report:?}");

    let data = report.steps[0].data.as_ref().unwrap();
    assert_eq!(
        data["states"],
        json!(["default", "hover", "focus", "active"])
    );
    assert_eq!(data["images"].as_array().unwrap().len(), 1);

    let strip = image::load_from_memory(&decode_step_image(&report)).unwrap();
    assert!(strip.width() > strip.height());

    let resting = case
        .tab
        .evaluate(
            "document.activeElement === document.body || document.activeElement === document.documentElement",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(resting, json!(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn clock_set_system_time_shifts_time_without_firing_timers() {
    let Some(case) = ClockCase::start().await else {
        return;
    };

    let report = case
        .run(vec![
            BrowserStep::ClockSetSystemTime {
                time: ClockTime::UnixMillis(1_614_729_600_000),
            },
            probe_state(),
        ])
        .await;
    assert!(report.ok, "set_system_time failed: {report:?}");

    let state = last_value(&report);
    assert_eq!(state["now"], json!(1_614_729_600_000i64));
    assert_eq!(state["interval"], json!(0), "set_system_time fired timers");
    assert_eq!(
        state["chain"],
        json!(0),
        "set_system_time advanced the chain"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn clock_resume_restarts_real_time_and_reset_restores_native_timers() {
    let Some(case) = ClockCase::start().await else {
        return;
    };

    let resumed = case
        .run(vec![
            BrowserStep::ClockResume,
            BrowserStep::WaitSeconds { seconds: 1.5 },
            probe_state(),
        ])
        .await;
    assert!(resumed.ok, "resume failed: {resumed:?}");
    let state = last_value(&resumed);
    assert!(
        state["interval"].as_i64().unwrap_or_default() >= 1,
        "resumed clock did not fire the interval: {state}"
    );
    assert!(state["now"].as_i64().unwrap_or_default() > 1_580_602_200_000);

    let after_reset = case.run(vec![BrowserStep::Reset, probe_state()]).await;
    assert!(after_reset.ok, "reset failed: {after_reset:?}");
    assert_eq!(
        after_reset.steps[0].data.as_ref().unwrap()["reset"]["clock_cleared"],
        json!(true)
    );
    assert_eq!(
        last_value(&after_reset)["isFake"],
        json!(false),
        "reset left the fake Date installed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn clock_steps_report_clear_errors_when_used_out_of_order() {
    let Some(mut case) = BrowserCase::start("clock.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let early = execute_request_with_runtime(
        runtime.clone(),
        clock_request(vec![BrowserStep::ClockFastForward {
            ticks: ClockTicks::Millis(1000),
        }]),
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(!early.ok);
    assert!(
        early.steps[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("clock_install"),
        "{early:?}"
    );

    let unpaused = execute_request_with_runtime(
        runtime,
        clock_request(vec![
            BrowserStep::ClockInstall { time: None },
            BrowserStep::ClockResume,
        ]),
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(!unpaused.ok);
    assert!(
        unpaused.steps[1]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("clock_pause_at"),
        "{unpaused:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn element_gallery_composes_a_grid_or_returns_separate_captures() {
    let Some(case) = BrowserCase::start("visual-states.html").await else {
        return;
    };
    let locators = vec![
        BrowserLocator::css("#secret"),
        BrowserLocator::css("#plain"),
    ];

    let separate = execute_steps(
        &case.tab,
        &[BrowserStep::ScreenshotElements {
            locators: locators.clone(),
            compose: BrowserComposeMode::Separate,
            labels: Some(false),
            options: BrowserScreenshotOptions::default(),
        }],
    );
    assert!(separate.ok, "separate gallery failed: {separate:?}");
    let separate_images = separate.steps[0].data.as_ref().unwrap()["images"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(separate_images.len(), 2);
    let first = base64::prelude::BASE64_STANDARD
        .decode(separate_images[0]["data"].as_str().unwrap())
        .unwrap();
    let second = base64::prelude::BASE64_STANDARD
        .decode(separate_images[1]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(dominant_color(&first), (0, 0, 128));
    assert_eq!(dominant_color(&second), (128, 128, 128));

    let grid = execute_steps(
        &case.tab,
        &[BrowserStep::ScreenshotElements {
            locators,
            compose: BrowserComposeMode::Grid,
            labels: Some(true),
            options: BrowserScreenshotOptions::default(),
        }],
    );
    assert!(grid.ok, "grid gallery failed: {grid:?}");
    let grid_data = grid.steps[0].data.as_ref().unwrap();
    assert_eq!(grid_data["count"], json!(2));
    assert_eq!(grid_data["images"].as_array().unwrap().len(), 1);
    assert_eq!(grid_data["labels"], json!(["css=#secret", "css=#plain"]));

    let sheet = image::load_from_memory(&decode_step_image(&grid)).unwrap();
    assert!(sheet.width() > 120 && sheet.height() > 60);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn screenshot_style_pierces_shadow_dom() {
    let Some(case) = BrowserCase::start("shadow-dom.html").await else {
        return;
    };

    let report = execute_steps(
        &case.tab,
        &[BrowserStep::Screenshot {
            options: BrowserScreenshotOptions {
                style: Some("* { visibility: hidden !important }".to_string()),
                ..Default::default()
            },
        }],
    );
    assert!(report.ok, "styled capture failed: {report:?}");

    let injected = case
        .tab
        .evaluate(
            "Array.from(document.querySelectorAll('*')).filter(el => el.shadowRoot).every(el => el.shadowRoot.querySelector('style[data-refact-screenshot]') === null)",
            false,
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(injected, json!(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn set_content_replaces_the_document_and_is_visible_in_the_next_snapshot() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: Some(false),
            network: NetworkReportMode::default(),
            block_service_workers: None,
            steps: vec![
                BrowserStep::SetContent {
                    html: "<!doctype html><html><body><h1>Fixture-less heading</h1><button>Press me</button></body></html>"
                        .to_string(),
                    wait_until: None,
                },
                BrowserStep::AccessibilitySnapshot {
                    options: AccessibilitySnapshotOptions::default(),
                },
                BrowserStep::PageContent,
            ],
            page_context: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "set_content batch failed: {report:?}");
    let snapshot = serde_json::to_string(&report.steps[1].data).unwrap();
    assert!(
        snapshot.contains("Fixture-less heading") && snapshot.contains("Press me"),
        "snapshot did not observe the new document: {snapshot}"
    );
    let content = report.steps[2]
        .data
        .as_ref()
        .and_then(|data| data.get("html"))
        .and_then(|html| html.as_str())
        .unwrap_or_default();
    assert!(content.starts_with("<!DOCTYPE html>"), "{content}");
    assert!(content.contains("Fixture-less heading"), "{content}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn cdp_send_reaches_page_and_browser_targets_and_keeps_overrides() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: Some(false),
            network: NetworkReportMode::default(),
            block_service_workers: None,
            page_context: None,
            steps: vec![
                BrowserStep::CdpSend {
                    method: "Runtime.evaluate".to_string(),
                    params: Some(json!({"expression": "40 + 2", "returnByValue": true})),
                    target: CdpTarget::Page,
                },
                BrowserStep::CdpSend {
                    method: "Browser.getVersion".to_string(),
                    params: None,
                    target: CdpTarget::Browser,
                },
                BrowserStep::CdpSend {
                    method: "Emulation.setDeviceMetricsOverride".to_string(),
                    params: Some(json!({
                        "width": 500,
                        "height": 400,
                        "deviceScaleFactor": 1,
                        "mobile": false
                    })),
                    target: CdpTarget::Page,
                },
                BrowserStep::Eval {
                    expression: "window.innerWidth".to_string(),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "cdp_send batch failed: {report:?}");

    let evaluated = report.steps[0].data.as_ref().unwrap();
    assert_eq!(evaluated["cdp_send"]["result"]["result"]["value"], 42);
    assert_eq!(evaluated["cdp_send"]["target"], "page");
    assert_eq!(evaluated["cdp_send"]["method"], "Runtime.evaluate");
    assert!(
        evaluated["cdp_send"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty(),
        "Runtime.evaluate must not warn: {evaluated}"
    );

    let version = report.steps[1].data.as_ref().unwrap();
    assert_eq!(version["cdp_send"]["target"], "browser");
    assert!(
        version["cdp_send"]["result"]["product"]
            .as_str()
            .is_some_and(|product| !product.is_empty()),
        "Browser.getVersion must reach the browser target: {version}"
    );

    let warned = report.steps[2].data.as_ref().unwrap();
    let warnings = warned["cdp_send"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "expected one warning: {warned}");
    assert!(warnings[0].as_str().unwrap().contains("reset"));

    assert_eq!(
        report.steps[3].data.as_ref().unwrap()["value"],
        500,
        "the long-lived cdp session must keep its overrides applied"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn init_script_survives_navigation_and_clears_on_remove_and_reset() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let server_url = case.server.url("readouts.html");
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let added = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: Some(false),
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::AddInitScript {
                    content: "window.__refact_init_flag = 'installed';".to_string(),
                },
                BrowserStep::Navigate {
                    url: server_url.clone(),
                    timeout_ms: None,
                },
                BrowserStep::Eval {
                    expression: "window.__refact_init_flag || 'missing'".to_string(),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(added.ok, "init script batch failed: {added:?}");
    let id = added.steps[0]
        .data
        .as_ref()
        .and_then(|data| data.get("id"))
        .and_then(|id| id.as_str())
        .expect("add_init_script must mint an id")
        .to_string();
    assert_eq!(returned_eval_string(&added), "installed");

    let removed = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: Some(false),
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::RemoveInitScript { id },
                BrowserStep::Navigate {
                    url: server_url.clone(),
                    timeout_ms: None,
                },
                BrowserStep::Eval {
                    expression: "window.__refact_init_flag || 'missing'".to_string(),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(removed.ok, "remove_init_script batch failed: {removed:?}");
    assert_eq!(returned_eval_string(&removed), "missing");

    let reset = execute_request_with_runtime(
        runtime,
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: Some(false),
            page_context: None,
            network: NetworkReportMode::default(),
            steps: vec![
                BrowserStep::AddInitScript {
                    content: "window.__refact_init_flag = 'reinstalled';".to_string(),
                },
                BrowserStep::Reset,
                BrowserStep::Navigate {
                    url: server_url,
                    timeout_ms: None,
                },
                BrowserStep::Eval {
                    expression: "window.__refact_init_flag || 'missing'".to_string(),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    assert!(reset.ok, "reset batch failed: {reset:?}");
    assert_eq!(
        reset.steps[1]
            .data
            .as_ref()
            .and_then(|data| data.pointer("/reset/init_scripts"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(returned_eval_string(&reset), "missing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn add_style_tag_changes_computed_style_and_add_script_tag_runs() {
    let Some(case) = BrowserCase::start("dispatch-event.html").await else {
        return;
    };

    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::AddStyleTag {
                url: None,
                content: Some("#target { color: rgb(1, 2, 3); }".to_string()),
            },
            BrowserStep::AddScriptTag {
                url: None,
                content: Some("window.__refact_script_tag = 'ran';".to_string()),
                script_type: None,
            },
            BrowserStep::Eval {
                expression:
                    "getComputedStyle(document.getElementById('target')).color + '|' + window.__refact_script_tag"
                        .to_string(),
            },
        ],
    );

    assert!(report.ok, "tag injection failed: {report:?}");
    assert_eq!(returned_eval_string(&report), "rgb(1, 2, 3)|ran");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn dispatch_event_fires_listeners_with_the_inferred_class_and_detail() {
    let Some(case) = BrowserCase::start("dispatch-event.html").await else {
        return;
    };

    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::DispatchEvent {
                locator: BrowserLocator::css("#target"),
                event_type: "click".to_string(),
                event_init: None,
            },
            BrowserStep::DispatchEvent {
                locator: BrowserLocator::css("#target"),
                event_type: "keydown".to_string(),
                event_init: Some(json!({"key": "Enter"})),
            },
            BrowserStep::DispatchEvent {
                locator: BrowserLocator::css("#target"),
                event_type: "app:custom".to_string(),
                event_init: Some(json!({"detail": {"id": 7}})),
            },
            BrowserStep::Eval {
                expression: "JSON.stringify(window.__events)".to_string(),
            },
        ],
    );

    assert!(report.ok, "dispatch_event batch failed: {report:?}");
    let events: serde_json::Value =
        serde_json::from_str(returned_eval_string(&report)).expect("listener log must be JSON");

    assert_eq!(events[0]["constructor"], json!("MouseEvent"));
    assert_eq!(events[0]["bubbles"], json!(true));
    assert_eq!(events[0]["cancelable"], json!(true));
    assert_eq!(events[0]["composed"], json!(true));

    assert_eq!(events[1]["constructor"], json!("KeyboardEvent"));
    assert_eq!(events[1]["key"], json!("Enter"));

    assert_eq!(events[2]["constructor"], json!("CustomEvent"));
    assert_eq!(events[2]["detail"], json!({"id": 7}));

    let bubbled = case
        .tab
        .evaluate("window.__bubbledToBody === true", false)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(bubbled, json!(true));
    let own_target_id = case.tab.get_target_id().to_string();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));

    let send = |method: &str, params: Option<serde_json::Value>, target: CdpTarget| {
        let runtime = runtime.clone();
        let method = method.to_string();
        async move {
            execute_request_with_runtime(
                runtime,
                BrowserActionRequest {
                    session: SessionPolicy::SharedDefault,
                    target: TabTarget::Active,
                    attach_screenshot: Some(false),
                    network: NetworkReportMode::default(),
                    steps: vec![BrowserStep::CdpSend {
                        method,
                        params,
                        target,
                    }],
                    page_context: None,
                    block_service_workers: None,
                },
                &ImagePolicy::browser_capture(),
            )
            .await
            .unwrap()
        }
    };

    let closed = send("Browser.close", None, CdpTarget::Browser).await;
    assert!(!closed.ok, "Browser.close must be refused: {closed:?}");
    assert!(
        closed.steps[0]
            .error
            .as_ref()
            .unwrap()
            .contains("shut down the shared browser"),
        "unexpected denial: {:?}",
        closed.steps[0]
    );

    let self_close = send(
        "Target.closeTarget",
        Some(json!({"targetId": own_target_id})),
        CdpTarget::Browser,
    )
    .await;
    assert!(!self_close.ok, "self-close must be refused: {self_close:?}");
    assert!(self_close.steps[0]
        .error
        .as_ref()
        .unwrap()
        .contains(&own_target_id));

    let unknown = send("Nope.doesNotExist", None, CdpTarget::Page).await;
    assert!(!unknown.ok, "unknown methods must fail: {unknown:?}");
    let error = unknown.steps[0].error.as_ref().unwrap();
    assert!(
        !error.contains('\n'),
        "CDP errors must be one line: {error}"
    );
    assert!(error.contains("not found") || error.contains("wasn't found"));

    let alive = send("Browser.getVersion", None, CdpTarget::Browser).await;
    assert!(
        alive.ok,
        "the browser must survive every refused call: {alive:?}"
    );
}

struct DesignToolCase {
    ccx: Arc<tokio::sync::Mutex<refact_lsp::at_commands::at_commands::AtCommandsContext>>,
    _profile: TempDir,
    _cache_dir: TempDir,
    _config_dir: TempDir,
    _server: FixtureServer,
    _tab: Arc<Tab>,
}

async fn design_tool_context(case: BrowserCase, chat_id: &str) -> DesignToolCase {
    let mut case = case;
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
    let app = refact_lsp::app_state::AppState::from_gcx(gcx).await;
    case.runtime.reattach(chat_id);
    refact_lsp::integrations::browser_runtime::register_browser_runtime(app.clone(), case.runtime)
        .await;
    DesignToolCase {
        ccx: Arc::new(tokio::sync::Mutex::new(
            refact_lsp::at_commands::at_commands::AtCommandsContext::new_from_app(
                app,
                4096,
                10,
                false,
                Vec::new(),
                chat_id.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        )),
        _profile: case._profile,
        _cache_dir: cache_dir,
        _config_dir: config_dir,
        _server: case.server,
        _tab: case.tab,
    }
}

fn design_tool_payload(
    messages: Vec<refact_lsp::call_validation::ContextEnum>,
) -> serde_json::Value {
    let refact_lsp::call_validation::ContextEnum::ChatMessage(message) = messages
        .into_iter()
        .next()
        .expect("design tool must return a message")
    else {
        panic!("expected a chat message");
    };
    let text = match message.content {
        ChatContent::SimpleText(text) => text,
        ChatContent::Multimodal(elements) => {
            elements
                .into_iter()
                .find(|element| element.m_type == "text")
                .expect("design tool must return a text element")
                .m_content
        }
        ChatContent::ContextFiles(_) => panic!("expected text or multimodal content"),
    };
    serde_json::from_str(&text).expect("design tools must return ToolJson")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn design_tools_drive_element_states_through_the_shared_browser_session() {
    let Some(mut case) = BrowserCase::start("visual-states.html").await else {
        return;
    };
    case.setup_world();
    let case = design_tool_context(case, "design-states-e2e").await;
    let ccx = case.ccx.clone();

    let mut probe = refact_lsp::tools::design_tools::ToolUiProbe {
        config_path: "builtin".to_string(),
    };
    let (_, messages) = refact_lsp::tools::tools_description::Tool::tool_execute(
        &mut probe,
        ccx,
        &"design-probe".to_string(),
        &std::collections::HashMap::from([
            ("targets".to_string(), json!(["#swatch"])),
            (
                "viewports".to_string(),
                json!([{"width": 800, "height": 600}]),
            ),
            ("themes".to_string(), json!(["light"])),
            (
                "states".to_string(),
                json!(["default", "hover", "focus", "active"]),
            ),
            ("properties".to_string(), json!(["background-color"])),
        ]),
    )
    .await
    .expect("ui_probe must succeed on the instrumented fixture");

    let payload = design_tool_payload(messages);
    let matrix = payload["matrix"].as_array().expect("matrix array");
    assert_eq!(matrix.len(), 4, "one cell per requested state: {matrix:?}");
    let measured = matrix
        .iter()
        .map(|cell| {
            (
                cell["state"].as_str().unwrap().to_string(),
                cell["styles"]["background-color"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        measured,
        vec![
            ("default".to_string(), "rgb(0, 0, 255)".to_string()),
            ("hover".to_string(), "rgb(0, 255, 0)".to_string()),
            ("focus".to_string(), "rgb(255, 255, 0)".to_string()),
            ("active".to_string(), "rgb(255, 0, 0)".to_string()),
        ],
        "each state must be driven through the shared element-state sequencer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn design_tools_scope_the_aria_snapshot_and_compose_with_refs() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let case = design_tool_context(case, "design-snapshot-e2e").await;
    let ccx = case.ccx.clone();

    let mut mark = refact_lsp::tools::design_tools::ToolMarkElements {
        config_path: "builtin".to_string(),
    };
    let (_, messages) = refact_lsp::tools::tools_description::Tool::tool_execute(
        &mut mark,
        ccx.clone(),
        &"design-mark-all".to_string(),
        &std::collections::HashMap::new(),
    )
    .await
    .expect("mark_elements must succeed on the instrumented fixture");
    let whole_page = design_tool_payload(messages);
    let whole_page_marks = whole_page["marks"].as_array().expect("marks array").len();
    assert!(whole_page_marks > 0, "the page must expose marked elements");

    let (_, messages) = refact_lsp::tools::tools_description::Tool::tool_execute(
        &mut mark,
        ccx.clone(),
        &"design-mark-scoped".to_string(),
        &std::collections::HashMap::from([("selector".to_string(), json!("main"))]),
    )
    .await
    .expect("scoped mark_elements must succeed");
    let scoped = design_tool_payload(messages);
    let scoped_marks = scoped["marks"].as_array().expect("marks array");
    assert!(
        scoped_marks.len() <= whole_page_marks,
        "a scoped snapshot must not exceed the whole-page snapshot"
    );

    let reference = scoped_marks
        .first()
        .expect("a scoped subtree must expose at least one ref")["ref"]
        .as_str()
        .expect("ref string")
        .to_string();
    let mut probe = refact_lsp::tools::design_tools::ToolUiProbe {
        config_path: "builtin".to_string(),
    };
    let (_, messages) = refact_lsp::tools::tools_description::Tool::tool_execute(
        &mut probe,
        ccx,
        &"design-probe-ref".to_string(),
        &std::collections::HashMap::from([
            ("targets".to_string(), json!([reference])),
            (
                "viewports".to_string(),
                json!([{"width": 800, "height": 600}]),
            ),
            ("themes".to_string(), json!(["light"])),
            ("states".to_string(), json!(["default"])),
        ]),
    )
    .await
    .expect("a ref emitted by mark_elements must compose as a ui_probe target");
    let payload = design_tool_payload(messages);
    assert_eq!(payload["matrix"].as_array().unwrap().len(), 1);
}
