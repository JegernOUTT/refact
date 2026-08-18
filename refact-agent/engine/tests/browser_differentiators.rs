use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;

use headless_chrome::protocol::cdp::{Page, Runtime};
use headless_chrome::Tab;
use refact_core::image_policy::ImagePolicy;
use refact_lsp::integrations::browser_controller::{
    execute_request_with_runtime, execute_steps_with_runtime,
};
use refact_lsp::integrations::browser_locators::{parse_element_info, INSPECT_ELEMENT_JS};
use refact_lsp::integrations::browser_models::{
    AccessibilitySnapshotOptions, BrowserActionRequest, BrowserAuthenticatorProtocol,
    BrowserAuthenticatorTransport, BrowserLocator, BrowserStep, ElementInfo, FieldKind,
    FillStrategy, HarContentPolicy, HarMode, HarNotFound, NetworkReportMode, SessionPolicy,
    TabTarget, UrlPattern, WebSocketEventKind, WebSocketMessageAction, WebSocketRouteMode,
};
use refact_lsp::refact_browser::{setup_recording_for_tab, BrowserRuntime, UTILITY_WORLD_NAME};
use refact_lsp::refact_integrations::browser_types::RecorderEvent;
use serde_json::{json, Value};

mod browser_common;

use browser_common::{discover_chrome, e2e_enabled, e2e_launch_options, print_skip, FixtureServer};

use tempfile::{tempdir, TempDir};

#[test]
fn websocket_inspection_and_har_replay_contracts_are_additive_and_masked() {
    let registry = refact_lsp::refact_browser::WebSocketRegistry::default();
    registry
        .add_route(
            UrlPattern::Text("wss://example.test/**".to_string()),
            WebSocketRouteMode::Mock,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        )
        .unwrap();
    registry.record_created(
        "ws-1".to_string(),
        "wss://example.test/socket?token=secret".to_string(),
    );
    registry.record_frame("ws-1", false, "password=hunter2".to_string(), 1);
    let events = registry.drain_report();
    assert_eq!(events[1].kind, WebSocketEventKind::FrameReceived);
    assert!(!events[0].url.contains("secret"));
    assert!(!events[1].data.as_deref().unwrap().contains("hunter2"));

    let temp = tempdir().unwrap();
    let recorder = refact_lsp::refact_browser::har::HarRecorder::default();
    let path = recorder
        .start(
            temp.path(),
            Some("offline.har"),
            HarMode::Full,
            HarContentPolicy::Embed,
            None,
            None,
        )
        .unwrap();
    recorder.record(
        &refact_lsp::refact_integrations::browser_types::NetworkEntry {
            method: "GET".to_string(),
            url: "https://example.test/page?token=secret".to_string(),
            status: Some(200),
            status_text: Some("OK".to_string()),
            ..Default::default()
        },
        Some(refact_lsp::refact_browser::har::normalize_response_body(
            "<h1>password=hunter2</h1>".to_string(),
            false,
            Some("text/html".to_string()),
        )),
    );
    let summary = recorder.stop().unwrap();
    assert_eq!(summary.entry_count, 1);
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert!(!bytes.contains("hunter2"));
    let replay =
        refact_lsp::refact_browser::har::HarReplay::load(&path, None, HarNotFound::Abort).unwrap();
    assert!(matches!(
        replay.match_request("GET", "https://example.test/missing"),
        Some(refact_lsp::integrations::browser_models::RouteHandler::Abort { .. })
    ));
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
        let mut runtime = BrowserRuntime::launch(
            profile.path().to_path_buf(),
            e2e_launch_options(discover_chrome()),
        )
        .unwrap();
        let tab = runtime.browser.new_tab().unwrap();
        runtime.set_active_tab_target_id(tab.get_target_id().to_string());
        tab.navigate_to(&server.url(page)).unwrap();
        tab.wait_until_navigated().unwrap();
        Some(Self {
            runtime,
            _profile: profile,
            server,
            tab,
        })
    }

    async fn start_with_chrome(page: &str) -> Self {
        let chrome = discover_chrome().expect(
            "install Chrome, Chromium, google-chrome, or chromium-browser to run this test",
        );
        let server = FixtureServer::start().await.unwrap();
        let profile = tempdir().unwrap();
        let mut runtime = BrowserRuntime::launch(
            profile.path().to_path_buf(),
            e2e_launch_options(Some(chrome)),
        )
        .unwrap();
        let tab = runtime.browser.new_tab().unwrap();
        runtime.set_active_tab_target_id(tab.get_target_id().to_string());
        tab.navigate_to(&server.url(page)).unwrap();
        tab.wait_until_navigated().unwrap();
        Self {
            runtime,
            _profile: profile,
            server,
            tab,
        }
    }

    fn setup_world(&mut self) {
        setup_recording_for_tab(&mut self.runtime, self.tab.clone()).unwrap();
    }
}

