use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::exec::{
    ExecOutputChunk, ExecOutputStream, ExecOwnerMeta, ExecProcessFilter, ExecProcessId,
    ExecProcessSnapshot, ExecRawRead, ExecSpawnRequest, ExecStatus,
};
use crate::files_correction::{
    canonicalize_normalized_path, check_if_its_inside_a_workspace_or_config,
    get_active_workspace_folder,
};

const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;
const DEFAULT_READ_LIMIT: usize = 1000;
const MAX_READ_LIMIT: usize = 10_000;
const RAW_TAIL_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
pub struct ExecSpawnHttpRequest {
    pub command: Option<String>,
    pub argv: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, String>>,
    #[serde(default = "default_true")]
    pub pty: bool,
    #[serde(default = "default_rows")]
    pub rows: u16,
    #[serde(default = "default_cols")]
    pub cols: u16,
    pub service_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecSpawnHttpResponse {
    pub process_id: String,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ExecProcessHttpSnapshot {
    pub process_id: String,
    pub status: &'static str,
    pub command_preview: String,
    pub created_at_ms: u64,
    pub tty: bool,
    pub service_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecListHttpResponse {
    pub processes: Vec<ExecProcessHttpSnapshot>,
}

#[derive(Debug, Deserialize)]
pub struct ExecReadQuery {
    #[serde(default)]
    pub since_seq: u64,
    pub limit: Option<usize>,
    #[serde(default)]
    pub raw: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExecSubscribeQuery {
    #[serde(default)]
    pub since_seq: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecOutputHttpChunk {
    pub seq: u64,
    pub stream: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ExecReadHttpResponse {
    pub chunks: Vec<ExecOutputHttpChunk>,
    pub next_seq: u64,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ExecKillHttpResponse {
    pub process_id: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct ExecResizeRequest {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Deserialize)]
pub struct ExecStdinRequest {
    pub chars: String,
}

#[derive(Debug, Serialize)]
pub struct ExecStdinResponse {
    pub process_id: String,
    pub status: &'static str,
    pub bytes_written: usize,
    pub since_seq: u64,
    pub next_seq: u64,
    pub latest_seq: u64,
}

fn default_true() -> bool {
    true
}

fn default_rows() -> u16 {
    DEFAULT_ROWS
}

fn default_cols() -> u16 {
    DEFAULT_COLS
}

fn exec_http_disabled() -> bool {
    // Daemon terminal_enabled policy is enforced at the proxy; this is the worker hard stop.
    std::env::var("REFACT_DISABLE_EXEC_HTTP")
        .map(|value| value == "1")
        .unwrap_or(false)
}

async fn active_workspace(app: &AppState) -> Result<PathBuf, ScratchError> {
    get_active_workspace_folder(app.gcx.clone())
        .await
        .map(canonicalize_normalized_path)
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::BAD_REQUEST,
                "no active workspace folder".to_string(),
            )
        })
}

async fn authorize_process(
    app: &AppState,
    process_id: &ExecProcessId,
) -> Result<(ExecProcessSnapshot, PathBuf), ScratchError> {
    let workspace = active_workspace(app).await?;
    let snapshot = app
        .runtime
        .exec_registry
        .authorize_process_access(process_id, "", Some(&workspace))
        .await
        .map_err(process_access_error)?;
    Ok((snapshot, workspace))
}

fn process_access_error(message: String) -> ScratchError {
    let status = if message.starts_with("process not found:") {
        StatusCode::NOT_FOUND
    } else if message.starts_with("process access denied:") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    };
    ScratchError::new(status, message)
}

fn command_from_request(request: &ExecSpawnHttpRequest) -> Result<String, ScratchError> {
    match (&request.command, &request.argv) {
        (Some(command), None) if !command.trim().is_empty() => Ok(command.clone()),
        (None, Some(argv)) if !argv.is_empty() && argv.iter().all(|part| !part.is_empty()) => {
            Ok(command_from_argv(argv))
        }
        (Some(_), Some(_)) => Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "provide exactly one of command or argv".to_string(),
        )),
        _ => Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "provide exactly one non-empty command or argv".to_string(),
        )),
    }
}

#[cfg(not(target_os = "windows"))]
fn command_from_argv(argv: &[String]) -> String {
    shell_words::join(argv)
}

