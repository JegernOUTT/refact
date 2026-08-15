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
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{Page, Runtime};
use hyper::body::Bytes;
use refact_core::image_policy::ImagePolicy;
use refact_lsp::integrations::browser_controller::execute_steps as execute_steps_with_policy;
use refact_lsp::integrations::browser_models::{BrowserLocator, BrowserStep};
use refact_lsp::refact_browser::{BrowserRuntime, UTILITY_WORLD_NAME};
use serde::Deserialize;
use serde_json::json;
use tempfile::{tempdir, TempDir};

const FIXTURE_PAGES: &[&str] = &[
    "delayed-button.html",
    "overlay.html",
    "moving-target.html",
    "controlled-input.html",
    "iframe-form.html",
    "shadow-dom.html",
    "dialog.html",
    "fetch-after-click.html",
    "popup.html",
    "upload.html",
    "download.html",
    "contenteditable.html",
    "hover-menu.html",
    "strict-multi.html",
    "hostile-globals.html",
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
        let report = execute_steps(
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
        let report = execute_steps(
            &self.tab,
            &[BrowserStep::Navigate {
                url: self.server.url(page),
            }],
        );
        assert!(report.ok, "navigation failed: {report:?}");
    }

    fn setup_world(&mut self) {
        refact_lsp::refact_browser::setup_recording_for_tab(&mut self.runtime, &self.tab).unwrap();
    }

    fn call_version(&self) -> serde_json::Value {
        self.runtime
            .world_manager
            .call_injected(&self.tab, "version", json!([]))
            .unwrap()
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

#[tokio::test]
async fn fixture_server_starts_and_serves_page() {
    let server = FixtureServer::start().await.unwrap();
    let response = reqwest::get(server.url("delayed-button.html"))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
    assert!(response.text().await.unwrap().contains("Delayed button"));
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
    let report = execute_steps(
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
async fn click_obscured_waits_for_overlay() {
    let Some(case) = BrowserCase::start("overlay.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#target"),
            },
            BrowserStep::WaitForText {
                text: "clicked after overlay".to_string(),
                timeout_ms: Some(2_000),
            },
        ],
    );
    assert!(
        report.ok,
        "click should wait for overlay removal: {report:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fill_controlled_input_react() {
    let Some(case) = BrowserCase::start("controlled-input.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::css("#controlled"),
                text: "typed by browser".to_string(),
                clear_first: true,
                verify: true,
            },
            text_step("#state"),
        ],
    );
    assert!(
        report.ok,
        "controlled fill should use trusted input: {report:?}"
    );
    assert_eq!(returned_text(&report), "typed by browser");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn hover_reveals_css_menu() {
    let Some(case) = BrowserCase::start("hover-menu.html").await else {
        return;
    };
    let report = execute_steps(
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
async fn strict_multi_click_errors() {
    let Some(case) = BrowserCase::start("strict-multi.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[BrowserStep::Click {
            locator: BrowserLocator::css(".duplicate"),
        }],
    );
    assert!(
        !report.ok,
        "strict click must reject multiple matches: {report:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn moving_target_waits_until_stable() {
    let Some(case) = BrowserCase::start("moving-target.html").await else {
        return;
    };
    let report = execute_steps(
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
async fn contenteditable_fill_updates_output() {
    let Some(case) = BrowserCase::start("contenteditable.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::css("#editor"),
                text: "editable text".to_string(),
                clear_first: true,
                verify: true,
            },
            text_step("#result"),
        ],
    );
    assert!(report.ok, "contenteditable fill failed: {report:?}");
    assert_eq!(returned_text(&report), "editable text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn fetch_after_click_renders_slow_echo() {
    let Some(case) = BrowserCase::start("fetch-after-click.html").await else {
        return;
    };
    let report = execute_steps(
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
async fn popup_click_opens_second_tab() {
    let Some(case) = BrowserCase::start("popup.html").await else {
        return;
    };
    let before = case.runtime.browser.get_tabs().lock().unwrap().len();
    let report = execute_steps(
        &case.tab,
        &[BrowserStep::Click {
            locator: BrowserLocator::css("#open"),
        }],
    );
    assert!(report.ok, "popup click failed: {report:?}");
    std::thread::sleep(Duration::from_millis(300));
    let after = case.runtime.browser.get_tabs().lock().unwrap().len();
    assert_eq!(after, before + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn iframe_form_submit_reaches_same_origin_frame() {
    let Some(case) = BrowserCase::start("iframe-form.html").await else {
        return;
    };
    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::css("#frame-name"),
                text: "Frame User".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#frame-submit"),
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
async fn nested_shadow_dom_button_and_input_are_actionable() {
    let Some(case) = BrowserCase::start("shadow-dom.html").await else {
        return;
    };
    let report = execute_steps(
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
async fn dialog_fixture_records_confirmed_result() {
    let Some(case) = BrowserCase::start("dialog.html").await else {
        return;
    };
    case.tab
        .evaluate("window.confirm = () => true", false)
        .unwrap();
    let report = execute_steps(
        &case.tab,
        &[
            BrowserStep::Click {
                locator: BrowserLocator::css("#confirm"),
            },
            text_step("#result"),
        ],
    );
    assert!(report.ok, "dialog fixture click failed: {report:?}");
    assert_eq!(returned_text(&report), "confirmed");
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