fn eval_value(tab: &Tab, expression: &str) -> Value {
    tab.evaluate(expression, false).unwrap().value.unwrap()
}

fn eval_json(tab: &Tab, expression: &str) -> Value {
    let wrapped = format!("JSON.stringify({expression})");
    let serialized = eval_value(tab, &wrapped);
    serde_json::from_str(serialized.as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn coverage_and_virtual_authenticator_work_without_always_on_domains() {
    let Some(mut case) = BrowserCase::start("coverage-target.html").await else {
        return;
    };
    case.setup_world();
    let coverage = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::StartCoverage {
                js: Some(true),
                css: Some(true),
                reset_on_navigation: Some(false),
            },
            BrowserStep::Eval {
                expression: "window.coverageTarget".to_string(),
            },
            BrowserStep::StopCoverage,
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(coverage.ok, "coverage failed: {coverage:?}");
    let data = coverage.steps[2].data.as_ref().unwrap();
    let summaries = data["coverage"].as_array().unwrap();
    assert!(summaries.iter().any(|summary| {
        let percentage = summary["used_percentage"].as_f64().unwrap_or_default();
        percentage > 0.0 && percentage < 100.0
    }));
    assert!(FsPath::new(data["artifact"]["path"].as_str().unwrap()).is_file());

    case.tab
        .navigate_to(&case.server.url("webauthn-target.html"))
        .unwrap();
    case.tab.wait_until_navigated().unwrap();
    let webauthn = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::AddVirtualAuthenticator {
                id: (),
                protocol: Some(BrowserAuthenticatorProtocol::Ctap2),
                transport: Some(BrowserAuthenticatorTransport::Internal),
                has_resident_key: Some(true),
                has_user_verification: Some(true),
                is_user_verified: Some(true),
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#create"),
            },
            BrowserStep::WaitForText {
                text: "created:public-key".to_string(),
                timeout_ms: Some(5_000),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(webauthn.ok, "WebAuthn create failed: {webauthn:?}");
    let authenticator_id = webauthn.steps[0].data.as_ref().unwrap()["authenticator_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        webauthn.steps[0].summary.contains(&authenticator_id),
        "add result must report the minted id: {:?}",
        webauthn.steps[0].summary
    );
    let credentials = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::ListCredentials {
                id: authenticator_id.clone(),
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#get"),
            },
            BrowserStep::WaitForText {
                text: "got:public-key".to_string(),
                timeout_ms: Some(5_000),
            },
            BrowserStep::RemoveVirtualAuthenticator {
                id: authenticator_id,
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(credentials.ok, "WebAuthn get failed: {credentials:?}");
    let serialized = serde_json::to_string(&credentials.steps[0].data).unwrap();
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains("privateKey"));
}

// One batched request avoids the extra agent round trips incurred by one-action-per-call tools.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_01_multi_step_batching_returns_ordered_indexed_results() {
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
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::AccessibilitySnapshot {
                options: AccessibilitySnapshotOptions::default(),
            },
            BrowserStep::Click {
                locator: BrowserLocator::reference(&reference("Save")),
            },
            BrowserStep::Fill {
                locator: BrowserLocator::reference(&reference("Search")),
                text: "batched ref fill".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::WaitForText {
                text: "Snapshot page".to_string(),
                timeout_ms: Some(2_000),
            },
            BrowserStep::Navigate {
                url: case.server.url("cookie-banner.html"),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "mixed batch failed: {report:?}");
    assert_eq!(report.steps.len(), 5);
    assert_eq!(
        report
            .steps
            .iter()
            .map(|step| step.step_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
}

// Optional clicks must not turn an absent convenience control into a failed workflow.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_02_click_if_exists_skips_missing_and_continues_batch() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::ClickIfExists {
                locator: BrowserLocator::css("#not-present"),
            },
            BrowserStep::Eval {
                expression: "document.body.dataset.continued = 'yes'; 'continued'".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "optional click aborted the batch: {report:?}");
    assert_eq!(report.steps.len(), 2);
    assert!(report.steps[0].ok);
    assert!(report.steps[0].summary.to_lowercase().contains("skipped"));
    assert_eq!(
        eval_value(&case.tab, "document.body.dataset.continued"),
        "yes"
    );
}

// Consent overlays must be removable both on demand and automatically before pointer actions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_03_dismiss_overlays_explicit_and_default_handler_paths() {
    let Some(mut case) = BrowserCase::start("cookie-banner.html").await else {
        return;
    };
    case.setup_world();
    let explicit = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::DismissOverlays,
            BrowserStep::Click {
                locator: BrowserLocator::css("#target"),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(
        explicit.ok,
        "explicit overlay dismissal failed: {explicit:?}"
    );
    assert!(explicit
        .locator_handlers
        .iter()
        .any(|firing| firing.name == "dismiss_overlays" && firing.ok));

    case.tab
        .navigate_to(&case.server.url("cookie-banner.html"))
        .unwrap();
    case.tab.wait_until_navigated().unwrap();
    let automatic = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Click {
            locator: BrowserLocator::css("#target"),
        }],
        &ImagePolicy::browser_capture(),
    );
    assert!(
        automatic.ok,
        "default overlay handler failed: {automatic:?}"
    );
    assert!(automatic
        .locator_handlers
        .iter()
        .any(|firing| firing.name == "dismiss_overlays" && firing.ok));
    assert_eq!(
        eval_value(&case.tab, "document.querySelector('#target').textContent"),
        "clicked"
    );
}

// Link extraction gives agents bounded structured data while preserving the full match count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_04_extract_links_is_bounded_and_reports_total() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.querySelector('nav').insertAdjacentHTML('beforeend', '<a href=\"/two\">Two</a><a href=\"/three\">Three</a>')",
            false,
        )
        .unwrap();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::ExtractLinks {
            locator: Some(BrowserLocator::css("nav")),
            limit: Some(2),
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "link extraction failed: {report:?}");
    let data = report.steps[0].data.as_ref().unwrap();
    assert_eq!(data["total"], 3);
    assert_eq!(data["links"].as_array().unwrap().len(), 2);
    assert!(data["links"]
        .as_array()
        .unwrap()
        .iter()
        .all(|link| { link["url"].is_string() && link["text"].is_string() }));
}