#[cfg(target_os = "windows")]
fn command_from_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|part| powershell_escape(part))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "windows")]
fn powershell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn resolve_cwd(
    app: &AppState,
    workspace: &FsPath,
    cwd: Option<&str>,
) -> Result<PathBuf, ScratchError> {
    let requested = cwd
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            }
        })
        .unwrap_or_else(|| workspace.to_path_buf());
    let cwd = canonicalize_normalized_path(requested);
    check_if_its_inside_a_workspace_or_config(app.gcx.clone(), &cwd)
        .await
        .map_err(|message| ScratchError::new(StatusCode::BAD_REQUEST, message))?;
    if !cwd.starts_with(workspace) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("cwd '{}' is outside the active workspace", cwd.display()),
        ));
    }
    if !cwd.is_dir() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("cwd '{}' is not a directory", cwd.display()),
        ));
    }
    Ok(cwd)
}

pub async fn handle_v1_exec_spawn(
    State(app): State<AppState>,
    Json(request): Json<ExecSpawnHttpRequest>,
) -> Result<Json<ExecSpawnHttpResponse>, ScratchError> {
    if exec_http_disabled() {
        return Err(ScratchError::new(
            StatusCode::FORBIDDEN,
            "exec HTTP is disabled".to_string(),
        ));
    }
    if request.pty && (request.rows == 0 || request.cols == 0) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "PTY rows and cols must be greater than zero".to_string(),
        ));
    }
    let command = command_from_request(&request)?;
    let workspace = active_workspace(&app).await?;
    let cwd = resolve_cwd(&app, &workspace, request.cwd.as_deref()).await?;
    let owner = ExecOwnerMeta {
        chat_id: None,
        tool_call_id: None,
        service_name: request.service_name.clone(),
        workspace: Some(workspace),
    };
    let spawn_request = if request.pty {
        ExecSpawnRequest::interactive(command)
    } else {
        ExecSpawnRequest::background(command)
    }
    .with_cwd(cwd)
    .with_env_map(request.env.unwrap_or_default())
    .with_tty(request.pty)
    .with_pty_size(request.rows, request.cols)
    .with_owner(owner);
    let result = app
        .runtime
        .exec_registry
        .spawn(spawn_request)
        .await
        .map_err(|message| ScratchError::new(StatusCode::BAD_REQUEST, message))?;
    Ok(Json(ExecSpawnHttpResponse {
        process_id: result.snapshot.meta.process_id.as_str().to_string(),
        status: status_label(&result.snapshot.status),
    }))
}

pub async fn handle_v1_exec_list(
    State(app): State<AppState>,
) -> Result<Json<ExecListHttpResponse>, ScratchError> {
    let workspace = active_workspace(&app).await?;
    let snapshots = app
        .runtime
        .exec_registry
        .list(ExecProcessFilter {
            workspace: Some(workspace),
            ..ExecProcessFilter::default()
        })
        .await;
    Ok(Json(ExecListHttpResponse {
        processes: snapshots.iter().map(http_snapshot).collect(),
    }))
}

pub async fn handle_v1_exec_read(
    State(app): State<AppState>,
    Path(process_id): Path<String>,
    Query(query): Query<ExecReadQuery>,
) -> Result<Json<ExecReadHttpResponse>, ScratchError> {
    let process_id = ExecProcessId(process_id);
    let (snapshot, _) = authorize_process(&app, &process_id).await?;
    if query.raw && snapshot.meta.tty {
        let raw = app
            .runtime
            .exec_registry
            .read_raw_since(&process_id, query.since_seq, Some(RAW_TAIL_MAX_BYTES))
            .await;
        let (chunks, next_seq) = match raw {
            Some(raw) if !raw.text.is_empty() => {
                let next_seq = raw.new_offset;
                (vec![raw_http_chunk(&raw)], next_seq)
            }
            Some(raw) => (Vec::new(), raw.new_offset.max(query.since_seq)),
            None => (Vec::new(), query.since_seq),
        };
        return Ok(Json(ExecReadHttpResponse {
            chunks,
            next_seq,
            status: status_label(&snapshot.status),
        }));
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_READ_LIMIT)
        .min(MAX_READ_LIMIT);
    let read = app
        .runtime
        .exec_registry
        .read(&process_id, query.since_seq, Some(limit))
        .await;
    Ok(Json(ExecReadHttpResponse {
        chunks: read.chunks.iter().map(http_chunk).collect(),
        next_seq: read.next_seq,
        status: status_label(&snapshot.status),
    }))
}

