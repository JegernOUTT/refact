use super::*;
use axum::extract::ConnectInfo;
use refact_self_update::public_release::{parse_source_url, PublicReleaseError, PublicResponse};
use refact_self_update::runtime_manifest::Channel;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

// The execution gate is intentionally process-global; keep lock-oracle tests
// isolated from other runtime route tests that briefly acquire it.

struct Fixture {
    dir: tempfile::TempDir,
    state: Arc<DaemonState>,
    core: RuntimeCoordinator,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let root = dir.path().canonicalize().unwrap();
        let mut config = DaemonConfig::default();
        config.auth.enabled = true;
        config.auth.username = Some("route-user".into());
        config.auth.password = Some("route-password".into());
        let state = DaemonState::new_with_daemon_dir(
            config,
            crate::daemon::events::EventBus::new(root.join("events.jsonl")),
            Some("worker-token".into()),
            root.join("daemon"),
            0,
        );
        let core = RuntimeCoordinator::new(
            root.join("config/runtime-selection.json"),
            root.join("staging"),
        )
        .unwrap();
        Self {
            dir,
            state,
            core,
        }
    }
    fn router(&self) -> Router {
        make_router(self.state.clone(), 0).layer(axum::Extension(self.core.clone()))
    }
}
fn basic() -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::prelude::BASE64_STANDARD.encode("route-user:route-password")
    )
}
fn selection() -> serde_json::Value {
    json!(parse_source_url(
        "https://github.com/JegernOUTT/refact/releases/tag/engine/v1.2.3",
        Channel::Preview
    )
    .unwrap())
}
async fn send(
    router: Router,
    method: &str,
    path: &str,
    auth: &str,
    body: impl Into<Body>,
) -> Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if !auth.is_empty() {
        builder = builder.header("authorization", auth);
    }
    let mut req = builder.body(body.into()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 40000))));
    router.oneshot(req).await.unwrap()
}
async fn value(r: Response) -> serde_json::Value {
    serde_json::from_slice(&hyper::body::to_bytes(r.into_body()).await.unwrap()).unwrap()
}

async fn send_headers(
    router: Router,
    endpoint: &str,
    auth: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/daemon/v1/runtime/{endpoint}"))
        .header("authorization", auth);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(body).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 40000))));
    router.oneshot(request).await.unwrap()
}