// Structured table rows prevent agents from reparsing presentation-oriented HTML or text.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_05_extract_table_returns_structured_rows() {
    let Some(mut case) = BrowserCase::start("roles.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::ExtractTable {
            locator: BrowserLocator::css("table:first-of-type"),
            limit: None,
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "table extraction failed: {report:?}");
    let data = report.steps[0].data.as_ref().unwrap();
    assert_eq!(data["total_rows"], 2);
    assert_eq!(data["rows"], json!([["Name", "Value"], ["Row", "Data"]]));

    let limited = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::ExtractTable {
            locator: BrowserLocator::css("table:first-of-type"),
            limit: Some(1),
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(limited.ok, "limited table extraction failed: {limited:?}");
    let limited_data = limited.steps[0].data.as_ref().unwrap();
    assert_eq!(limited_data["total_rows"], 2);
    assert_eq!(limited_data["rows"], json!([["Name", "Value"]]));
}

// Verified fallback filling preserves reliable form completion when trusted typing is rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_06_fill_fallback_reports_strategy_verification_and_retries() {
    let Some(mut case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::Fill {
            locator: BrowserLocator::css("#fallback"),
            text: "fallback value".to_string(),
            clear_first: true,
            verify: true,
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "fallback fill failed: {report:?}");
    assert_eq!(report.steps[0].field_kind, Some(FieldKind::TextInput));
    assert_ne!(
        report.steps[0].fill_strategy,
        Some(FillStrategy::NativeTyping)
    );
    assert_eq!(report.steps[0].verified, Some(true));
    assert!(report.steps[0].retries > 0);
}

