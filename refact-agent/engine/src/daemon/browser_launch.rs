use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequest, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Form, Router, Server};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::{JoinHandle, JoinSet};
use uuid::Uuid;

use super::auth::{generate_token, token_matches};
use super::proxy::BROWSER_LAUNCH_HEADER;
use super::state::DaemonState;

const TICKET_LIFETIME: Duration = Duration::from_secs(60);
const DOCUMENT_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
const REVOKED_HOST_LIFETIME: Duration = Duration::from_secs(30);
const IPC_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_IPC_BYTES: usize = 8192;
const MAX_LAUNCHES: usize = 64;
pub(super) const REVOKE_PATH: &str = "/browser-launch/revoke";

#[derive(Serialize, Deserialize)]
struct LaunchRequest {
    project_id: String,
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct LaunchReply {
    pub file: PathBuf,
    pub url: String,
}

pub(super) struct BrowserLaunch {
    state: Arc<DaemonState>,
    port: u16,
    files: tempfile::TempDir,
    launches: Mutex<HashMap<String, Launch>>,
}

struct Launch {
    project_id: String,
    expires: Instant,
    phase: Phase,
}

enum Phase {
    Pending {
        ticket: String,
        _file: tempfile::TempPath,
    },
    Active {
        credential: Option<String>,
    },
}

pub(super) struct Runtime {
    launch: Arc<BrowserLaunch>,
    task: JoinHandle<()>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.task.abort();
        self.launch.launches.lock().clear();
    }
}

fn socket_name(pid: u32, control_port: u16) -> OsString {
    OsString::from_vec(format!("\0refact-browser-launch-v1.{pid}.{control_port}").into_bytes())
}

fn peer_verified(stream: &UnixStream, expected_pid: Option<u32>) -> bool {
    // SO_PEERCRED is supplied by the kernel, not by the process on the other end.
    let uid = unsafe { libc::geteuid() };
    stream.peer_cred().is_ok_and(|peer| {
        peer.uid() == uid
            && expected_pid
                .is_none_or(|pid| peer.pid().and_then(|pid| u32::try_from(pid).ok()) == Some(pid))
    })
}

