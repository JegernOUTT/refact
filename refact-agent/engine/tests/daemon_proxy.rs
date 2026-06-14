use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::BoxBody;
use axum::Router;
use hyper::body::HttpBody;
use hyper::{Body, Method, Request, Response, StatusCode};
use refact_lsp::daemon::config::DaemonConfig;
use refact_lsp::daemon::events::EventBus;
use refact_lsp::daemon::projects::ProjectEntry;
use refact_lsp::daemon::state::DaemonState;
use refact_lsp::daemon::supervisor::{WorkerInfo, WorkerState};
use serial_test::serial;
use tempfile::{tempdir, TempDir};
use tokio::time::timeout;
use tower::ServiceExt;

struct EnvGuard {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set() -> Option<Self> {
        let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
        if std::process::Command::new(&python)
            .arg("--version")
            .output()
            .is_err()
        {
            return None;
        }
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake_worker.py");
        let previous = vec![
            (
                "REFACT_DAEMON_WORKER_CMD",
                std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
            ),
            (
                "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
            ),
            ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
        ];
        std::env::set_var(
            "REFACT_DAEMON_WORKER_CMD",
            format!("{} {}", python, script.display()),
        );
        std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
        std::env::remove_var("FAKE_WORKER_CRASH");
        Some(Self { keys: previous })
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

struct Harness {
    _dir: TempDir,
    state: Arc<DaemonState>,
    router: Router,
    entry: ProjectEntry,
}

impl Harness {
    async fn new(name: &str) -> Self {
        let dir = tempdir().unwrap();
        let project_root = dir.path().join(name);
        std::fs::create_dir_all(&project_root).unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        let router = refact_lsp::daemon::server::make_router(state.clone(), 8488);
        Self {
            _dir: dir,
            state,
            router,
            entry,
        }
    }

    fn v1_uri(&self, suffix: &str) -> String {
        format!("/p/{}/v1{}", self.entry.id, suffix)
    }

    fn build_info_uri(&self) -> String {
        format!("/p/{}/v1/build_info", self.entry.id)
    }