// Capped computed-style inspection keeps diagnostics useful without flooding tool output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_07_styles_filters_properties_and_caps_output() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Styles {
                locator: BrowserLocator::css("body"),
                property_filter: None,
            },
            BrowserStep::Styles {
                locator: BrowserLocator::css("body"),
                property_filter: Some("color".to_string()),
            },
        ],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "style inspection failed: {report:?}");
    let all = report.steps[0].data.as_ref().unwrap()["styles"]
        .as_array()
        .unwrap();
    assert_eq!(all.len(), 51);
    assert!(all.last().unwrap().as_str().unwrap().contains("more"));
    let filtered = report.steps[1].data.as_ref().unwrap()["styles"]
        .as_array()
        .unwrap();
    assert!(!filtered.is_empty());
    assert!(filtered
        .iter()
        .all(|property| property.as_str().unwrap().contains("color")));
}

// Visible highlighting lets a human observer verify the element an agent is discussing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_08_highlight_element_draws_visible_outline() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[BrowserStep::HighlightElement {
            locator: BrowserLocator::css("nav button"),
        }],
        &ImagePolicy::browser_capture(),
    );

    assert!(report.ok, "highlight failed: {report:?}");
    let outline = eval_value(
        &case.tab,
        "document.querySelector('nav button').style.outline",
    )
    .as_str()
    .unwrap()
    .to_string();
    assert!(outline.contains("3px"), "missing outline width: {outline}");
    assert!(
        outline.contains("solid"),
        "missing outline style: {outline}"
    );
    assert!(outline.contains("231"), "missing outline color: {outline}");
}

// Independent report, chat-context, and timeline cursors prevent one consumer from starving another.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_09_tab_log_and_runtime_buffers_keep_independent_cursors() {
    let Some(mut case) = BrowserCase::start("fetch-after-click.html").await else {
        return;
    };
    case.setup_world();
    let runtime = Arc::new(tokio::sync::Mutex::new(case.runtime));
    let report = execute_request_with_runtime(
        runtime.clone(),
        BrowserActionRequest {
            session: SessionPolicy::SharedDefault,
            target: TabTarget::Active,
            attach_screenshot: None,
            page_context: None,
            network: NetworkReportMode::Full,
            steps: vec![
                BrowserStep::TabLog,
                BrowserStep::Eval {
                    expression: "console.log('cursor-console'); setTimeout(function(){ throw new Error('cursor-page-error'); }, 0); fetch('/slow-echo?ms=400').then(() => document.body.insertAdjacentText('beforeend', 'cursor-network-done')); 'started'".to_string(),
                },
                BrowserStep::WaitForText {
                    text: "cursor-network-done".to_string(),
                    timeout_ms: Some(2_000),
                },
            ],
            block_service_workers: None,
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();

    assert!(report.ok, "buffer-producing request failed: {report:?}");
    assert!(report.steps[0].ok);
    assert!(report
        .console
        .iter()
        .any(|entry| entry.text.contains("cursor-console")));
    assert!(report
        .page_errors
        .iter()
        .any(|entry| entry.contains("cursor-page-error")));
    assert!(report
        .network
        .iter()
        .any(|entry| entry.url.contains("/slow-echo")));
    let mut runtime = runtime.lock().await;
    assert!(runtime
        .flush_console_buffer()
        .iter()
        .any(|entry| entry.text.contains("cursor-console")));
    assert!(runtime
        .flush_network_buffer()
        .iter()
        .any(|entry| entry.url.contains("/slow-echo")));
    let (_, timeline_console, timeline_network) = runtime.flush_timeline_events();
    assert!(timeline_console
        .iter()
        .any(|entry| entry.text.contains("cursor-console")));
    assert!(timeline_network
        .iter()
        .any(|entry| entry.url.contains("/slow-echo")));
}

// The live recorder preserves replayable user activity and mutation summaries across the new core.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_10_live_recorder_emits_actions_and_mutation_summaries() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    case.tab
        .evaluate(
            "document.body.innerHTML = '<form id=\"record-form\" onsubmit=\"event.preventDefault()\"><input id=\"record-input\"><button id=\"record-click\" type=\"button\">Click</button><button type=\"submit\">Submit</button></form><div id=\"record-bottom\" style=\"margin-top:2000px\">Bottom</div>'",
            false,
        )
        .unwrap();
    let report = execute_steps_with_runtime(
        &mut case.runtime,
        &[
            BrowserStep::Fill {
                locator: BrowserLocator::css("#record-input"),
                text: "recorded".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::Click {
                locator: BrowserLocator::css("#record-click"),
            },
            BrowserStep::Eval {
                expression: "document.querySelector('#record-input').dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', bubbles:true})); document.querySelector('#record-form').requestSubmit(); window.scrollTo(0, document.body.scrollHeight); document.body.append(document.createElement('aside')); 'events-dispatched'".to_string(),
            },
        ],
        &ImagePolicy::browser_capture(),
    );
    assert!(report.ok, "recorded interactions failed: {report:?}");
    tokio::time::sleep(Duration::from_millis(800)).await;
    case.runtime.drain_raw_events();

    assert!(case
        .runtime
        .action_buffer
        .iter()
        .any(|event| matches!(event, RecorderEvent::Navigation { .. })));
    assert!(case
        .runtime
        .action_buffer
        .iter()
        .any(|event| matches!(event, RecorderEvent::Click { .. })));
    assert!(case
        .runtime
        .action_buffer
        .iter()
        .any(|event| matches!(event, RecorderEvent::Input { .. })));
    assert!(case
        .runtime
        .action_buffer
        .iter()
        .any(|event| matches!(event, RecorderEvent::Keypress { .. })));
    assert!(case
        .runtime
        .action_buffer
        .iter()
        .any(|event| matches!(event, RecorderEvent::Submit { .. })));
    assert!(case
        .runtime
        .action_buffer
        .iter()
        .any(|event| matches!(event, RecorderEvent::Scroll { .. })));
    assert!(!case.runtime.mutation_summary.is_empty());
}

// The closed high-z toolbar must remain visible to users without exposing page-owned DOM access.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_11_injected_toolbar_is_closed_high_z_and_emits_actions() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let toolbar = eval_json(
        &case.tab,
        "(() => { const host = document.querySelector('#__refact_toolbar_host'); return { mounted: !!host, closed: host && host.shadowRoot === null, z: host && getComputedStyle(host).zIndex }; })()",
    );
    assert_eq!(toolbar["mounted"], true);
    assert_eq!(toolbar["closed"], true);
    assert_eq!(toolbar["z"], "2147483646");

    case.tab
        .evaluate(
            "window.__refact_event(JSON.stringify({type:'toolbar_action',action:'screenshot',timestamp:Date.now()}))",
            false,
        )
        .unwrap();
    case.runtime.drain_raw_events();
    assert_eq!(case.runtime.drain_toolbar_actions(), vec!["screenshot"]);
}