pub(super) async fn request(
    daemon_pid: u32,
    control_port: u16,
    project_id: &str,
    root: &Path,
) -> Result<LaunchReply, String> {
    tokio::time::timeout(IPC_TIMEOUT, async {
        let mut socket = UnixStream::connect(socket_name(daemon_pid, control_port)).await.ok()?;
        if !peer_verified(&socket, Some(daemon_pid)) {
            return None;
        }
        let payload = serde_json::to_vec(&LaunchRequest {
            project_id: project_id.to_owned(),
            root: root.to_path_buf(),
        }).ok()?;
        if payload.len() > MAX_IPC_BYTES {
            return None;
        }
        socket.write_all(&payload).await.ok()?;
        socket.shutdown().await.ok()?;
        read_ipc(&mut socket).await
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| "Could not authorize the local Project Browser launch. Ensure the running daemon supports Linux Browser launch and both loopback addresses are available.".to_string())
}

async fn read_ipc<T: serde::de::DeserializeOwned>(socket: &mut UnixStream) -> Option<T> {
    let mut bytes = Vec::new();
    socket
        .take((MAX_IPC_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    if bytes.len() > MAX_IPC_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn bind_loopbacks() -> io::Result<(TcpListener, TcpListener, u16)> {
    let ipv4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = ipv4.local_addr()?.port();
    let ipv6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port))?;
    ipv4.set_nonblocking(true)?;
    ipv6.set_nonblocking(true)?;
    Ok((ipv4, ipv6, port))
}

pub(super) fn start(state: Arc<DaemonState>, control_port: u16) -> io::Result<Runtime> {
    let (ipv4, ipv6, port) = bind_loopbacks()?;
    let files = tempfile::Builder::new()
        .prefix("refact-browser-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let ipc = UnixListener::bind(socket_name(std::process::id(), control_port))?;
    let launch = Arc::new(BrowserLaunch {
        state,
        port,
        files,
        launches: Mutex::default(),
    });
    let router = make_router(launch.clone());
    let ipv4 = Server::from_tcp(ipv4)
        .map_err(io::Error::other)?
        .serve(router.clone().into_make_service());
    let ipv6 = Server::from_tcp(ipv6)
        .map_err(io::Error::other)?
        .serve(router.into_make_service());
    let running = launch.clone();
    let mut shutdown = running.state.shutdown_receiver();
    let task = tokio::spawn(async move {
        tokio::select! {
            _ = ipv4 => {},
            _ = ipv6 => {},
            _ = ipc_loop(ipc, running.clone()) => {},
            _ = shutdown.recv() => {},
        }
        running.launches.lock().clear();
        if !running.state.is_shutting_down() {
            tracing::warn!("local Project Browser launcher stopped unexpectedly");
        }
    });
    Ok(Runtime {
        launch,
        task,
    })
}

async fn ipc_loop(listener: UnixListener, launch: Arc<BrowserLaunch>) -> io::Result<()> {
    let mut handlers = JoinSet::new();
    let mut sweep = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut socket, _) = match accepted {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::warn!(%error, "local Browser launcher could not accept a connection");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                };
                if !peer_verified(&socket, None) || handlers.len() >= 16 {
                    continue;
                }
                let launch = launch.clone();
                handlers.spawn(async move {
                    let _ = tokio::time::timeout(IPC_TIMEOUT, async {
                        let Some(request) = read_ipc::<LaunchRequest>(&mut socket).await else { return; };
                        let matches = launch.state.projects.read().await.get(&request.project_id)
                            .is_some_and(|entry| entry.root == request.root);
                        if matches {
                            if let Ok(Some(reply)) = launch.issue(request.project_id) {
                                if let Ok(bytes) = serde_json::to_vec(&reply) {
                                    let _ = socket.write_all(&bytes).await;
                                }
                            }
                        }
                    }).await;
                });
            }
            _ = handlers.join_next(), if !handlers.is_empty() => {},
            _ = sweep.tick() => launch.launches.lock().retain(|_, launch| launch.expires > Instant::now()),
        }
    }
}

impl BrowserLaunch {
    fn issue(&self, project_id: String) -> io::Result<Option<LaunchReply>> {
        if self.state.is_shutting_down()
            || project_id.is_empty()
            || !project_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Ok(None);
        }
        let mut launches = self.launches.lock();
        launches.retain(|_, launch| launch.expires > Instant::now());
        if launches.len() >= MAX_LAUNCHES {
            return Ok(None);
        }
        let host = format!("refact-{}.localhost:{}", Uuid::new_v4().simple(), self.port);
        let url = format!("http://{host}/p/{project_id}/");
        let ticket = generate_token();
        let mut file = tempfile::Builder::new()
            .suffix(".html")
            .permissions(std::fs::Permissions::from_mode(0o600))
            .tempfile_in(self.files.path())?;
        write!(
            file,
            r#"<!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; form-action http://{host}; base-uri 'none'"><title>Open Refact</title><p>Opening your Refact project…</p><form method="post" action="{url}"><input type="hidden" name="ticket" value="{ticket}"></form><script>document.forms[0].submit()</script>"#
        )?;
        let file = file.into_temp_path();
        let reply = LaunchReply {
            file: file.to_path_buf(),
            url,
        };
        launches.insert(
            host,
            Launch {
                project_id,
                expires: Instant::now() + TICKET_LIFETIME,
                phase: Phase::Pending {
                    ticket,
                    _file: file,
                },
            },
        );
        Ok(Some(reply))
    }

    fn redeem(&self, host: &str, project_id: &str, proof: &str) -> Option<String> {
        let mut launches = self.launches.lock();
        let launch = launches.get_mut(host)?;
        if launch.project_id != project_id || launch.expires <= Instant::now() {
            return None;
        }
        let Phase::Pending {
            ticket, ..
        } = &launch.phase
        else {
            return None;
        };
        if !token_matches(proof, ticket) {
            return None;
        }
        let credential = generate_token();
        launch.phase = Phase::Active {
            credential: Some(credential.clone()),
        };
        launch.expires = Instant::now() + DOCUMENT_LIFETIME;
        Some(credential)
    }

    pub(super) fn authorize_browser(&self, headers: &HeaderMap, project_id: &str) -> bool {
        if self.state.is_shutting_down() {
            return false;
        }
        let Some((host, proof)) = browser_proof(headers) else {
            return false;
        };
        self.launches
            .lock()
            .get(host)
            .is_some_and(|launch| valid_proof(launch, project_id, proof))
    }

    fn revoke(&self, headers: &HeaderMap, project_id: &str) -> bool {
        let Some((host, proof)) = browser_proof(headers) else {
            return false;
        };
        let mut launches = self.launches.lock();
        let Some(launch) = launches.get_mut(host) else {
            return false;
        };
        if !valid_proof(launch, project_id, proof) {
            return false;
        }
        launch.phase = Phase::Active {
            credential: None,
        };
        launch.expires = Instant::now() + REVOKED_HOST_LIFETIME;
        true
    }
}