pub async fn handle_v1_exec_kill(
    State(app): State<AppState>,
    Path(process_id): Path<String>,
) -> Result<Json<ExecKillHttpResponse>, ScratchError> {
    let process_id = ExecProcessId(process_id);
    authorize_process(&app, &process_id).await?;
    let snapshot = app
        .runtime
        .exec_registry
        .kill(&process_id)
        .await
        .map_err(|message| ScratchError::new(StatusCode::BAD_REQUEST, message))?;
    Ok(Json(ExecKillHttpResponse {
        process_id: process_id.as_str().to_string(),
        status: status_label(&snapshot.status),
    }))
}

pub async fn handle_v1_exec_resize(
    State(app): State<AppState>,
    Path(process_id): Path<String>,
    Json(request): Json<ExecResizeRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let process_id = ExecProcessId(process_id);
    authorize_process(&app, &process_id).await?;
    app.runtime
        .exec_registry
        .resize(&process_id, request.rows, request.cols)
        .await
        .map_err(|message| ScratchError::new(StatusCode::BAD_REQUEST, message))?;
    Ok(Json(json!({})))
}

pub async fn handle_v1_exec_stdin(
    State(app): State<AppState>,
    Path(process_id): Path<String>,
    Json(request): Json<ExecStdinRequest>,
) -> Result<Json<ExecStdinResponse>, ScratchError> {
    let process_id = ExecProcessId(process_id);
    authorize_process(&app, &process_id).await?;
    let result = app
        .runtime
        .exec_registry
        .write_stdin(&process_id, &request.chars, 0)
        .await
        .map_err(|message| ScratchError::new(StatusCode::BAD_REQUEST, message))?;
    let snapshot = app
        .runtime
        .exec_registry
        .get(&process_id)
        .await
        .ok_or_else(|| ScratchError::new(StatusCode::NOT_FOUND, "process not found".to_string()))?;
    Ok(Json(ExecStdinResponse {
        process_id: process_id.as_str().to_string(),
        status: status_label(&snapshot.status),
        bytes_written: result.bytes_written,
        since_seq: result.read.since_seq,
        next_seq: result.read.next_seq,
        latest_seq: result.read.latest_seq,
    }))
}