// Two-layer masking must keep a password out of the final serialized browser payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_12_password_masking_survives_final_serialization() {
    const SECRET: &str = "t49-secret-value";
    let Some(mut case) = BrowserCase::start("form-actions.html").await else {
        return;
    };
    case.tab
        .evaluate(
            "document.body.insertAdjacentHTML('afterbegin', '<label>Password<input id=\"password\" type=\"password\" autocomplete=\"current-password\"></label>')",
            false,
        )
        .unwrap();
    case.setup_world();
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
                BrowserStep::Fill {
                    locator: BrowserLocator::css("#password"),
                    text: SECRET.to_string(),
                    clear_first: true,
                    verify: true,
                },
                BrowserStep::Eval {
                    expression: format!("console.error('password={SECRET}'); setTimeout(function(){{ throw new Error('password={SECRET}'); }}, 0); fetch('/slow-echo?ms=50'); 'started'"),
                },
                BrowserStep::WaitForResponse {
                    pattern: UrlPattern::Text("/slow-echo".to_string()),
                    method: None,
                    status: None,
                    timeout_ms: Some(2_000),
                },
            ],
        },
        &ImagePolicy::browser_capture(),
    )
    .await
    .unwrap();
    let recorder = runtime.lock().await.flush_action_buffer();
    let serialized = serde_json::to_string(&json!({
        "report": report,
        "recorder": recorder,
    }))
    .unwrap();

    assert!(!serialized.contains(SECRET), "secret leaked: {serialized}");
    assert!(serialized.contains("[REDACTED]") || serialized.contains("********"));
}