#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_csrf_headers_rejected_before_body_or_side_effects() {
    let f = Fixture::new();
    #[cfg(unix)]
    f.core
        .select(
            serde_json::from_value(json!({"expected_revision":0,"selection":selection()})).unwrap(),
        )
        .unwrap();
    #[cfg(unix)]
    let before = std::fs::read(f.dir.path().join("config/runtime-selection.json")).unwrap();
    let network = Arc::new(NoNetwork(AtomicUsize::new(0)));
    let cases: Vec<(Vec<(&str, &str)>, StatusCode)> = vec![
        (vec![], StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (
            vec![("content-type", "text/plain")],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            vec![("content-type", "application/json, text/plain")],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            vec![("content-type", "application/json; charset=")],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("content-type", "application/json"),
            ],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "https://attacker.example"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![("content-type", "application/json"), ("origin", "null")],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "localhost"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "http://localhost/path"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "http://user@localhost"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "http://localhost#x"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "http://localhost https://attacker.example"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "http://localhost"),
                ("origin", "http://localhost"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("origin", "http://localhost.attacker.example"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("host", "daemon.local:8488"),
                ("origin", "http://evil.local:8488"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("host", "daemon.local:8488"),
                ("origin", "http://daemon.local:8489"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("host", "daemon.local:8488"),
                ("host", "daemon.local:8488"),
                ("origin", "http://daemon.local:8488"),
            ],
            StatusCode::FORBIDDEN,
        ),
        (
            vec![
                ("content-type", "application/json"),
                ("host", "daemon.local:8488"),
                ("origin", "http://localhost:8488"),
            ],
            StatusCode::FORBIDDEN,
        ),
    ];
    for endpoint in ["select", "resolve", "stage"] {
        for (headers, expected) in &cases {
            let body = Body::wrap_stream(futures::stream::poll_fn(
                |_| -> std::task::Poll<Option<Result<Vec<u8>, std::io::Error>>> {
                    panic!("rejected CSRF request body was polled")
                },
            ));
            let response = send_headers(
                f.router()
                    .layer(axum::Extension(Transport(network.clone()))),
                endpoint,
                &basic(),
                headers,
                body,
            )
            .await;
            assert_eq!(response.status(), *expected, "{endpoint} {headers:?}");
        }
        assert_eq!(
            send_headers(
                f.router(),
                endpoint,
                "Bearer worker-token",
                &[
                    ("origin", "https://attacker.example"),
                    ("content-type", "text/plain")
                ],
                Body::empty()
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[cfg(unix)]
    {
        assert_eq!(
            std::fs::read(f.dir.path().join("config/runtime-selection.json")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read_dir(f.dir.path().join("config"))
                .unwrap()
                .count(),
            2
        );
    }
    #[cfg(not(unix))]
    assert!(!f.dir.path().join("config").exists());
    assert!(!f.dir.path().join("staging").exists());
    assert_eq!(network.0.load(Ordering::SeqCst), 0);
    assert!(!f.state.is_shutting_down());
}

#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_csrf_allows_cli_and_canonical_trusted_gui_json() {
    let f = Fixture::new();
    let network = Arc::new(NoNetwork(AtomicUsize::new(0)));
    for (revision, (host, origin)) in [
        (None, None),
        (Some("localhost:8488"), Some("http://localhost:8488")),
        (Some("127.0.0.1:8488"), Some("http://127.0.0.1:8488")),
        (Some("[::1]:8488"), Some("http://[::1]:8488")),
        (Some("daemon.local:8488"), Some("http://daemon.local:8488")),
        (Some("192.168.1.50:8488"), Some("http://192.168.1.50:8488")),
        (Some("daemon.local:8488"), Some("http://refactai")),
        (
            Some("daemon.local:8488"),
            Some("vscode-webview://11112222-3333-4444-5555-666677778888"),
        ),
    ]
    .iter()
    .enumerate()
    {
        let mut headers = vec![("content-type", "application/json; charset=UTF-8")];
        if let Some(host) = host {
            headers.push(("host", *host));
        }
        if let Some(origin) = origin {
            headers.push(("origin", *origin));
        }
        let response = send_headers(
            f.router(),
            "select",
            &basic(),
            &headers,
            Body::from(json!({"expected_revision":revision,"selection":selection()}).to_string()),
        )
        .await;
        #[cfg(unix)]
        {
            assert_eq!(response.status(), StatusCode::OK, "{origin:?}");
            assert_eq!(value(response).await["revision"], revision + 1);
        }
        #[cfg(not(unix))]
        {
            assert_eq!(response.status(), StatusCode::CONFLICT, "{origin:?}");
            assert!(value(response).await["error"]
                .as_str()
                .unwrap()
                .contains("security unsupported"));
        }
        for endpoint in ["resolve", "stage"] {
            let response = send_headers(
                f.router()
                    .layer(axum::Extension(Transport(network.clone()))),
                endpoint,
                &basic(),
                &headers,
                Body::from(r#"{"expected_revision":0}"#),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CONFLICT);
            let response = value(response).await;
            #[cfg(unix)]
            assert!(response["error"]
                .as_str()
                .unwrap()
                .contains("revision conflict"));
            #[cfg(not(unix))]
            assert!(response["error"]
                .as_str()
                .unwrap()
                .contains("selection absent"));
        }
    }
    assert_eq!(network.0.load(Ordering::SeqCst), 0);
    assert!(!f.dir.path().join("staging").exists());
    assert!(!f.state.is_shutting_down());
}

#[cfg(not(unix))]
#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_nonunix_mutations_fail_closed_without_files_or_network() {
    let f = Fixture::new();
    let before = std::fs::read_dir(f.dir.path()).unwrap().count();
    let network = Arc::new(NoNetwork(AtomicUsize::new(0)));
    let response = send(
        f.router(),
        "POST",
        "/daemon/v1/runtime/select",
        &basic(),
        json!({"expected_revision":0,"selection":selection()}).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(value(response).await["error"]
        .as_str()
        .unwrap()
        .contains("security unsupported"));
    for endpoint in ["resolve", "stage"] {
        let response = send(
            f.router()
                .layer(axum::Extension(Transport(network.clone()))),
            "POST",
            &format!("/daemon/v1/runtime/{endpoint}"),
            &basic(),
            r#"{"expected_revision":1}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(value(response).await["error"]
            .as_str()
            .unwrap()
            .contains("selection absent"));
    }
    let response = send(
        f.router(),
        "GET",
        "/daemon/v1/runtime/status",
        &basic(),
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = value(response).await;
    assert_eq!(response["status"]["selected"]["state"], "unknown");
    assert_eq!(response["status"]["installed"]["state"], "unknown");
    assert_eq!(response["status"]["staged"]["state"], "unknown");
    assert_eq!(response["activation_supported"], false);
    assert_eq!(response["restart_dispatched"], false);
    assert!(!f.dir.path().join("config").exists());
    assert!(!f.dir.path().join("staging").exists());
    assert_eq!(std::fs::read_dir(f.dir.path()).unwrap().count(), before);
    assert_eq!(network.0.load(Ordering::SeqCst), 0);
    assert!(!f.state.is_shutting_down());
}

struct NoNetwork(AtomicUsize);
impl PublicTransport for NoNetwork {
    fn get<'a>(&'a self, _: &'a str, _: bool, _: usize) -> ReadFuture<'a> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(PublicReleaseError::Rejected(
                "fixture network refused".into(),
            ))
        })
    }
}

#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_status_normal_auth_strict_pair_and_no_filesystem_writes() {
    let f = Fixture::new();
    let before = std::fs::read_dir(f.dir.path()).unwrap().count();
    assert_eq!(
        send(
            f.router(),
            "GET",
            "/daemon/v1/runtime/status",
            "",
            Body::empty()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    for query in ["?manifest_digest=abc", "?selection_revision=1", "?manifest_digest=abc&selection_revision=1", "?selection_revision=0&manifest_digest=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "?activate=true"] {
        assert_eq!(send(f.router(), "GET", &format!("/daemon/v1/runtime/status{query}"), &basic(), Body::empty()).await.status(), StatusCode::BAD_REQUEST);
    }
    let r = send(
        f.router(),
        "GET",
        "/daemon/v1/runtime/status",
        "Bearer worker-token",
        Body::empty(),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = value(r).await;
    assert_eq!(v["status"]["selection_revision"], 0);
    assert_eq!(v["activation_supported"], false);
    assert_eq!(v["restart_dispatched"], false);
    assert_eq!(std::fs::read_dir(f.dir.path()).unwrap().count(), before);
    assert!(!f.state.is_shutting_down());
    assert_eq!(f.state.supervisor.worker_count().await, 0);
}

#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_mutations_require_native_user_before_body_and_enforce_stream_cap() {
    let f = Fixture::new();
    for endpoint in ["select", "resolve", "stage"] {
        let path = format!("/daemon/v1/runtime/{endpoint}");
        for auth in ["", "Bearer worker-token", "Basic Zm9yZWlnbjpwdw=="] {
            assert_eq!(
                send(f.router(), "POST", &path, auth, "{".repeat(5000))
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        // Streaming body without content-length must also be bounded before JSON decode.
        let chunks = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(vec![b' '; 2048]),
            Ok(vec![b' '; 2049]),
        ]);
        assert_eq!(
            send(
                f.router(),
                "POST",
                &path,
                &basic(),
                Body::wrap_stream(chunks)
            )
            .await
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            send(
                f.router(),
                "POST",
                &path,
                &basic(),
                r#"{"expected_revision":1,"activate":true}"#
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert!(!f.dir.path().join("config").exists());
    assert!(!f.dir.path().join("staging").exists());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_select_cas_and_stale_prepare_never_fetch_or_install() {
    let f = Fixture::new();
    let request = json!({"expected_revision":0,"selection":selection()}).to_string();
    let r = send(
        f.router(),
        "POST",
        "/daemon/v1/runtime/select",
        &basic(),
        request.clone(),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(value(r).await["revision"], 1);
    assert_eq!(
        send(
            f.router(),
            "POST",
            "/daemon/v1/runtime/select",
            &basic(),
            request
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let network = Arc::new(NoNetwork(AtomicUsize::new(0)));
    for endpoint in ["resolve", "stage"] {
        let router = f
            .router()
            .layer(axum::Extension(Transport(network.clone())));
        assert_eq!(
            send(
                router,
                "POST",
                &format!("/daemon/v1/runtime/{endpoint}"),
                &basic(),
                r#"{"expected_revision":2}"#
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
    }
    assert_eq!(network.0.load(Ordering::SeqCst), 0);
    assert_eq!(f.core.load_selection().unwrap().unwrap().revision(), 1);
    assert!(!f.dir.path().join("staging").exists());
    assert!(!f.state.is_shutting_down());
    assert_eq!(f.state.supervisor.worker_count().await, 0);
    for endpoint in ["activate", "restart", "install"] {
        assert_eq!(
            send(
                f.router(),
                "POST",
                &format!("/daemon/v1/runtime/{endpoint}"),
                &basic(),
                "{}"
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_active_update_and_shutdown_reject_before_io_without_phase_changes() {
    let f = Fixture::new();
    let network = Arc::new(NoNetwork(AtomicUsize::new(0)));
    for phase in [
        UpdatePhase::Checking,
        UpdatePhase::Downloading,
        UpdatePhase::Restarting,
    ] {
        f.state.update_state.lock().await.phase = phase;
        let before = f.state.update_state.lock().await.clone();
        for endpoint in ["select", "resolve", "stage"] {
            let body = if endpoint == "select" {
                json!({"expected_revision":0,"selection":selection()})
            } else {
                json!({"expected_revision":1})
            };
            assert_eq!(
                send(
                    f.router()
                        .layer(axum::Extension(Transport(network.clone()))),
                    "POST",
                    &format!("/daemon/v1/runtime/{endpoint}"),
                    &basic(),
                    body.to_string()
                )
                .await
                .status(),
                StatusCode::CONFLICT
            );
        }
        assert_eq!(*f.state.update_state.lock().await, before);
    }
    f.state.update_state.lock().await.phase = UpdatePhase::Idle;
    f.state.request_shutdown("fixture".into());
    assert_eq!(
        send(
            f.router(),
            "POST",
            "/daemon/v1/runtime/select",
            &basic(),
            json!({"expected_revision":0,"selection":selection()}).to_string()
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(network.0.load(Ordering::SeqCst), 0);
    assert!(!f.dir.path().join("config").exists());
}

struct PausedTransport {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl PublicTransport for PausedTransport {
    fn get<'a>(&'a self, _: &'a str, _: bool, _: usize) -> ReadFuture<'a> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            Err(PublicReleaseError::Rejected("paused fixture".into()))
        })
    }
}
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_network_holds_no_global_locks_and_postcheck_reports_cache_uncertainty() {
    let f = Fixture::new();
    f.core
        .select(
            serde_json::from_value(json!({"expected_revision":0,"selection":selection()})).unwrap(),
        )
        .unwrap();
    let network = Arc::new(PausedTransport {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let router = f
        .router()
        .layer(axum::Extension(Transport(network.clone())));
    let task = tokio::spawn(async move {
        send(
            router,
            "POST",
            "/daemon/v1/runtime/stage",
            &basic(),
            r#"{"expected_revision":1}"#,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), network.entered.notified())
        .await
        .unwrap();
    let execution = crate::buddy::resource_execution::execution_gate()
        .try_lock()
        .expect("network must not block buddy execution");
    let mut update = f
        .state
        .update_state
        .try_lock()
        .expect("network must not hold update state");
    assert_eq!(update.phase, UpdatePhase::Idle);
    update.phase = UpdatePhase::Downloading;
    drop(update);
    drop(execution);
    network.release.notify_one();
    let r = task.await.unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert!(value(r).await["error"]
        .as_str()
        .unwrap()
        .contains("may already be cached"));
    assert_eq!(
        f.state.update_state.lock().await.phase,
        UpdatePhase::Downloading
    );
    assert!(!f.state.is_shutting_down());
    assert!(!f.dir.path().join("staging").exists());
}

struct ReleaseFixture(std::sync::Mutex<std::collections::VecDeque<(String, Vec<u8>)>>);
impl PublicTransport for ReleaseFixture {
    fn get<'a>(&'a self, url: &'a str, _: bool, cap: usize) -> ReadFuture<'a> {
        Box::pin(async move {
            let (suffix, body) = self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected fetch");
            assert!(url.ends_with(&suffix), "{url} != {suffix}");
            assert!(body.len() <= cap);
            Ok(PublicResponse {
                status: 200,
                location: None,
                body,
            })
        })
    }
}
fn release_fixture(archive_required: bool) -> ReleaseFixture {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    fn hash(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }
    let mut tar = vec![0u8; 2048];
    tar[..6].copy_from_slice(b"refact");
    tar[100..108].copy_from_slice(b"0000755\0");
    tar[108..116].copy_from_slice(b"0000000\0");
    tar[116..124].copy_from_slice(b"0000000\0");
    tar[124..136].copy_from_slice(b"00000000003\0");
    tar[136..148].copy_from_slice(b"00000000000\0");
    tar[148..156].fill(b' ');
    tar[156] = b'0';
    tar[257..263].copy_from_slice(b"ustar\0");
    tar[263..265].copy_from_slice(b"00");
    let sum: u32 = tar[..512].iter().map(|b| *b as u32).sum();
    tar[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    tar[512..515].copy_from_slice(b"new");
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).unwrap();
    let archive = gzip.finish().unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../crates/refact-self-update/tests/fixtures/runtime-manifest-v1.json"
    ))
    .unwrap();
    manifest["repository"] = json!({"owner":"JegernOUTT","name":"refact","id":1254216370u64});
    manifest["workflow"] = json!({"path":".github/workflows/plugin_build.yml","id":1});
    manifest["run"] = json!({"id":1,"attempt":1});
    manifest["source_sha"] = json!("a".repeat(40));
    manifest["build"]["version"] = json!("1.2.3");
    manifest["component_versions"]["runtime"] = json!("1.2.3");
    manifest["compatibility"] = json!({"daemon_api":{"min":1,"max":1},"worker_api":{"min":1,"max":1},"host_bridge":{"min":1,"max":1}});
    manifest["artifacts"] = json!([{"component":"runtime","target":"x86_64-unknown-linux-gnu","filename":"runtime.tar.gz","format":"tar_gz","size":archive.len(),"sha256":hash(&archive),"extracted_files":[{"path":"refact","size":3,"sha256":hash(b"new")}]}]);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let repo = json!({"id":1254216370u64,"full_name":"JegernOUTT/refact"});
    let mut rows = vec![
        ("/refact", serde_json::to_vec(&repo).unwrap()),
        ("releases/tags/engine%2Fv1%2E2%2E3", serde_json::to_vec(&json!({"id":99,"tag_name":"engine/v1.2.3","draft":false,"prerelease":true,"assets":[
            {"id":1,"name":"runtime-manifest.json","size":bytes.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&bytes))},
            {"id":2,"name":"runtime.tar.gz","size":archive.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&archive))}]})).unwrap()),
        ("git/ref/tags/engine%2Fv1%2E2%2E3", serde_json::to_vec(&json!({"ref":"refs/tags/engine/v1.2.3","object":{"type":"commit","sha":"a".repeat(40)}})).unwrap()),
        ("releases/assets/1", bytes),
        ("actions/workflows/plugin%5Fbuild%2Eyml", serde_json::to_vec(&json!({"id":1,"path":".github/workflows/plugin_build.yml","state":"active"})).unwrap()),
        ("actions/runs/1/attempts/1", serde_json::to_vec(&json!({"id":1,"run_attempt":1,"workflow_id":1,"path":".github/workflows/plugin_build.yml","head_sha":"a".repeat(40),"event":"workflow_dispatch","status":"completed","conclusion":"success","repository":repo,"head_repository":repo})).unwrap()),
    ];
    if archive_required {
        rows.push(("releases/assets/2", archive));
    }
    ReleaseFixture(std::sync::Mutex::new(
        rows.into_iter().map(|(s, b)| (s.into(), b)).collect(),
    ))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[tokio::test]
#[serial_test::serial(runtime_routes)]
async fn runtime_real_core_resolve_stage_status_keeps_installed_and_workers_unchanged() {
    let f = Fixture::new();
    let installed = f.dir.path().join("installed-sentinel");
    std::fs::write(&installed, b"old installed bytes").unwrap();
    assert_eq!(
        send(
            f.router(),
            "POST",
            "/daemon/v1/runtime/select",
            &basic(),
            json!({"expected_revision":0,"selection":selection()}).to_string()
        )
        .await
        .status(),
        StatusCode::OK
    );
    let before = f.state.update_state.lock().await.clone();
    for endpoint in ["resolve", "stage"] {
        let network = Arc::new(release_fixture(endpoint == "stage"));
        let router = f
            .router()
            .layer(axum::Extension(Transport(network.clone())));
        let r = send(
            router,
            "POST",
            &format!("/daemon/v1/runtime/{endpoint}"),
            &basic(),
            r#"{"expected_revision":1}"#,
        )
        .await;
        let code = r.status();
        let v = value(r).await;
        assert_eq!(code, StatusCode::OK, "{v}");
        assert!(network.0.lock().unwrap().is_empty());
        let plan = if endpoint == "stage" { &v["plan"] } else { &v };
        assert_eq!(plan["activation_supported"], false);
        assert_eq!(plan["restart_dispatched"], false);
        if endpoint == "resolve" {
            assert!(!f.dir.path().join("staging").exists());
        } else {
            let query = format!(
                "/daemon/v1/runtime/status?manifest_digest={}&selection_revision=1",
                v["staged"]["manifest_digest"].as_str().unwrap()
            );
            let status =
                value(send(f.router(), "GET", &query, &basic(), Body::empty()).await).await;
            assert_eq!(status["status"]["installed"]["state"], "unknown");
            assert_eq!(status["status"]["staged"]["state"], "known");
        }
    }
    assert_eq!(*f.state.update_state.lock().await, before);
    assert_eq!(std::fs::read(installed).unwrap(), b"old installed bytes");
    assert_eq!(f.state.supervisor.worker_count().await, 0);
    assert!(!f.state.is_shutting_down());
}