    async fn stop(&self) {
        self.state.supervisor.stop_all().await;
    }
}

async fn send(router: Router, request: Request<Body>) -> Response<BoxBody> {
    router.oneshot(request).await.unwrap()
}

async fn json_body(response: Response<BoxBody>) -> serde_json::Value {
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn request(method: Method, uri: String) -> hyper::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "daemon")
}

async fn read_chunk<B>(body: &mut B) -> String
where
    B: HttpBody<Data = hyper::body::Bytes> + Unpin,
    B::Error: std::fmt::Debug,
{
    let chunk = timeout(Duration::from_secs(3), body.data())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    String::from_utf8_lossy(&chunk).to_string()
}

async fn wait_for_live_streams(state: &DaemonState, project_id: &str, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let activity = state.proxy_activity(project_id).await;
        if activity.live_proxy_streams == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "live stream count did not settle"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_ready_with_new_pid(
    state: &DaemonState,
    project_id: &str,
    old_pid: u32,
) -> WorkerInfo {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(info) = state.supervisor.worker_info(project_id).await {
            if matches!(info.state, WorkerState::Ready) && info.pid != Some(old_pid) {
                return info;
            }
        }
        assert!(Instant::now() < deadline, "worker did not restart in time");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }

    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

async fn wait_for_port_closed(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            return;
        }
        assert!(Instant::now() < deadline, "worker port did not close");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_roundtrip_preserves_method_query_headers_body_and_auto_wakes() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("roundtrip-project").await;
    assert!(harness
        .state
        .supervisor
        .worker_info(&harness.entry.id)
        .await
        .is_none());

    let response = send(
        harness.router.clone(),
        request(
            Method::POST,
            harness.v1_uri("/echo?name=refact&space=a%20b"),
        )
        .header("x-custom", "kept")
        .header("connection", "x-hop")
        .header("x-hop", "stripped")
        .body(Body::from("hello proxy"))
        .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-hop-test").unwrap(), "visible");
    assert!(response.headers().get("x-hidden").is_none());
    let body = json_body(response).await;
    assert_eq!(body["method"], "POST");
    assert_eq!(body["path"], "/v1/echo?name=refact&space=a%20b");
    assert_eq!(body["headers"]["x-custom"], "kept");
    assert_eq!(body["headers"]["x-refact-project-id"], harness.entry.id);
    assert!(body["headers"].get("x-hop").is_none());
    assert_eq!(body["body_text"], "hello proxy");
    assert_eq!(body["body_len"], 11);
    assert_eq!(
        harness
            .state
            .supervisor
            .worker_info(&harness.entry.id)
            .await
            .unwrap()
            .state,
        WorkerState::Ready
    );
    assert!(
        harness
            .state
            .proxy_activity(&harness.entry.id)
            .await
            .last_proxy_activity_ms
            > 0
    );

    let build_info = send(
        harness.router.clone(),
        Request::builder()
            .uri(harness.build_info_uri())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(build_info.status(), StatusCode::OK);
    assert_eq!(json_body(build_info).await["version"], "fake-worker");

    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_auto_wakes_stopped_worker() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("stopped-worker-project").await;
    harness
        .state
        .supervisor
        .ensure_worker(&harness.entry)
        .await
        .unwrap();
    let stopped = harness
        .state
        .supervisor
        .stop_worker(&harness.entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.state, WorkerState::Stopped);

    let response = send(
        harness.router.clone(),
        Request::builder()
            .uri(harness.v1_uri("/echo"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["path"], "/v1/echo");
    assert_eq!(
        harness
            .state
            .supervisor
            .worker_info(&harness.entry.id)
            .await
            .unwrap()
            .state,
        WorkerState::Ready
    );
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_sse_delivers_first_chunk_before_second_is_sent() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("sse-project").await;
    harness
        .state
        .supervisor
        .ensure_worker(&harness.entry)
        .await
        .unwrap();

    let started = Instant::now();
    let response = send(
        harness.router.clone(),
        Request::builder()
            .uri(harness.v1_uri("/sse"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    wait_for_live_streams(&harness.state, &harness.entry.id, 1).await;
    let mut body = response.into_body();
    let mut collected = String::new();
    let mut first_at = None;
    let mut second_at = None;
    while second_at.is_none() {
        collected.push_str(&read_chunk(&mut body).await);
        if first_at.is_none() && collected.contains("chunk-a") {
            first_at = Some(Instant::now());
        }
        if second_at.is_none() && collected.contains("chunk-b") {
            second_at = Some(Instant::now());
        }
    }
    let first_at = first_at.unwrap();
    let second_at = second_at.unwrap();
    assert!(first_at.duration_since(started) < Duration::from_millis(400));
    assert!(second_at.duration_since(first_at) >= Duration::from_millis(400));
    drop(body);
    wait_for_live_streams(&harness.state, &harness.entry.id, 0).await;
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_concurrent_streams_to_same_worker_do_not_block_each_other() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("concurrent-sse-project").await;
    harness
        .state
        .supervisor
        .ensure_worker(&harness.entry)
        .await
        .unwrap();

    async fn first_chunk(router: Router, uri: String) -> (StatusCode, String, Duration) {
        let started = Instant::now();
        let response = send(
            router,
            Request::builder().uri(uri).body(Body::empty()).unwrap(),
        )
        .await;
        let status = response.status();
        let mut body = response.into_body();
        let chunk = read_chunk(&mut body).await;
        (status, chunk, started.elapsed())
    }

    let (first, second) = tokio::join!(
        first_chunk(harness.router.clone(), harness.v1_uri("/sse")),
        first_chunk(harness.router.clone(), harness.v1_uri("/sse")),
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    assert!(first.1.contains("chunk-a"));
    assert!(second.1.contains("chunk-a"));
    assert!(first.2 < Duration::from_millis(400));
    assert!(second.2 < Duration::from_millis(400));
    wait_for_live_streams(&harness.state, &harness.entry.id, 0).await;
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_unknown_project_returns_404_json() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("unknown-project").await;
    let response = send(
        harness.router.clone(),
        Request::builder()
            .uri("/p/missing/v1/echo")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["error"], "project not found");
    assert_eq!(body["project_id"], "missing");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_dead_worker_returns_502_emits_event_and_restarts() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("dead-worker-project").await;
    let info = harness
        .state
        .supervisor
        .ensure_worker(&harness.entry)
        .await
        .unwrap();
    let old_pid = info.pid.unwrap();
    kill_pid(old_pid);
    wait_for_port_closed(info.http_port).await;

    let response = send(
        harness.router.clone(),
        Request::builder()
            .uri(harness.v1_uri("/echo"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = json_body(response).await;
    assert_eq!(body["error"], "worker unavailable");
    assert_eq!(body["project_id"], harness.entry.id);
    let events = harness.state.events.snapshot().await;
    assert!(events.iter().any(|event| {
        event.kind == "proxy_worker_unreachable"
            && event.project_id.as_deref() == Some(harness.entry.id.as_str())
    }));
    let restarted = wait_for_ready_with_new_pid(&harness.state, &harness.entry.id, old_pid).await;
    assert_eq!(restarted.state, WorkerState::Ready);
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_body_limit_accepts_15mb_and_rejects_larger() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("body-limit-project").await;
    let accepted = vec![b'a'; refact_lsp::daemon::proxy::PROXY_BODY_LIMIT];
    let response = send(
        harness.router.clone(),
        request(Method::POST, harness.v1_uri("/echo"))
            .body(Body::from(accepted))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["body_len"],
        refact_lsp::daemon::proxy::PROXY_BODY_LIMIT
    );

    let rejected = vec![b'a'; refact_lsp::daemon::proxy::PROXY_BODY_LIMIT + 1];
    let response = send(
        harness.router.clone(),
        request(Method::POST, harness.v1_uri("/echo"))
            .body(Body::from(rejected))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn proxy_upgrade_requests_return_501() {
    let Some(_env) = EnvGuard::set() else {
        return;
    };
    let harness = Harness::new("upgrade-project").await;
    let response = send(
        harness.router.clone(),
        Request::builder()
            .uri(harness.v1_uri("/sse"))
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    harness.stop().await;
}