// Stealth belongs in the page main world while automation internals stay isolated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_13_stealth_patches_main_world_only() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    let stealth = case
        .tab
        .evaluate(
            "(async () => JSON.stringify({ webdriverUndefined: navigator.webdriver === undefined, runtime: typeof window.chrome.runtime.sendMessage, notification: (await navigator.permissions.query({name:'notifications'})).state === Notification.permission }))()",
            true,
        )
        .unwrap()
        .value
        .unwrap();
    let stealth: Value = serde_json::from_str(stealth.as_str().unwrap()).unwrap();
    assert_eq!(stealth["webdriverUndefined"], true);
    assert_eq!(stealth["runtime"], "function");
    assert_eq!(stealth["notification"], true);

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

// Per-chat attachment and idle expiry let one persistent profile survive changing chat owners safely.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_14_per_chat_runtime_preserves_profile_and_lifecycle() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    assert_eq!(case.runtime.profile_dir, case._profile.path());
    assert!(case.runtime.profile_dir.is_dir());
    case.runtime.reattach("chat-a");
    assert_eq!(case.runtime.attached_chat_id.as_deref(), Some("chat-a"));
    case.runtime.reattach("chat-b");
    assert_eq!(case.runtime.attached_chat_id.as_deref(), Some("chat-b"));
    case.runtime.detach();
    assert_eq!(case.runtime.attached_chat_id, None);
    case.runtime.idle_timeout = Duration::from_millis(1);
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(case.runtime.is_idle_expired());
    case.runtime.idle_timeout = Duration::from_secs(1);
    case.runtime.touch();
    assert!(!case.runtime.is_idle_expired());
}

// Device presets must keep their distinct viewport and DPR behavior after tab orchestration changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_15_device_presets_apply_dimensions_and_dpr() {
    let Some(mut case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.setup_world();
    for (device, width, height, dpr) in [
        ("desktop", 1440, 900, 2.0),
        ("mobile", 390, 844, 3.0),
        ("tablet", 834, 1112, 2.0),
    ] {
        let report = execute_steps_with_runtime(
            &mut case.runtime,
            &[BrowserStep::OpenTab {
                device: Some(device.to_string()),
                url: None,
            }],
            &ImagePolicy::browser_capture(),
        );
        assert!(report.ok, "{device} preset failed: {report:?}");
        let tab = case.runtime.get_active_tab().unwrap();
        let metrics = eval_json(
            &tab,
            "({screenWidth: screen.width, screenHeight: screen.height, innerWidth: innerWidth, innerHeight: innerHeight, dpr: devicePixelRatio})",
        );
        if device == "desktop" {
            assert_eq!(metrics["innerWidth"], width, "{device} width");
            assert_eq!(metrics["innerHeight"], height, "{device} height");
        } else {
            assert_eq!(metrics["screenWidth"], width, "{device} width");
            assert_eq!(metrics["screenHeight"], height, "{device} height");
        }
        assert_eq!(metrics["dpr"], dpr, "{device} DPR");
    }
}

// Rich element metadata lets form and actionability logic decide without ad hoc DOM reinspection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
async fn differentiator_16_rich_element_info_reports_state_text_box_and_field_kind() {
    let Some(case) = BrowserCase::start("snapshot.html").await else {
        return;
    };
    case.tab
        .evaluate(
            "document.body.insertAdjacentHTML('afterbegin', '<label>Rich<input id=\"rich\" value=\"rich value\" readonly></label><div id=\"editable\" contenteditable=\"true\">editable text</div>')",
            false,
        )
        .unwrap();
    case.tab.evaluate(INSPECT_ELEMENT_JS, false).unwrap();
    let inspect = |selector: &str| -> ElementInfo {
        let expression = format!(
            "JSON.stringify(__refact_inspect_element(document.querySelector({}), 1))",
            serde_json::to_string(selector).unwrap()
        );
        let value = eval_value(&case.tab, &expression);
        parse_element_info(value.as_str().unwrap()).unwrap()
    };

    let input = inspect("#rich");
    assert!(input.visible);
    assert!(input.enabled);
    assert!(input.readonly);
    assert!(!input.content_editable);
    assert_eq!(input.value.as_deref(), Some("rich value"));
    assert!(input
        .bbox
        .as_ref()
        .is_some_and(|bbox| bbox.width > 0.0 && bbox.height > 0.0));
    assert_eq!(input.field_kind, FieldKind::TextInput);

    let editable = inspect("#editable");
    assert!(editable.content_editable);
    assert_eq!(editable.inner_text.as_deref(), Some("editable text"));
    assert_eq!(editable.field_kind, FieldKind::ContentEditable);
}