pub async fn handle_v1_exec_subscribe(
    State(app): State<AppState>,
    Path(process_id): Path<String>,
    Query(query): Query<ExecSubscribeQuery>,
) -> Result<Response<Body>, ScratchError> {
    let process_id = ExecProcessId(process_id);
    let (initial_snapshot, _) = authorize_process(&app, &process_id).await?;
    if initial_snapshot.meta.tty {
        return Ok(subscribe_raw_tty_stream(
            app,
            process_id,
            initial_snapshot,
            query.since_seq,
        ));
    }
    let registry = app.runtime.exec_registry.clone();
    let mut output_rx = registry.subscribe_output();
    let initial_read = registry
        .read(&process_id, 0, Some(DEFAULT_READ_LIMIT))
        .await;
    let mut next_seq = initial_read.next_seq;
    let shutdown_flag = app.gcx.shutdown_flag.clone();

    let stream = async_stream::stream! {
        let snapshot = json!({
            "status": status_label(&initial_snapshot.status),
            "chunks": initial_read.chunks.iter().map(http_chunk).collect::<Vec<_>>(),
            "next_seq": initial_read.next_seq,
        });
        yield Ok::<_, Infallible>(sse_event("snapshot", &snapshot));
        if initial_snapshot.status.is_terminal() {
            yield Ok::<_, Infallible>(sse_exit(&initial_snapshot));
            return;
        }

        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(15),
            Duration::from_secs(15),
        );
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut status_poll = tokio::time::interval(Duration::from_millis(100));
        status_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut shutdown_poll = tokio::time::interval(Duration::from_millis(200));
        shutdown_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = output_rx.recv() => {
                    match result {
                        Ok(chunk) if chunk.process_id == process_id && chunk.seq >= next_seq => {
                            next_seq = chunk.seq.saturating_add(1);
                            yield Ok::<_, Infallible>(sse_event("output", &http_chunk(&chunk)));
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let read = registry.read(&process_id, next_seq, Some(MAX_READ_LIMIT)).await;
                            for chunk in read.chunks {
                                next_seq = chunk.seq.saturating_add(1);
                                yield Ok::<_, Infallible>(sse_event("output", &http_chunk(&chunk)));
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = status_poll.tick() => {
                    match registry.get(&process_id).await {
                        Some(snapshot) if snapshot.status.is_terminal() => {
                            let read = registry.read(&process_id, next_seq, Some(MAX_READ_LIMIT)).await;
                            for chunk in read.chunks {
                                yield Ok::<_, Infallible>(sse_event("output", &http_chunk(&chunk)));
                            }
                            yield Ok::<_, Infallible>(sse_exit(&snapshot));
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok::<_, Infallible>(": hb\n\n".to_string());
                }
                _ = shutdown_poll.tick() => {
                    if shutdown_flag.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(stream))
        .unwrap())
}

async fn next_raw_increment(
    registry: &crate::exec::ExecRegistry,
    process_id: &ExecProcessId,
    offset: u64,
) -> Option<ExecRawRead> {
    registry
        .read_raw_since(process_id, offset, None)
        .await
        .filter(|raw| !raw.text.is_empty())
}

fn subscribe_raw_tty_stream(
    app: AppState,
    process_id: ExecProcessId,
    initial_snapshot: ExecProcessSnapshot,
    since_offset: u64,
) -> Response<Body> {
    let registry = app.runtime.exec_registry.clone();
    let mut output_rx = registry.subscribe_output();
    let shutdown_flag = app.gcx.shutdown_flag.clone();

    let stream = async_stream::stream! {
        let initial = registry
            .read_raw_since(&process_id, since_offset, Some(RAW_TAIL_MAX_BYTES))
            .await;
        let mut offset = initial
            .as_ref()
            .map(|raw| raw.new_offset)
            .unwrap_or(since_offset)
            .max(since_offset);
        let chunks = match initial.as_ref() {
            Some(raw) if !raw.text.is_empty() => vec![raw_http_chunk(raw)],
            _ => Vec::new(),
        };
        let snapshot = json!({
            "status": status_label(&initial_snapshot.status),
            "chunks": chunks,
            "next_seq": offset,
        });
        yield Ok::<_, Infallible>(sse_event("snapshot", &snapshot));
        if initial_snapshot.status.is_terminal() {
            yield Ok::<_, Infallible>(sse_exit(&initial_snapshot));
            return;
        }

        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(15),
            Duration::from_secs(15),
        );
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut status_poll = tokio::time::interval(Duration::from_millis(100));
        status_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut shutdown_poll = tokio::time::interval(Duration::from_millis(200));
        shutdown_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = output_rx.recv() => {
                    match result {
                        Ok(chunk) if chunk.process_id == process_id => {
                            if let Some(raw) = next_raw_increment(&registry, &process_id, offset).await {
                                offset = raw.new_offset;
                                yield Ok::<_, Infallible>(sse_event("output", &raw_http_chunk(&raw)));
                            }
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Some(raw) = next_raw_increment(&registry, &process_id, offset).await {
                                offset = raw.new_offset;
                                yield Ok::<_, Infallible>(sse_event("output", &raw_http_chunk(&raw)));
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = status_poll.tick() => {
                    match registry.get(&process_id).await {
                        Some(snapshot) if snapshot.status.is_terminal() => {
                            if let Some(raw) = next_raw_increment(&registry, &process_id, offset).await {
                                yield Ok::<_, Infallible>(sse_event("output", &raw_http_chunk(&raw)));
                            }
                            yield Ok::<_, Infallible>(sse_exit(&snapshot));
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok::<_, Infallible>(": hb\n\n".to_string());
                }
                _ = shutdown_poll.tick() => {
                    if shutdown_flag.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(stream))
        .unwrap()
}

fn http_snapshot(snapshot: &ExecProcessSnapshot) -> ExecProcessHttpSnapshot {
    ExecProcessHttpSnapshot {
        process_id: snapshot.meta.process_id.as_str().to_string(),
        status: status_label(&snapshot.status),
        command_preview: snapshot.meta.short_description.clone(),
        created_at_ms: snapshot.meta.created_at_ms,
        tty: snapshot.meta.tty,
        service_name: snapshot.meta.owner.service_name.clone(),
    }
}

fn http_chunk(chunk: &ExecOutputChunk) -> ExecOutputHttpChunk {
    ExecOutputHttpChunk {
        seq: chunk.seq,
        stream: stream_label(&chunk.stream),
        text: chunk.text.clone(),
        offset: None,
    }
}

fn raw_http_chunk(raw: &ExecRawRead) -> ExecOutputHttpChunk {
    ExecOutputHttpChunk {
        seq: raw.start_offset,
        stream: "combined",
        text: raw.text.clone(),
        offset: Some(raw.new_offset),
    }
}

fn sse_event<T: Serialize>(event: &str, value: &T) -> String {
    let data = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    format!("event: {event}\ndata: {data}\n\n")
}

fn sse_exit(snapshot: &ExecProcessSnapshot) -> String {
    sse_event(
        "exit",
        &json!({
            "process_id": snapshot.meta.process_id.as_str(),
            "status": status_label(&snapshot.status),
        }),
    )
}

fn stream_label(stream: &ExecOutputStream) -> &'static str {
    match stream {
        ExecOutputStream::Stdout => "stdout",
        ExecOutputStream::Stderr => "stderr",
        ExecOutputStream::Combined => "combined",
    }
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited { .. } => "exited",
        ExecStatus::Failed { .. } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hyper::body::{to_bytes, HttpBody};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::app_state::AppState;
    use crate::http::routers::make_refact_http_server;

    async fn test_app() -> (tempfile::TempDir, AppState) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        (temp, AppState::from_gcx(gcx).await)
    }

    async fn json_response(router: axum::Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body()).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    fn post_json(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn spawn_requires_exactly_one_command_shape() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app);

        let (status, _) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "echo one", "argv": ["echo", "two"] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = json_response(
            router,
            post_json("/v1/exec/spawn", json!({ "command": null, "argv": null })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn auth_matrix_spawn_rejects_cwd_outside_workspace() {
        let (_temp, app) = test_app().await;
        let outside = tempfile::tempdir().unwrap();
        let router = make_refact_http_server(app);

        let (status, json) = json_response(
            router,
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "echo hi", "pty": false, "cwd": outside.path() }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["detail"].as_str().is_some());
    }

    #[tokio::test]
    #[serial_test::serial(exec_http_env)]
    async fn auth_matrix_spawn_forbidden_when_exec_http_disabled() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app);

        std::env::set_var("REFACT_DISABLE_EXEC_HTTP", "1");
        let result = json_response(
            router,
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "echo hi", "pty": false }),
            ),
        )
        .await;
        std::env::remove_var("REFACT_DISABLE_EXEC_HTTP");

        let (status, json) = result;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(json["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("exec HTTP is disabled"));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn exec_http_lifecycle_is_workspace_scoped() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app.clone());
        let (status, spawned) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "argv": ["sh", "-c", "printf hello; sleep 30"], "pty": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let process_id = spawned["process_id"].as_str().unwrap();

        let (status, listed) = json_response(
            router.clone(),
            Request::builder()
                .uri("/v1/exec/list")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(listed["processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|process| process["process_id"] == process_id));

        tokio::time::sleep(Duration::from_millis(100)).await;
        let (status, read) = json_response(
            router.clone(),
            Request::builder()
                .uri(format!("/v1/exec/{process_id}/read?since_seq=0&limit=10"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(read["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|chunk| chunk["text"].as_str().unwrap().contains("hello")));

        let (status, killed) = json_response(
            router,
            post_json(&format!("/v1/exec/{process_id}/kill"), json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(killed["status"], "killed");
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn write_stdin_to_running_tty_process_succeeds() {
        let (_temp, app_state) = test_app().await;
        let router = make_refact_http_server(app_state.clone());
        let (status, spawned) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "read line; printf 'got:%s\\n' \"$line\"", "pty": true }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let process_id = spawned["process_id"].as_str().unwrap();

        let (status, _) = json_response(
            router.clone(),
            post_json(
                &format!("/v1/exec/{process_id}/resize"),
                json!({ "rows": 40, "cols": 120 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, json) = json_response(
            router,
            post_json(
                &format!("/v1/exec/{process_id}/stdin"),
                json!({ "chars": "ribbit\n" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["process_id"], process_id);
        assert_eq!(json["bytes_written"], "ribbit\n".len());

        let process_id = crate::exec::ExecProcessId(process_id.to_string());
        let _ = app_state
            .runtime
            .exec_registry
            .wait(&process_id)
            .await
            .unwrap();
        let read = app_state
            .runtime
            .exec_registry
            .read(&process_id, 0, None)
            .await;
        assert!(read.chunks.iter().any(|chunk| chunk.text.contains("got:")));
    }

    #[tokio::test]
    async fn process_access_rejects_another_workspace() {
        let (_temp, app) = test_app().await;
        let foreign = tempfile::tempdir().unwrap();
        let snapshot = app
            .runtime
            .exec_registry
            .register(
                crate::exec::ExecProcessMeta::new(
                    crate::exec::ExecMode::Background,
                    "foreign".to_string(),
                )
                .with_workspace(foreign.path().to_path_buf()),
                1024,
            )
            .await;
        let router = make_refact_http_server(app);

        let (status, _) = json_response(
            router,
            Request::builder()
                .uri(format!(
                    "/v1/exec/{}/read",
                    snapshot.meta.process_id.as_str()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn subscribe_streams_snapshot_output_and_exit() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app);
        let (status, spawned) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "printf terminal-sse", "pty": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let process_id = spawned["process_id"].as_str().unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/exec/{process_id}/subscribe"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        let collected = tokio::time::timeout(Duration::from_secs(5), async move {
            let mut text = String::new();
            while let Some(chunk) = body.data().await {
                text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
                if text.contains("event: exit") {
                    break;
                }
            }
            text
        })
        .await
        .expect("SSE stream should terminate after exit");

        assert!(collected.contains("event: snapshot"));
        assert!(collected.contains("terminal-sse"));
        assert!(collected.contains("event: exit"));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn subscribe_streams_raw_pty_bytes_with_crlf() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app);
        let (status, spawned) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "printf 'a\\r\\nb\\r\\n'", "pty": true }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let process_id = spawned["process_id"].as_str().unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/exec/{process_id}/subscribe?since_seq=0"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        let collected = tokio::time::timeout(Duration::from_secs(5), async move {
            let mut text = String::new();
            while let Some(chunk) = body.data().await {
                text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
                if text.contains("event: exit") {
                    break;
                }
            }
            text
        })
        .await
        .expect("SSE stream should terminate after exit");

        assert!(collected.contains("event: snapshot"));
        assert!(
            collected.contains("\\r\\n"),
            "SSE payload must keep CRLF byte-faithful for tty: {collected:?}"
        );
        assert!(collected.contains("\"offset\""));
        assert!(collected.contains("event: exit"));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn read_raw_uses_byte_cursor_for_tty_process() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app.clone());
        let (status, spawned) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "printf 'x\\r\\ny\\r\\n'", "pty": true }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let process_id = spawned["process_id"].as_str().unwrap().to_string();
        let exec_process_id = crate::exec::ExecProcessId(process_id.clone());
        app.runtime
            .exec_registry
            .wait(&exec_process_id)
            .await
            .unwrap();

        let (status, read) = json_response(
            router.clone(),
            Request::builder()
                .uri(format!("/v1/exec/{process_id}/read?raw=true&since_seq=0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let chunks = read["chunks"].as_array().unwrap();
        assert_eq!(chunks.len(), 1);
        let text = chunks[0]["text"].as_str().unwrap();
        assert!(
            text.contains("\r\n"),
            "raw read must keep CRLF byte-faithful: {text:?}"
        );
        let next_seq = read["next_seq"].as_u64().unwrap();
        assert!(next_seq > 0);
        assert_eq!(chunks[0]["offset"].as_u64().unwrap(), next_seq);
        assert_eq!(chunks[0]["seq"].as_u64().unwrap(), 0);

        let (status, drained) = json_response(
            router,
            Request::builder()
                .uri(format!(
                    "/v1/exec/{process_id}/read?raw=true&since_seq={next_seq}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(drained["chunks"].as_array().unwrap().is_empty());
        assert_eq!(drained["next_seq"].as_u64().unwrap(), next_seq);
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::parallel(exec_http_env)]
    async fn read_raw_flag_is_ignored_for_non_tty_process() {
        let (_temp, app) = test_app().await;
        let router = make_refact_http_server(app.clone());
        let (status, spawned) = json_response(
            router.clone(),
            post_json(
                "/v1/exec/spawn",
                json!({ "command": "printf plain-hello", "pty": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let process_id = spawned["process_id"].as_str().unwrap().to_string();
        app.runtime
            .exec_registry
            .wait(&crate::exec::ExecProcessId(process_id.clone()))
            .await
            .unwrap();

        let (status, read) = json_response(
            router,
            Request::builder()
                .uri(format!("/v1/exec/{process_id}/read?raw=true&since_seq=0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let chunks = read["chunks"].as_array().unwrap();
        assert!(chunks
            .iter()
            .any(|chunk| chunk["text"].as_str().unwrap().contains("plain-hello")));
        assert!(chunks.iter().all(|chunk| chunk.get("offset").is_none()));
    }
}