fn valid_proof(launch: &Launch, project_id: &str, proof: &str) -> bool {
    launch.project_id == project_id
        && launch.expires > Instant::now()
        && matches!(&launch.phase, Phase::Active { credential: Some(credential) } if token_matches(proof, credential))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

fn origin_matches(headers: &HeaderMap, host: &str) -> bool {
    single_header(headers, "origin").and_then(|s| s.strip_prefix("http://")) == Some(host)
}

fn browser_proof(headers: &HeaderMap) -> Option<(&str, &str)> {
    let host = single_header(headers, "host")?;
    if !origin_matches(headers, host) {
        return None;
    }
    Some((host, single_header(headers, BROWSER_LAUNCH_HEADER)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketForm {
    ticket: String,
}

fn make_router(launch: Arc<BrowserLaunch>) -> Router {
    Router::new()
        .fallback(handle_request)
        .layer(DefaultBodyLimit::max(128))
        .with_state(launch)
}

async fn handle_request(
    State(launch): State<Arc<BrowserLaunch>>,
    request: Request<Body>,
) -> Response {
    let mut response = dispatch(&launch, request).await;
    let headers = response.headers_mut();
    headers.remove(header::SET_COOKIE);
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

async fn dispatch(launch: &BrowserLaunch, request: Request<Body>) -> Response {
    if launch.state.is_shutting_down() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let headers = request.headers();
    let Some(host) = single_header(headers, "host") else {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    };
    if request.uri().authority().is_some()
        || headers.contains_key("service-worker")
        || single_header(headers, "sec-fetch-dest") == Some("serviceworker")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (project_id, active) = {
        let launches = launch.launches.lock();
        let Some(entry) = launches
            .get(host)
            .filter(|entry| entry.expires > Instant::now())
        else {
            return StatusCode::MISDIRECTED_REQUEST.into_response();
        };
        (
            entry.project_id.clone(),
            matches!(entry.phase, Phase::Active { .. }),
        )
    };
    let base = format!("/p/{project_id}");
    let path = request.uri().path();
    let project_path = path.strip_prefix(&base);
    if project_path == Some("/")
        && request.method() == Method::POST
        && request.uri().query().is_none()
    {
        if single_header(headers, "origin") != Some("null")
            || single_header(headers, "sec-fetch-mode") != Some("navigate")
            || single_header(headers, "sec-fetch-dest") != Some("document")
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        let host = host.to_owned();
        let Ok(Form(form)) = Form::<TicketForm>::from_request(request, &()).await else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let Some(proof) = launch.redeem(&host, &project_id, &form.ticket) else {
            return (
                StatusCode::UNAUTHORIZED,
                "Launch expired. Open the project again with refact ui.",
            )
                .into_response();
        };
        return super::web::isolated_project_index(&project_id, &host, Some(&proof));
    }
    if !active {
        return StatusCode::NOT_FOUND.into_response();
    }
    if (headers.contains_key(header::ORIGIN) && !origin_matches(headers, host))
        || (headers.contains_key("sec-fetch-site")
            && !matches!(
                single_header(headers, "sec-fetch-site"),
                Some("same-origin" | "none")
            ))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if path == REVOKE_PATH && request.method() == Method::POST {
        return if launch.revoke(headers, &project_id) {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::UNAUTHORIZED
        }
        .into_response();
    }
    if project_path == Some("/") && request.method() == Method::GET {
        return super::web::isolated_project_index(&project_id, host, None);
    }
    if let Some(asset) = path
        .strip_prefix("/dist/chat/")
        .filter(|_| matches!(*request.method(), Method::GET | Method::HEAD))
    {
        return super::web::handle_daemon_gui_asset(
            axum::extract::Path(asset.to_owned()),
            request.headers().clone(),
        )
        .await
        .into_response();
    }
    if project_path.is_some_and(|path| path == "/v1" || path.starts_with("/v1/")) {
        if matches!(
            single_header(headers, "sec-fetch-dest"),
            Some("script" | "sharedworker" | "worker")
        ) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let mut response =
            super::proxy::proxy_isolated_project(launch.state.clone(), project_id, request, launch)
                .await
                .into_response();
        response.headers_mut().append(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox; default-src 'none'; frame-ancestors 'none'"),
        );
        return response;
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Arc<DaemonState>) {
        let directory = tempfile::tempdir().unwrap();
        let state = DaemonState::new_with_daemon_dir(
            super::super::config::DaemonConfig::default(),
            super::super::events::EventBus::new(directory.path().join("events.jsonl")),
            None,
            directory.path().to_path_buf(),
            0,
        );
        (directory, state)
    }

    fn launch_host(url: &str) -> &str {
        url.strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
    }

    fn ticket_from_file(path: &Path) -> String {
        std::fs::read_to_string(path)
            .unwrap()
            .split_once("name=\"ticket\" value=\"")
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0
            .to_owned()
    }

    #[tokio::test]
    async fn ipc_verifies_daemon_pid_and_issues_private_files_only_for_registered_root() {
        let (directory, state) = fixture();
        let root = directory.path().canonicalize().unwrap();
        let project = state
            .projects
            .write()
            .await
            .open_with_settings(root.clone(), None)
            .await
            .unwrap();
        let runtime = start(state.clone(), 0).unwrap();
        let pid = std::process::id();

        let socket = UnixStream::connect(socket_name(pid, 0)).await.unwrap();
        assert!(peer_verified(&socket, Some(pid)));
        assert!(!peer_verified(&socket, Some(pid + 1)));
        drop(socket);
        let wrong_root = tempfile::tempdir().unwrap();
        assert!(request(pid, 0, &project.id, wrong_root.path())
            .await
            .is_err());
        let first = request(pid, 0, &project.id, &root).await.unwrap();
        let second = request(pid, 0, &project.id, &root).await.unwrap();
        assert_ne!(launch_host(&first.url), launch_host(&second.url));
        assert_eq!(first.file.extension().unwrap(), "html");
        assert_eq!(
            first.file.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            first
                .file
                .parent()
                .unwrap()
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(!first.url.contains(&ticket_from_file(&first.file)));
        let file = second.file.clone();
        drop(runtime);
        assert!(!file.exists());
    }

    #[test]
    fn binds_one_port_on_both_exact_loopbacks() {
        let (ipv4, ipv6, port) = bind_loopbacks().unwrap();
        assert_ne!(port, 0);
        assert_eq!(
            ipv4.local_addr().unwrap(),
            (Ipv4Addr::LOCALHOST, port).into()
        );
        assert_eq!(
            ipv6.local_addr().unwrap(),
            (Ipv6Addr::LOCALHOST, port).into()
        );
    }

    #[tokio::test]
    async fn ticket_is_spent_before_document_proof_and_origin_project_and_revocation_are_fenced() {
        let (_directory, state) = fixture();
        let launch = Arc::new(BrowserLaunch {
            state,
            port: 12345,
            files: tempfile::tempdir().unwrap(),
            launches: Mutex::default(),
        });
        let reply = launch.issue("project-a".into()).unwrap().unwrap();
        let host = launch_host(&reply.url);
        let ticket = ticket_from_file(&reply.file);
        let post = || {
            Request::builder()
                .method(Method::POST)
                .uri("/p/project-a/")
                .header(header::HOST, host)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::ORIGIN, "null")
                .header("sec-fetch-mode", "navigate")
                .header("sec-fetch-dest", "document")
                .body(Body::from(format!("ticket={ticket}")))
                .unwrap()
        };

        let (first, second) = tokio::join!(
            handle_request(State(launch.clone()), post()),
            handle_request(State(launch.clone()), post()),
        );
        let winner = if first.status() == StatusCode::UNAUTHORIZED {
            second
        } else {
            assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
            first
        };
        assert_eq!(winner.status(), StatusCode::OK);
        assert_eq!(winner.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!winner.headers().contains_key(header::SET_COOKIE));
        assert!(!reply.file.exists());
        let proof = match &launch.launches.lock()[host].phase {
            Phase::Active {
                credential: Some(proof),
            } => proof.clone(),
            _ => panic!("redemption must install a document proof"),
        };
        assert_ne!(proof, ticket);
        let html = hyper::body::to_bytes(winner.into_body()).await.unwrap();
        let html = std::str::from_utf8(&html).unwrap();
        assert!(html.contains(&proof));
        assert!(!html.contains(&ticket));

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        headers.insert(header::ORIGIN, format!("http://{host}").parse().unwrap());
        headers.insert(BROWSER_LAUNCH_HEADER, proof.parse().unwrap());
        assert!(launch.authorize_browser(&headers, "project-a"));
        assert!(!launch.authorize_browser(&headers, "project-b"));
        let mut wrong_origin = headers.clone();
        wrong_origin.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(!launch.authorize_browser(&wrong_origin, "project-a"));
        wrong_origin.remove(header::ORIGIN);
        assert!(!launch.authorize_browser(&wrong_origin, "project-a"));
        let mut duplicate = headers.clone();
        duplicate.append(BROWSER_LAUNCH_HEADER, HeaderValue::from_static("duplicate"));
        assert!(!launch.authorize_browser(&duplicate, "project-a"));
        for uri in [
            "/daemon/v1/status",
            "/p/project-b/v1/browser/project/status",
        ] {
            let mut request = Request::builder()
                .method(Method::POST)
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            *request.headers_mut() = headers.clone();
            let response = handle_request(State(launch.clone()), request).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        let reload = Request::builder()
            .uri("/p/project-a/")
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        let reload = handle_request(State(launch.clone()), reload).await;
        assert_eq!(reload.status(), StatusCode::OK);
        let reload = hyper::body::to_bytes(reload.into_body()).await.unwrap();
        assert!(!std::str::from_utf8(&reload).unwrap().contains(&proof));
        let mut revoke = Request::builder()
            .method(Method::POST)
            .uri(REVOKE_PATH)
            .body(Body::empty())
            .unwrap();
        *revoke.headers_mut() = headers.clone();
        assert_eq!(
            handle_request(State(launch.clone()), revoke).await.status(),
            StatusCode::NO_CONTENT
        );
        assert!(!launch.authorize_browser(&headers, "project-a"));

        let pending = launch.issue("project-a".into()).unwrap().unwrap();
        let host = launch_host(&pending.url);
        let proof = ticket_from_file(&pending.file);
        launch.launches.lock().get_mut(host).unwrap().expires = Instant::now();
        assert!(launch.redeem(host, "project-a", &proof).is_none());
    }
}