// A fail-open install flag makes the health loop treat a crashed panel as healthy and never retry it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome"]
async fn injected_panel_install_flags_report_the_real_mount_state() {
    let mut case = BrowserCase::start_with_chrome("hostile-globals.html").await;
    case.setup_world();

    for flag in [
        "!!window.__refact_toolbar_installed",
        "!!window.__refact_recorder_installed",
        "!!window.__refact_overlays_installed",
        "!!document.querySelector('#__refact_toolbar_host')",
        "typeof window.__refact_overlays === 'object'",
        "!window.__refact_toolbar_blocked",
        "!window.__refact_recorder_blocked",
        "!window.__refact_overlays_blocked",
    ] {
        assert_eq!(eval_value(&case.tab, flag), Value::Bool(true), "{flag}");
    }

    assert_eq!(
        eval_value(
            &case.tab,
            "typeof window.__refact_overlays.startPicker === 'function' && typeof window.__refact_overlays.cancelPicker === 'function' && typeof window.__refact_overlays.startAnnotate === 'function'"
        ),
        Value::Bool(true)
    );

    let health = refact_lsp::refact_browser::probe_injection_health(&case.tab);
    assert!(!health.needs_injection);
    assert_eq!(health.blocked, None);

    case.tab
        .evaluate(
            "(function(){ document.querySelector('#__refact_toolbar_host').remove(); window.__refact_toolbar_installed = false; })()",
            false,
        )
        .unwrap();

    let degraded = refact_lsp::refact_browser::probe_injection_health(&case.tab);
    assert!(degraded.needs_injection);

    refact_lsp::refact_browser::ensure_injection_into_tab(
        &case.tab,
        true,
        case.runtime.buffers.raw_recorder_events.clone(),
    );

    for flag in [
        "!!window.__refact_toolbar_installed",
        "!!document.querySelector('#__refact_toolbar_host')",
    ] {
        assert_eq!(eval_value(&case.tab, flag), Value::Bool(true), "{flag}");
    }
}

// A picker overlay without a release path traps every click on the page behind a full-viewport shield.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome"]
async fn picker_overlay_always_releases_the_page() {
    let mut case = BrowserCase::start_with_chrome("selectors.html").await;
    case.setup_world();

    case.tab
        .evaluate("window.__refact_overlays.startPicker(60000)", false)
        .unwrap();
    let active = eval_json(
        &case.tab,
        "({ overlay: !!document.getElementById('__refact_picker_overlay'), active: !!window.__refact_picker_active })",
    );
    assert_eq!(active["overlay"], true);
    assert_eq!(active["active"], true);

    case.tab
        .evaluate(
            "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
            false,
        )
        .unwrap();
    let released = eval_json(
        &case.tab,
        "({ overlay: !!document.getElementById('__refact_picker_overlay'), active: !!window.__refact_picker_active })",
    );
    assert_eq!(released["overlay"], false);
    assert_eq!(released["active"], false);

    case.tab
        .evaluate("window.__refact_overlays.startPicker(150)", false)
        .unwrap();
    std::thread::sleep(Duration::from_millis(600));
    let timed_out = eval_json(
        &case.tab,
        "({ overlay: !!document.getElementById('__refact_picker_overlay'), active: !!window.__refact_picker_active })",
    );
    assert_eq!(timed_out["overlay"], false);
    assert_eq!(timed_out["active"], false);

    case.tab
        .evaluate("window.__refact_overlays.startPicker(60000)", false)
        .unwrap();
    case.tab
        .evaluate("window.__refact_overlays.cancelPicker()", false)
        .unwrap();
    let cancelled = eval_json(
        &case.tab,
        "({ overlay: !!document.getElementById('__refact_picker_overlay'), active: !!window.__refact_picker_active })",
    );
    assert_eq!(cancelled["overlay"], false);
    assert_eq!(cancelled["active"], false);
}
