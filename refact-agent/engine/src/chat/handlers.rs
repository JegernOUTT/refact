use axum::extract::Path;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use futures::StreamExt;
use hyper::{body::Bytes, Body};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::{broadcast, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::task::AbortOnDropHandle;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::custom_error::ScratchError;

use super::content::{validate_content_with_attachments, validate_context_files};
use super::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use super::queue::process_command_queue;
use super::queue::{
    add_mode_switch_event_and_plan_if_changed, resolve_worktree_setparams_update,
    worktree_activation_message, worktree_disabled_message,
};
use super::session::{get_or_create_session_with_trajectory, snapshot_with_agents_mode};
use super::trajectories::validate_trajectory_id;
use super::trajectory_ops::sanitize_messages_for_model_switch;
use super::types::*;
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

#[derive(serde::Deserialize)]
pub struct QueuePriorityPatch {
    priority: bool,
}

const SNAPSHOT_CHUNK_TARGET_BYTES: usize = 1024 * 1024;
const SNAPSHOT_CHUNK_MAX_MESSAGES: usize = 32;
const SNAPSHOTS_PER_RESPONSE: usize = 4;

static SNAPSHOT_PREPARATIONS: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(4)));
static SNAPSHOT_ENCODERS: Semaphore = Semaphore::const_new(2);
type SnapshotResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct SnapshotBootstrap {
    initial: Bytes,
    chunks: Option<SnapshotChunks>,
}

struct SnapshotChunks {
    messages: std::vec::IntoIter<ChatMessage>,
    lengths: Vec<usize>,
}

#[derive(Serialize)]
struct SnapshotChunkEnvelope<'a> {
    chat_id: &'a str,
    #[serde(serialize_with = "serialize_snapshot_seq")]
    seq: u64,
    #[serde(rename = "type")]
    event_type: &'static str,
    chunk_index: usize,
    chunk_count: usize,
    messages: &'a [ChatMessage],
}

#[derive(Serialize)]
struct SnapshotEndEnvelope<'a> {
    chat_id: &'a str,
    #[serde(serialize_with = "serialize_snapshot_seq")]
    seq: u64,
    #[serde(rename = "type")]
    event_type: &'static str,
    chunk_count: usize,
    message_count: usize,
}

fn serialize_snapshot_seq<S>(seq: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&seq.to_string())
}

struct SnapshotWriter<'a, W> {
    writer: W,
    cancelled: &'a AtomicBool,
}

impl<W: Write> Write for SnapshotWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(io::Error::other("snapshot encoding cancelled"));
        }
        self.writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[derive(Default)]
struct SnapshotByteCount(usize);

impl Write for SnapshotByteCount {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > SNAPSHOT_CHUNK_TARGET_BYTES {
            return Err(io::Error::other("oversized snapshot message"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn snapshot_frame(value: &impl Serialize, cancelled: &AtomicBool) -> serde_json::Result<Bytes> {
    let mut bytes = b"data: ".to_vec();
    serde_json::to_writer(
        SnapshotWriter {
            writer: &mut bytes,
            cancelled,
        },
        value,
    )?;
    bytes.extend_from_slice(b"\n\n");
    Ok(Bytes::from(bytes))
}

struct CancelSnapshotOnDrop(Arc<AtomicBool>);

impl Drop for CancelSnapshotOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

async fn snapshot_work<T: Send + 'static>(
    work: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> SnapshotResult<T> {
    let permit = SNAPSHOT_ENCODERS.acquire().await?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelSnapshotOnDrop(cancelled.clone());
    let job = AbortOnDropHandle::new(tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work(&cancelled)
    }));
    Ok(job.await?)
}

#[derive(Default)]
struct SnapshotChunkPlan {
    lengths: Vec<usize>,
    index: usize,
    current_count: usize,
    current_bytes: usize,
}

impl SnapshotChunkPlan {
    fn measure_batch(&mut self, messages: &[ChatMessage], cancelled: &AtomicBool) {
        let mut batch_bytes = 0;
        for message in messages[self.index..]
            .iter()
            .take(SNAPSHOT_CHUNK_MAX_MESSAGES)
        {
            let mut size = SnapshotByteCount::default();
            let result = serde_json::to_writer(
                SnapshotWriter {
                    writer: &mut size,
                    cancelled,
                },
                message,
            );
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            let bytes = if size.0 > SNAPSHOT_CHUNK_TARGET_BYTES || result.is_ok() {
                size.0
            } else {
                SNAPSHOT_CHUNK_TARGET_BYTES
            };
            if self.current_count != 0
                && (self.current_count == SNAPSHOT_CHUNK_MAX_MESSAGES
                    || self.current_bytes.saturating_add(bytes) > SNAPSHOT_CHUNK_TARGET_BYTES)
            {
                self.lengths.push(self.current_count);
                self.current_count = 0;
                self.current_bytes = 0;
            }
            self.current_bytes = self.current_bytes.saturating_add(bytes);
            self.current_count += 1;
            self.index += 1;
            batch_bytes += bytes.min(SNAPSHOT_CHUNK_TARGET_BYTES);
            if batch_bytes >= SNAPSHOT_CHUNK_TARGET_BYTES {
                break;
            }
        }
    }

    fn finish(mut self) -> Vec<usize> {
        if self.current_count != 0 {
            self.lengths.push(self.current_count);
        }
        self.lengths
    }
}

async fn serialize_snapshot_bootstrap(
    chat_id: &Arc<str>,
    seq: u64,
    event: ChatEvent,
    messages: Option<Vec<ChatMessage>>,
) -> SnapshotResult<SnapshotBootstrap> {
    let chunks = if let Some(mut messages) = messages {
        let mut plan = SnapshotChunkPlan::default();
        while plan.index < messages.len() {
            #[cfg(test)]
            let sizing_gate = tests::take_snapshot_sizing_gate(chat_id, seq);
            (plan, messages) = snapshot_work(move |cancelled| {
                #[cfg(test)]
                if let Some(gate) = sizing_gate {
                    gate.park();
                }
                plan.measure_batch(&messages, cancelled);
                (plan, messages)
            })
            .await?;
        }
        Some(SnapshotChunks {
            messages: messages.into_iter(),
            lengths: plan.finish(),
        })
    } else {
        None
    };

    let envelope = EventEnvelope {
        chat_id: chat_id.to_string(),
        seq,
        event,
    };
    let counts = chunks
        .as_ref()
        .map(|chunks| (chunks.lengths.len(), chunks.messages.len()));
    let initial = snapshot_work(move |cancelled| {
        if let Some((chunk_count, message_count)) = counts {
            let mut initial = serde_json::to_value(envelope)?;
            if let Some(object) = initial.as_object_mut() {
                object.insert("snapshot_chunked".into(), serde_json::json!(true));
                object.insert("chunk_count".into(), serde_json::json!(chunk_count));
                object.insert("message_count".into(), serde_json::json!(message_count));
            }
            snapshot_frame(&initial, cancelled)
        } else {
            snapshot_frame(&envelope, cancelled)
        }
    })
    .await??;
    Ok(SnapshotBootstrap {
        initial,
        chunks,
    })
}

fn snapshot_frames(
    bootstrap: SnapshotBootstrap,
    chat_id: Arc<str>,
    seq: u64,
) -> impl futures::Stream<Item = SnapshotResult<Bytes>> + Send {
    async_stream::try_stream! {
        yield bootstrap.initial;
        if let Some(SnapshotChunks { mut messages, lengths }) = bootstrap.chunks {
            let chunk_count = lengths.len();
            let message_count = messages.len();
            for (chunk_index, count) in lengths.into_iter().enumerate() {
                let chat_id = chat_id.clone();
                let frame;
                let encode = snapshot_work(move |cancelled| {
                    let frame = snapshot_frame(&SnapshotChunkEnvelope {
                        chat_id: &chat_id,
                        seq,
                        event_type: "snapshot_chunk",
                        chunk_index,
                        chunk_count,
                        messages: &messages.as_slice()[..count],
                    }, cancelled)?;
                    messages.by_ref().take(count).for_each(drop);
                    Ok::<_, serde_json::Error>((messages, frame))
                });
                (messages, frame) = AbortOnDropHandle::new(tokio::spawn(encode)).await???;
                yield frame;
            }
            yield snapshot_frame(&SnapshotEndEnvelope {
                chat_id: &chat_id,
                seq,
                event_type: "snapshot_end",
                chunk_count,
                message_count,
            }, &AtomicBool::new(false))?;
        }
    }
}

struct PreparedSubscriptionSnapshot {
    rx: broadcast::Receiver<Arc<String>>,
    seq: u64,
    closed: Arc<AtomicBool>,
    bootstrap: SnapshotBootstrap,
}

async fn prepare_subscription_snapshot(
    app: &AppState,
    session: &Arc<Mutex<ChatSession>>,
    chat_id: &Arc<str>,
    chunked: bool,
    recovery: bool,
    response_slots: &Arc<Semaphore>,
    preparations: Arc<Semaphore>,
) -> SnapshotResult<Option<(PreparedSubscriptionSnapshot, OwnedSemaphorePermit)>> {
    let response_slot = response_slots.clone().acquire_owned().await?;
    let job = prepare_snapshot_job(
        app.clone(),
        session.clone(),
        chat_id.clone(),
        chunked,
        recovery,
        preparations,
    );
    Ok(AbortOnDropHandle::new(tokio::spawn(job))
        .await??
        .map(|snapshot| (snapshot, response_slot)))
}

async fn prepare_snapshot_job(
    app: AppState,
    session: Arc<Mutex<ChatSession>>,
    chat_id: Arc<str>,
    chunked: bool,
    recovery: bool,
    preparations: Arc<Semaphore>,
) -> SnapshotResult<Option<PreparedSubscriptionSnapshot>> {
    let (_preparing, rx, seq, closed, messages, snapshot) = loop {
        let preparing = preparations.acquire().await?;
        let captured = snapshot_work({
            let session = session.clone();
            let app = app.clone();
            move |cancelled| {
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let session = session.try_lock()?;
                if recovery && session.closed {
                    return Ok(None);
                }
                // Subscribe before reading the sequence and capturing the immutable snapshot.
                let rx = session.subscribe();
                let seq = session.event_seq;
                let messages = chunked.then(|| session.snapshot_messages());
                let snapshot = snapshot_with_agents_mode(app, &session, !chunked);
                Ok::<_, tokio::sync::TryLockError>(Some((
                    rx,
                    seq,
                    session.closed_flag.clone(),
                    messages,
                    snapshot,
                )))
            }
        })
        .await?;
        match captured {
            Ok(Some((rx, seq, closed, messages, snapshot))) => {
                break (preparing, rx, seq, closed, messages, snapshot);
            }
            Ok(None) => return Ok(None),
            Err(_) => {
                drop(preparing);
                drop(session.lock().await);
            }
        }
    };
    let (snapshot, background_agents) = snapshot.await;
    session
        .lock()
        .await
        .upsert_background_agents(background_agents);

    let started_at = perf_diagnostics::is_enabled().then(Instant::now);
    let result = serialize_snapshot_bootstrap(&chat_id, seq, snapshot, messages).await;
    if let Some(started_at) = started_at {
        record_sse_serialize(
            &chat_id,
            if result.is_ok() {
                PerfOutcome::Success
            } else {
                PerfOutcome::Failure
            },
            started_at
                .elapsed()
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX),
            result
                .as_ref()
                .ok()
                .map(|bootstrap| (bootstrap.initial.len() - b"data: \n\n".len()) as u64),
        );
    }
    Ok(Some(PreparedSubscriptionSnapshot {
        rx,
        seq,
        closed,
        bootstrap: result?,
    }))
}

fn command_error_response(status: StatusCode, code: &str, error: String) -> Response<Body> {
    let body = serde_json::to_string(&serde_json::json!({
        "code": code,
        "error": error,
    }))
    .unwrap_or_else(|_| r#"{"code":"command_error","error":"command failed"}"#.to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn spawn_pending_background_agent_flush(app: AppState, chat_id: String) {
    tokio::spawn(async move {
        let _ = crate::agents::push::flush_pending_pushes_for_parent(app, &chat_id).await;
    });
}

fn record_lagged_recovery(chat_id: &str, outcome: PerfOutcome, elapsed_us: u64, skipped: u64) {
    perf_diagnostics::record(
        PerfComponent::SseLagged,
        Some(chat_id),
        outcome,
        elapsed_us,
        None,
        Some(skipped),
        None,
    );
}

fn record_sse_serialize(
    chat_id: &str,
    outcome: PerfOutcome,
    elapsed_us: u64,
    size_bytes: Option<u64>,
) {
    perf_diagnostics::record(
        PerfComponent::SseSerialize,
        Some(chat_id),
        outcome,
        elapsed_us,
        size_bytes,
        None,
        None,
    );
}

const SUBSCRIPTION_CLOSED_MESSAGE_MAX_CHARS: usize = 160;

#[derive(Serialize)]
struct SubscriptionClosedEnvelope<'a> {
    chat_id: &'a str,
    #[serde(rename = "type")]
    event_type: &'static str,
    reason: &'static str,
    retryable: bool,
    message: String,
}

/// Builds the terminal multiplex frame for one chat. The message is derived only from the
/// HTTP status class, never from the underlying error text, so it cannot leak paths or secrets.
fn subscription_closed_frame(
    chat_id: &str,
    reason: &'static str,
    retryable: bool,
    message: String,
) -> String {
    let message: String = message
        .chars()
        .take(SUBSCRIPTION_CLOSED_MESSAGE_MAX_CHARS)
        .collect();
    let json = serde_json::to_string(&SubscriptionClosedEnvelope {
        chat_id,
        event_type: "subscription_closed",
        reason,
        retryable,
        message,
    })
    .unwrap_or_else(|_| {
        format!(
            r#"{{"chat_id":{},"type":"subscription_closed","reason":"{}","retryable":{},"message":"subscription closed"}}"#,
            serde_json::Value::String(chat_id.to_string()),
            reason,
            retryable
        )
    });
    format!("data: {}\n\n", json)
}

fn subscription_retryable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn subscription_unavailable_frame(chat_id: &str, error: &ScratchError) -> String {
    let status = error.status_code;
    let retryable = subscription_retryable(status);
    let message = format!(
        "chat subscription unavailable: {} {}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("error")
    );
    subscription_closed_frame(chat_id, "unavailable", retryable, message)
}

type MultiplexItem = Result<hyper::body::Bytes, Box<dyn std::error::Error + Send + Sync>>;

/// Multiplexes per-chat subscription bodies. With `per_chat_close`, a chat whose stream ends
/// or cannot be prepared emits exactly one `subscription_closed` frame and finishes without an
/// `Err` item, so the remaining chats keep streaming and the response reaches EOF once every
/// chat has finished. Without it, such a chat yields an `Err` item that aborts the response.
fn multiplex_chat_subscriptions<F, Fut>(
    chat_ids: Vec<String>,
    per_chat_close: bool,
    subscribe: F,
) -> Body
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Response<Body>, ScratchError>> + Send + 'static,
{
    type ChatBodyStream = std::pin::Pin<Box<dyn futures::Stream<Item = MultiplexItem> + Send>>;

    let streams = chat_ids.into_iter().map(|id| {
        let prepare = subscribe(id.clone());
        Box::pin(async_stream::stream! {
            match prepare.await {
                Ok(response) => {
                    let mut body = response.into_body();
                    while let Some(chunk) = body.next().await {
                        match chunk {
                            Ok(bytes) => yield Ok::<_, Box<dyn std::error::Error + Send + Sync>>(bytes),
                            Err(error) if per_chat_close => {
                                tracing::warn!(chat_id = %id, %error, "multiplex chat subscription body failed");
                                break;
                            }
                            Err(error) => yield Err(Box::new(error) as Box<dyn std::error::Error + Send + Sync>),
                        }
                    }
                    if per_chat_close {
                        yield Ok(hyper::body::Bytes::from(subscription_closed_frame(
                            &id,
                            "ended",
                            true,
                            "chat subscription ended".to_string(),
                        )));
                    } else {
                        yield Err(Box::new(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "chat subscription ended")) as Box<dyn std::error::Error + Send + Sync>);
                    }
                }
                Err(error) => {
                    tracing::warn!(chat_id = %id, %error, "multiplex chat subscription preparation failed");
                    if per_chat_close {
                        yield Ok(hyper::body::Bytes::from(subscription_unavailable_frame(&id, &error)));
                    } else {
                        yield Err(Box::new(std::io::Error::other("chat subscription preparation failed")) as Box<dyn std::error::Error + Send + Sync>);
                    }
                }
            }
        }) as ChatBodyStream
    });
    Body::wrap_stream(futures::stream::select_all(streams))
}

pub async fn handle_v1_chats_subscribe_many(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response<Body>, ScratchError> {
    let chat_ids: Vec<String> = params
        .get("chat_ids")
        .and_then(|value| serde_json::from_str(value).ok())
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::BAD_REQUEST,
                "chat_ids must be a JSON array".into(),
            )
        })?;
    if chat_ids.is_empty() || chat_ids.len() > 128 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "chat_ids must contain 1 to 128 IDs".into(),
        ));
    }
    for id in &chat_ids {
        validate_trajectory_id(id)?;
    }
    let mut seen = std::collections::HashSet::new();
    let chat_ids: Vec<String> = chat_ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();
    let per_chat_close = params
        .get("per_chat_close")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    let response_slots = Arc::new(Semaphore::new(SNAPSHOTS_PER_RESPONSE));
    let body = multiplex_chat_subscriptions(chat_ids, per_chat_close, move |id| {
        let app = app.clone();
        let mut params = params.clone();
        params.insert("chat_id".into(), id);
        subscribe_chat(app, params, response_slots.clone())
    });
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(body)
        .unwrap())
}

pub async fn handle_v1_chat_subscribe(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response<Body>, ScratchError> {
    subscribe_chat(app, params, Arc::new(Semaphore::new(1))).await
}

async fn subscribe_chat(
    app: AppState,
    params: HashMap<String, String>,
    response_slots: Arc<Semaphore>,
) -> Result<Response<Body>, ScratchError> {
    let chat_id = params
        .get("chat_id")
        .ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "chat_id required".to_string()))?
        .clone();
    let chunked_snapshot = params
        .get("snapshot_chunks")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    validate_trajectory_id(&chat_id)?;

    let sessions = app.chat.sessions.clone();

    let session_arc =
        get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await?;
    if let Err(error) = crate::agents::delivery::recover_runner_deliveries(&app, &chat_id).await {
        tracing::warn!(%chat_id, %error, "runner delivery recovery failed");
    }
    let (needs_task_goal, version) = {
        let session = session_arc.lock().await;
        (
            session.task_goal_messages.is_none(),
            session.trajectory_version,
        )
    };
    if needs_task_goal {
        if let Err(error) =
            crate::tasks::goal::hydrate_session_task_goal(app.gcx.clone(), &session_arc).await
        {
            tracing::warn!(%chat_id, %error, "Task goal hydration before snapshot refused");
        } else if session_arc.lock().await.trajectory_version != version {
            super::trajectories::maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        }
    }
    spawn_pending_background_agent_flush(app.clone(), chat_id.clone());
    let chat_id = Arc::<str>::from(chat_id);
    let (
        PreparedSubscriptionSnapshot {
            mut rx,
            seq,
            closed,
            bootstrap,
        },
        response_slot,
    ) = match prepare_subscription_snapshot(
        &app,
        &session_arc,
        &chat_id,
        chunked_snapshot,
        false,
        &response_slots,
        SNAPSHOT_PREPARATIONS.clone(),
    )
    .await
    {
        Ok(Some(snapshot)) => snapshot,
        result => {
            tracing::error!(%chat_id, error = ?result.err(), "Failed to serialize initial SSE snapshot");
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "snapshot serialization failed".to_string(),
            ));
        }
    };

    let stream = async_stream::stream! {
        {
            let _response_slot = response_slot;
            let frames = snapshot_frames(bootstrap, chat_id.clone(), seq);
            tokio::pin!(frames);
            while let Some(frame) = frames.next().await {
                match frame {
                    Ok(bytes) => yield Ok::<_, std::convert::Infallible>(bytes),
                    Err(error) => {
                        tracing::error!(%chat_id, %error, "Failed to stream initial SSE snapshot");
                        return;
                    }
                }
            }
        }

        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(json) => yield Ok(Bytes::from(format!("data: {}\n\n", json))),
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            let started_at = perf_diagnostics::is_enabled().then(Instant::now);
                            tracing::info!(%chat_id, skipped, "SSE subscriber lagged, sending fresh snapshot");
                            let start = prepare_subscription_snapshot(&app, &session_arc, &chat_id, chunked_snapshot, true, &response_slots, SNAPSHOT_PREPARATIONS.clone()).await;
                            if let Some(started_at) = started_at.filter(|_| !matches!(&start, Ok(None))) {
                                record_lagged_recovery(
                                    &chat_id,
                                    if start.is_ok() { PerfOutcome::Success } else { PerfOutcome::Failure },
                                    started_at.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
                                    skipped,
                                );
                            }
                            let (PreparedSubscriptionSnapshot { rx: next_rx, seq, bootstrap, .. }, _response_slot) = match start {
                                Ok(Some(snapshot)) => snapshot,
                                Ok(None) => break,
                                Err(error) => {
                                    tracing::error!(%chat_id, %error, "Failed to serialize SSE recovery snapshot");
                                    break;
                                }
                            };
                            rx = next_rx;
                            let frames = snapshot_frames(bootstrap, chat_id.clone(), seq);
                            tokio::pin!(frames);
                            while let Some(frame) = frames.next().await {
                                match frame {
                                    Ok(bytes) => yield Ok(bytes),
                                    Err(error) => {
                                        tracing::error!(%chat_id, %error, "Failed to stream SSE recovery snapshot");
                                        return;
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat_interval.tick() => {
                    if closed.load(Ordering::Relaxed) {
                        break;
                    }
                    yield Ok(Bytes::from(format!(": hb {}\n\n", chrono::Utc::now().timestamp())));
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

pub async fn handle_v1_chat_app_workspace(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;
    let request: CommandRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    if !matches!(request.command, ChatCommand::SetAppWorkspace { .. }) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Only set_app_workspace is accepted by this endpoint".to_string(),
        ));
    }
    drop(request);
    handle_v1_chat_command(State(app), Path(chat_id), body_bytes).await
}

/// A pause or stop of the goal also cancels a waiting or attempting recovery, which could revive it.
pub(crate) fn cancel_recovery_for_goal_stop(session: &mut ChatSession) {
    if !(session.runtime.recovery.as_ref().is_some_and(|r| {
        matches!(
            r.state,
            refact_core::recovery::RecoveryState::Waiting
                | refact_core::recovery::RecoveryState::Attempting
        )
    }) && session.goal.is_some())
    {
        return;
    }
    if let Some(budget) = &session.recovery_budget {
        budget.cancel();
    }
    session.preserve_interrupted_recovery_output();
    if let Some(mut record) = session.runtime.recovery.clone() {
        record.state = refact_core::recovery::RecoveryState::Cancelled;
        record.revision = record.revision.saturating_add(1);
        record.next_retry_at_ms = None;
        record.terminal_reason = Some("goal_control".into());
        session.update_recovery(Some(record));
    }
    session.abort_stream();
}

/// Stops the current generation of a locked chat, as the Abort command of the command route does;
/// true when the trajectory must be saved after the lock is released.
pub(crate) fn abort_chat_locked(session: &mut ChatSession, request_id: &str) -> bool {
    session.apply_priority_abort();
    let should_save_trajectory = session.trajectory_dirty;
    session.remember_accepted_request(request_id);
    session.emit(ChatEvent::Ack {
        client_request_id: request_id.to_string(),
        accepted: true,
        result: Some(serde_json::json!({"aborted": true})),
    });
    should_save_trajectory
}

/// Removes one queued command, as the cancel-queued route does; false when it was not queued.
pub(crate) fn cancel_queued_locked(
    session: &mut ChatSession,
    chat_id: &str,
    client_request_id: &str,
) -> bool {
    let initial_len = session.command_queue.len();
    session
        .command_queue
        .retain(|r| r.client_request_id != client_request_id);
    session.command_enqueued_at.remove(client_request_id);
    session.queued_user_merge_targets.remove(client_request_id);
    if session.command_queue.len() < initial_len {
        super::app_context::release(chat_id, client_request_id);
        session.touch();
        session.emit_queue_update();
        session.queue_notify.notify_one();
        true
    } else {
        false
    }
}

pub async fn handle_v1_chat_command(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;

    let request: CommandRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    if matches!(request.command, ChatCommand::PatchMessages { .. }) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "patch_messages is queued only by chat.patch of a python script in that chat".into(),
        ));
    }

    let goal_policy = if matches!(
        &request.command,
        ChatCommand::SetGoal { .. } | ChatCommand::UpdateGoal { .. }
    ) {
        Some(super::verify_cmd::VerifyCommandPolicy::load(app.gcx.clone()).await)
    } else {
        None
    };
    let sessions = app.chat.sessions.clone();

    let session_arc =
        get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await?;
    if let Err(error) = crate::agents::delivery::recover_runner_deliveries(&app, &chat_id).await {
        tracing::warn!(%chat_id, %error, "runner delivery recovery failed");
    }
    spawn_pending_background_agent_flush(app.clone(), chat_id.clone());
    let _app_workspace_lease = if matches!(request.command, ChatCommand::SetAppWorkspace { .. }) {
        Some(super::app_workspace::AppWorkspaceMutationLease::acquire(session_arc.clone()).await)
    } else {
        None
    };
    let mut session = session_arc.lock().await;
    let _native_mutation =
        super::session::NativeMutationGuard::enter_locked(&session_arc, &mut session)
            .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;

    if crate::buddy::companion::is_companion(&session.thread)
        && !crate::buddy::companion::command_allowed(&request.command)
    {
        let error =
            "Companion conversations cannot change execution settings or restore another session"
                .to_string();
        super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
        return Err(ScratchError::new(StatusCode::FORBIDDEN, error));
    }

    if session.has_seen_request(&request.client_request_id) {
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id.clone(),
            accepted: true,
            result: Some(serde_json::json!({"duplicate": true})),
        });
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"duplicate"}"#))
            .unwrap());
    }

    if let Err(error) = super::queue::prepare_guarded_command(&mut session, &request) {
        super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
        return Err(ScratchError::new(StatusCode::CONFLICT, error));
    }

    if let Some(policy) = &goal_policy {
        let validation = match &request.command {
            ChatCommand::SetGoal {
                criteria, ..
            } => super::evidence::validate_goal_verify_commands(
                criteria.as_deref().unwrap_or_default(),
                policy,
            ),
            _ => Ok(()),
        };
        if let Err(error) = validation {
            super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
            return Ok(command_error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                error,
            ));
        }
    }

    let approval_response = match &request.command {
        ChatCommand::AnswerQuestions {
            tool_call_id,
            answers,
            asked_at_ms,
        } => Some((tool_call_id, answers.clone(), *asked_at_ms, None)),
        ChatCommand::SubmitQuestionResponse {
            tool_call_id,
            asked_at_ms,
            response,
        } => Some((
            tool_call_id,
            Vec::new(),
            Some(*asked_at_ms),
            Some(response.clone()),
        )),
        _ => None,
    };
    if let Some((tool_call_id, answers, asked_at_ms, response)) = approval_response {
        if crate::agents::approval::is_user_approval(tool_call_id) {
            if request.supervision_guard.is_some() {
                return Err(ScratchError::new(
                    StatusCode::FORBIDDEN,
                    "command approval requires an explicit user response".into(),
                ));
            }
            drop(session);
            let result = crate::agents::approval::apply_user_answer(
                &app,
                &session_arc,
                tool_call_id,
                answers,
                asked_at_ms,
                &request.client_request_id,
                response,
            )
            .await;
            let mut session = session_arc.lock().await;
            let accepted = result.is_ok();
            crate::buddy::decision_receipts::transition(
                &mut session,
                &request.client_request_id,
                if accepted { "applied" } else { "unknown" },
                (!accepted).then(|| {
                    "Original command approval policy did not acknowledge application".into()
                }),
            );
            let payload = match result {
                Ok(()) => serde_json::json!({"applied":true}),
                Err(error) => serde_json::json!({"error":error}),
            };
            drop(session);
            if let Err(error) =
                super::trajectories::flush_trajectory_for_session(app.clone(), session_arc.clone())
                    .await
            {
                session_arc.lock().await.emit(ChatEvent::Ack {
                    client_request_id: request.client_request_id.clone(), accepted: false,
                    result: Some(serde_json::json!({"status":"unknown", "error":"Approval outcome durability is uncertain"})),
                });
                return Err(ScratchError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("Approval outcome durability is uncertain: {error}"),
                ));
            }
            let mut session = session_arc.lock().await;
            if accepted {
                session.remember_accepted_request(&request.client_request_id);
            }
            session.emit(ChatEvent::Ack {
                client_request_id: request.client_request_id,
                accepted,
                result: Some(payload.clone()),
            });
            return Ok(Response::builder()
                .status(if accepted {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_REQUEST
                })
                .header("Content-Type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap());
        }
    }

    // Delivery edits must bypass the command processor: it awaits the running
    // turn, precisely when a user needs to reprioritize or cancel a delivery.
    if matches!(
        &request.command,
        ChatCommand::DeliverMessages { .. } | ChatCommand::UpdatePendingDelivery { .. }
    ) {
        drop(session);
        let result = match request.command {
            ChatCommand::DeliverMessages {
                delivery,
            } => super::delivery::deliver_to_chat(app.clone(), &chat_id, delivery)
                .await
                .map(|outcome| serde_json::json!({"delivery_outcome": outcome})),
            ChatCommand::UpdatePendingDelivery {
                delivery_id,
                push,
                cancel,
            } => super::delivery::update_pending_delivery_in_chat(
                app.clone(),
                &chat_id,
                &delivery_id,
                push,
                cancel,
            )
            .await
            .map(|()| serde_json::json!({"updated": true})),
            _ => unreachable!(),
        };
        let mut session = session_arc.lock().await;
        if result.is_ok() {
            session.remember_accepted_request(&request.client_request_id);
        }
        let accepted = result.is_ok();
        let payload = result.unwrap_or_else(|error| serde_json::json!({"error": error}));
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted,
            result: Some(payload.clone()),
        });
        return Ok(Response::builder()
            .status(if accepted {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            })
            .header("Content-Type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap());
    }

    if let ChatCommand::RecoveryRetryNow {
        ref run_id,
        expected_revision,
    } = request.command
    {
        let accepted = session.runtime.recovery.as_ref().is_some_and(|r| {
            r.run_id == *run_id
                && r.revision == expected_revision
                && r.state == refact_core::recovery::RecoveryState::Waiting
                && (r.manual
                    || session
                        .goal
                        .as_ref()
                        .is_none_or(|goal| goal.active && goal.status == GoalStatus::Active))
                && r.retry_not_before_ms.unwrap_or(0) <= super::recovery::now_ms()
        });
        if accepted {
            if let Some(mut record) = session.runtime.recovery.clone() {
                record.revision = record.revision.saturating_add(1);
                record.next_retry_at_ms = Some(super::recovery::now_ms());
                session.update_recovery(Some(record));
            }
            session.remember_accepted_request(&request.client_request_id);
            session.recovery_notify.notify_one();
        }
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted,
            result: None,
        });
        return Ok(Response::builder()
            .status(if accepted {
                StatusCode::OK
            } else {
                StatusCode::CONFLICT
            })
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"accepted": accepted}).to_string(),
            ))
            .unwrap());
    }

    if let ChatCommand::GoalControl {
        action,
    } = &request.command
    {
        if let Err(error) = super::queue::validate_goal_control(&session, action) {
            super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
            return Err(ScratchError::new(StatusCode::BAD_REQUEST, error));
        }
    }

    if matches!(&request.command, ChatCommand::GoalControl { action } if matches!(action.trim().to_ascii_lowercase().as_str(), "pause" | "stop"))
    {
        cancel_recovery_for_goal_stop(&mut session);
    }

    if matches!(
        request.command,
        ChatCommand::PythonApprovalDecision { .. } | ChatCommand::PythonRunControl { .. }
    ) {
        drop(session);
        let result = match &request.command {
            ChatCommand::PythonApprovalDecision {
                request_id,
                decision,
            } => {
                app.python_tool
                    .approvals
                    .decide(&app, &chat_id, request_id, *decision)
                    .await
            }
            ChatCommand::PythonRunControl {
                tool_call_id, ..
            } => app.python_tool.stop_run(&chat_id, tool_call_id),
            _ => unreachable!(),
        };
        let mut session = session_arc.lock().await;
        let accepted = result.is_ok();
        let payload = match result {
            Ok(()) => serde_json::json!({"status":"ok"}),
            Err(error) => serde_json::json!({"error":error}),
        };
        if accepted {
            session.remember_accepted_request(&request.client_request_id);
        }
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted,
            result: Some(payload.clone()),
        });
        return Ok(Response::builder()
            .status(if accepted {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            })
            .header("Content-Type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap());
    }

    if matches!(request.command, ChatCommand::Abort {}) {
        let should_save_trajectory = abort_chat_locked(&mut session, &request.client_request_id);
        drop(session);
        if should_save_trajectory {
            super::trajectories::maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        }
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"aborted"}"#))
            .unwrap());
    }

    if let ChatCommand::SetAppWorkspace {
        expected_revision,
        ref workspace,
    } = request.command
    {
        let result = super::app_workspace::apply_set_app_workspace_locked(
            &mut session,
            expected_revision,
            workspace.clone(),
        );
        let (status, payload) = match &result {
            Ok(stored) => {
                session.remember_accepted_request(&request.client_request_id);
                (
                    StatusCode::OK,
                    serde_json::json!({"revision": stored.revision, "app_workspace": stored}),
                )
            }
            Err(
                error @ refact_chat_api::AppWorkspaceSetError::Conflict {
                    ..
                },
            ) => (StatusCode::CONFLICT, error.to_json()),
            Err(error) => (StatusCode::BAD_REQUEST, error.to_json()),
        };
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: result.is_ok(),
            result: Some(payload.clone()),
        });
        drop(session);
        if result.is_ok() {
            super::trajectories::maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        }
        return Ok(Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap());
    }

    if let ChatCommand::SetParams {
        ref patch,
    } = request.command
    {
        if let Err(error) = super::recovery::validate_policy_patch(patch) {
            session.emit(ChatEvent::Ack {
                client_request_id: request.client_request_id,
                accepted: false,
                result: Some(serde_json::json!({"error": error})),
            });
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::json!({"error": error}).to_string()))
                .unwrap());
        }
        if !patch.is_object() {
            let error = "SetParams patch must be an object".to_string();
            session.emit(ChatEvent::Ack {
                client_request_id: request.client_request_id,
                accepted: false,
                result: Some(serde_json::json!({"error": error.clone()})),
            });
            return Ok(command_error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                error,
            ));
        }
        let mode_switch_reason = patch
            .get("reason")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let scope_patch = super::worktree_lifecycle::is_scope_patch(patch);
        drop(session);
        let _scope_transition = if scope_patch {
            Some(super::worktree_lifecycle::transition_lock(&session_arc).await)
        } else {
            None
        };
        let thread_before = session_arc.lock().await.thread.clone();
        if let Err(error) =
            super::queue::check_worktree_setparams_authority(&app, &chat_id, &thread_before, patch)
                .await
        {
            let mut session = session_arc.lock().await;
            session.emit(ChatEvent::Ack {
                client_request_id: request.client_request_id,
                accepted: false,
                result: Some(serde_json::json!({"error": error.clone()})),
            });
            return Ok(command_error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                error,
            ));
        }
        let mut session = session_arc.lock().await;
        if super::worktree_lifecycle::changes_attachment(&session.thread, patch) {
            if let Err(error) = super::worktree_lifecycle::check_idle_attachment(&session) {
                super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
                return Ok(command_error_response(StatusCode::CONFLICT, "busy", error));
            }
        }
        let thread_before = session.thread.clone();
        drop(session);
        let worktree_update =
            match resolve_worktree_setparams_update(app.clone(), &chat_id, &thread_before, patch)
                .await
            {
                Ok(update) => update,
                Err(e) => {
                    let mut session = session_arc.lock().await;
                    session.emit(ChatEvent::Ack {
                        client_request_id: request.client_request_id,
                        accepted: false,
                        result: Some(serde_json::json!({"error": e.clone()})),
                    });
                    return Ok(command_error_response(
                        StatusCode::BAD_REQUEST,
                        "bad_request",
                        e,
                    ));
                }
            };
        let mut session = session_arc.lock().await;
        if scope_patch {
            if let Err(error) =
                super::worktree_lifecycle::check_unchanged_attachment(&session, &thread_before)
            {
                super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
                return Ok(command_error_response(
                    StatusCode::CONFLICT,
                    "scope_changed",
                    error,
                ));
            }
        }
        if super::worktree_lifecycle::changes_attachment(&session.thread, patch) {
            if let Err(error) = super::worktree_lifecycle::check_idle_attachment(&session) {
                super::queue::reject_guard(&mut session, &request.client_request_id, error.clone());
                return Ok(command_error_response(
                    StatusCode::CONFLICT,
                    "scope_changed",
                    error,
                ));
            }
        }
        let old_model = session.thread.model.clone();
        let old_mode = session.thread.mode.clone();
        let (mut changed, sanitized_patch) =
            super::queue::apply_setparams_patch(&mut session.thread, patch);
        let worktree_message = worktree_update
            .as_ref()
            .filter(|update| update.changed)
            .map(|update| match update.worktree.as_ref() {
                Some(worktree) => worktree_activation_message(worktree),
                None => worktree_disabled_message(update.previous_worktree.as_ref()),
            });
        if let Some(update) = worktree_update.clone() {
            session.thread.worktree = update.worktree;
            changed |= update.changed;
        }

        let mode_in_patch = patch.get("mode").and_then(|v| v.as_str());
        if let Some(mode_str) = mode_in_patch {
            let normalized_mode = map_legacy_mode_to_id(mode_str);
            if session.thread.mode != normalized_mode {
                session.thread.mode = normalized_mode.to_string();
                changed = true;
            }
        }

        let mode_changed = session.thread.mode != old_mode;
        if mode_changed {
            let model_id = if session.thread.model.is_empty() {
                None
            } else {
                Some(session.thread.model.as_str())
            };
            if let Some(mode_config) =
                get_mode_config(app.gcx.clone(), &session.thread.mode, model_id).await
            {
                let defaults = &mode_config.thread_defaults;
                if let Some(v) = defaults.include_project_info {
                    if session.thread.include_project_info != v {
                        session.thread.include_project_info = v;
                        changed = true;
                    }
                }
                if let Some(v) = defaults.checkpoints_enabled {
                    if session.thread.checkpoints_enabled != v {
                        session.thread.checkpoints_enabled = v;
                        changed = true;
                    }
                }
                if let Some(v) = defaults.auto_approve_editing_tools {
                    if session.thread.auto_approve_editing_tools != v {
                        session.thread.auto_approve_editing_tools = v;
                        changed = true;
                    }
                }
                if let Some(v) = defaults.auto_approve_dangerous_commands {
                    if session.thread.auto_approve_dangerous_commands != v {
                        session.thread.auto_approve_dangerous_commands = v;
                        changed = true;
                    }
                }
            }
        }

        if session.thread.model != old_model {
            sanitize_messages_for_model_switch(&mut session.messages);
        }
        if session.thread.model != old_model || mode_changed {
            session.release_turn_only_state();
        }
        let title_in_patch = patch.get("title").and_then(|v| v.as_str());
        let is_gen_in_patch = patch.get("is_title_generated").and_then(|v| v.as_bool());
        if let Some(title) = title_in_patch {
            let is_generated = is_gen_in_patch.unwrap_or(false);
            session.set_title(title.to_string(), is_generated);
        } else if let Some(is_gen) = is_gen_in_patch {
            if session.thread.is_title_generated != is_gen {
                session.thread.is_title_generated = is_gen;
                let title = session.thread.title.clone();
                session.set_title(title, is_gen);
            }
        }

        let mut patch_for_chat_sse = sanitized_patch;
        if let Some(obj) = patch_for_chat_sse.as_object_mut() {
            obj.remove("title");
            obj.remove("is_title_generated");
            if let Some(update) = worktree_update {
                obj.insert("worktree".to_string(), update.sse_value);
            }
            if mode_changed {
                obj.insert("mode".to_string(), serde_json::json!(session.thread.mode));
                obj.insert(
                    "include_project_info".to_string(),
                    serde_json::json!(session.thread.include_project_info),
                );
                obj.insert(
                    "checkpoints_enabled".to_string(),
                    serde_json::json!(session.thread.checkpoints_enabled),
                );
                obj.insert(
                    "auto_approve_editing_tools".to_string(),
                    serde_json::json!(session.thread.auto_approve_editing_tools),
                );
                obj.insert(
                    "auto_approve_dangerous_commands".to_string(),
                    serde_json::json!(session.thread.auto_approve_dangerous_commands),
                );
            }
        }
        session.emit(ChatEvent::ThreadUpdated {
            params: patch_for_chat_sse,
        });
        if changed {
            session.increment_version();
            session.touch();
        }
        if mode_changed {
            add_mode_switch_event_and_plan_if_changed(
                app.clone(),
                &mut session,
                &old_mode,
                mode_switch_reason.as_deref(),
                "chat.session",
            )
            .await;
        }
        if let Some(message) = worktree_message {
            session.add_message(message);
        }
        session.remember_accepted_request(&request.client_request_id);
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: true,
            result: Some(serde_json::json!({"applied": true})),
        });
        drop(session);
        if changed {
            super::trajectories::maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        }
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"applied"}"#))
            .unwrap());
    }

    let validation_error = match &request.command {
        ChatCommand::UserMessage {
            content,
            attachments,
            context_files,
            client_message_id,
            suppress_auto_enrichment: _,
            app_context_ids: _,
        } => validate_content_with_attachments(content, attachments)
            .err()
            .or_else(|| validate_context_files(context_files).err())
            .or_else(|| {
                refact_chat_api::validate_client_message_id(client_message_id.as_deref()).err()
            })
            .or_else(|| super::app_context::admit(&chat_id, &request).err()),
        ChatCommand::RetryFromIndex {
            content,
            attachments,
            ..
        } => validate_content_with_attachments(content, attachments).err(),
        ChatCommand::UpdateMessage {
            content,
            attachments,
            ..
        } => validate_content_with_attachments(content, attachments).err(),
        _ => None,
    };

    if let Some(error) = validation_error {
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: false,
            result: Some(serde_json::json!({"error": error})),
        });
        let body = serde_json::to_string(&serde_json::json!({
            "status": "invalid_content",
            "error": error
        }))
        .unwrap_or_else(|_| r#"{"status":"invalid_content"}"#.to_string());
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap());
    }

    let goal_advisory = if matches!(&request.command, ChatCommand::UpdateGoal { .. }) {
        goal_policy
            .as_ref()
            .map(|policy| super::evidence::unverifiable_goal_criteria(&session, policy))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let client_request_id_for_ack = request.client_request_id.clone();
    let enqueue_outcome = session.enqueue_accepted_command(request);
    if matches!(
        enqueue_outcome,
        EnqueueCommandOutcome::Full | EnqueueCommandOutcome::Rejected
    ) {
        super::app_context::release(&chat_id, &client_request_id_for_ack);
    }
    if enqueue_outcome == EnqueueCommandOutcome::Rejected {
        return Ok(Response::builder()
            .status(StatusCode::CONFLICT)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"authority_changed","error":"Refresh native state before responding"}"#))
            .unwrap());
    }
    if enqueue_outcome == EnqueueCommandOutcome::Full {
        session.emit(ChatEvent::Ack {
            client_request_id: client_request_id_for_ack,
            accepted: false,
            result: Some(serde_json::json!({"error": "queue full"})),
        });
        return Ok(Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"queue_full"}"#))
            .unwrap());
    }

    for advisory in goal_advisory {
        tracing::info!(%chat_id, %advisory, "UpdateGoal accepted with advisory");
    }
    session.remember_accepted_request(&client_request_id_for_ack);
    session.emit(ChatEvent::Ack {
        client_request_id: client_request_id_for_ack,
        accepted: true,
        result: Some(serde_json::json!({"queued": true})),
    });

    let queue_notify = session.queue_notify.clone();
    let processor_running = session.queue_processor_running.clone();
    drop(session);

    if !processor_running.swap(true, Ordering::SeqCst) {
        tokio::spawn(process_command_queue(
            app.clone(),
            session_arc,
            processor_running,
        ));
    } else {
        queue_notify.notify_one();
    }

    Ok(Response::builder()
        .status(StatusCode::ACCEPTED)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"accepted"}"#))
        .unwrap())
}

pub async fn handle_v1_chat_cancel_queued(
    State(app): State<AppState>,
    Path((chat_id, client_request_id)): Path<(String, String)>,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;

    let sessions = app.chat.sessions.clone();

    let session_arc =
        get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await?;
    let mut session = session_arc.lock().await;

    let _native_mutation =
        super::session::NativeMutationGuard::enter_locked(&session_arc, &mut session)
            .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;
    if cancel_queued_locked(&mut session, &chat_id, &client_request_id) {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"cancelled"}"#))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"not_found"}"#))
            .unwrap())
    }
}

pub async fn handle_v1_chat_reprioritize_queued(
    State(app): State<AppState>,
    Path((chat_id, client_request_id)): Path<(String, String)>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;
    let patch: QueuePriorityPatch = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let Some(session_arc) = app.chat.sessions.read().await.get(&chat_id).cloned() else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"not_found"}"#))
            .unwrap());
    };
    let mut session = session_arc.lock().await;

    let _native_mutation =
        super::session::NativeMutationGuard::enter_locked(&session_arc, &mut session)
            .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;
    if session.reprioritize_queued_command(&client_request_id, patch.priority) {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"updated"}"#))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"not_found"}"#))
            .unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};
    use crate::chat::perf_diagnostics::{self, MemoryPerfSink, PerfClock, PerfRecorder};
    use crate::chat::types::SessionState;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    type SizingKey = (String, u64);
    static SIZING_GATES: std::sync::LazyLock<
        std::sync::Mutex<HashMap<SizingKey, SnapshotSizingGate>>,
    > = std::sync::LazyLock::new(Default::default);

    pub(super) struct SnapshotSizingGate {
        entered: tokio::sync::oneshot::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }

    impl SnapshotSizingGate {
        pub(super) fn park(self) {
            if self.entered.send(()).is_ok() {
                match self
                    .release
                    .recv_timeout(std::time::Duration::from_secs(15))
                {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
                    Err(error) => panic!("snapshot sizing never released: {error}"),
                }
            }
        }
    }

    pub(super) fn take_snapshot_sizing_gate(chat_id: &str, seq: u64) -> Option<SnapshotSizingGate> {
        SIZING_GATES.lock().unwrap().remove(&(chat_id.into(), seq))
    }

    fn park_snapshot_sizing(
        chat_id: &str,
        seq: u64,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered, observed) = tokio::sync::oneshot::channel();
        let (release, parked) = std::sync::mpsc::channel();
        assert!(SIZING_GATES
            .lock()
            .unwrap()
            .insert(
                (chat_id.into(), seq),
                SnapshotSizingGate {
                    entered,
                    release: parked
                },
            )
            .is_none());
        (observed, release)
    }

    #[test]
    fn chunked_snapshot_bootstrap_preserves_message_order_and_bounds_events() {
        let messages = (0..70)
            .map(|index| {
                ChatMessage::new(
                    "user".to_string(),
                    format!("{index}:{}", "x".repeat(48 * 1024)),
                )
            })
            .collect::<Vec<_>>();
        let expected = messages
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>();
        let mut plan = SnapshotChunkPlan::default();
        let cancelled = AtomicBool::new(false);
        while plan.index < messages.len() {
            plan.measure_batch(&messages, &cancelled);
        }
        let mut rest = messages.as_slice();
        let chunks = plan
            .finish()
            .into_iter()
            .map(|len| {
                let (chunk, tail) = rest.split_at(len);
                rest = tail;
                chunk
            })
            .collect::<Vec<_>>();
        let actual = chunks
            .iter()
            .flat_map(|chunk| chunk.iter())
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.len() <= SNAPSHOT_CHUNK_MAX_MESSAGES));
        assert!(chunks.iter().all(|chunk| {
            chunk.len() == 1
                || serde_json::to_vec(chunk).unwrap().len()
                    <= SNAPSHOT_CHUNK_TARGET_BYTES + SNAPSHOT_CHUNK_MAX_MESSAGES * 2
        }));
    }

    struct TestClock {
        now: AtomicU64,
    }

    impl PerfClock for TestClock {
        fn now_us(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    fn install_perf_recorder() -> (
        perf_diagnostics::TestRecorderGuard,
        Arc<MemoryPerfSink>,
        Arc<PerfRecorder>,
    ) {
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestClock {
                now: AtomicU64::new(0),
            }),
            sink.clone(),
            [11; 32],
        ));
        (
            perf_diagnostics::install_test_recorder(recorder.clone()),
            sink,
            recorder,
        )
    }

    async fn test_app_with_workspace(root: &std::path::Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![root.to_path_buf()];
        app
    }

    #[tokio::test]
    async fn multiplex_validates_all_ids_before_loading_sessions() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let params = HashMap::from([(
            "chat_ids".into(),
            r#"["would-create-session","../invalid"]"#.into(),
        )]);
        assert!(
            handle_v1_chats_subscribe_many(State(app.clone()), axum::extract::Query(params))
                .await
                .is_err()
        );
        assert!(app.chat.sessions.read().await.is_empty());
    }

    #[tokio::test]
    async fn multiplex_bootstrap_is_concurrent_ordered_and_dropped_with_body() {
        use hyper::body::HttpBody;
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let first = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            "mux-first".into(),
        )));
        let second = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            "mux-second".into(),
        )));
        {
            let mut sessions = app.chat.sessions.write().await;
            sessions.insert("mux-first".into(), first.clone());
            sessions.insert("mux-second".into(), second.clone());
        }
        let blocked = first.lock().await;
        let params = HashMap::from([
            (
                "chat_ids".into(),
                r#"["mux-first","mux-second","mux-second"]"#.into(),
            ),
            ("snapshot_chunks".into(), "1".into()),
            ("per_chat_close".into(), "1".into()),
        ]);
        let mut body =
            handle_v1_chats_subscribe_many(State(app.clone()), axum::extract::Query(params))
                .await
                .unwrap()
                .into_body();
        async fn next(body: &mut Body) -> serde_json::Value {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let bytes = body.data().await.unwrap().unwrap();
                    let text = std::str::from_utf8(&bytes).unwrap();
                    if let Some(json) = text.strip_prefix("data: ") {
                        return serde_json::from_str(json.trim()).unwrap();
                    }
                }
            })
            .await
            .expect("ready chat must not wait for blocked chat")
        }
        let snapshot = next(&mut body).await;
        assert_eq!(snapshot["chat_id"], "mux-second");
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(next(&mut body).await["type"], "snapshot_end");
        assert_eq!(second.lock().await.event_tx.receiver_count(), 1);
        drop(blocked);
        let first_snapshot = next(&mut body).await;
        assert_eq!(first_snapshot["chat_id"], "mux-first");
        assert_eq!(first_snapshot["type"], "snapshot");
        assert_eq!(next(&mut body).await["type"], "snapshot_end");
        let sender = second.lock().await.event_tx.clone();
        sender.send(Arc::new(r#"{"chat_id":"mux-second","seq":"1","type":"stream_started","message_id":"ordered"}"#.into())).unwrap();
        let event = next(&mut body).await;
        assert_eq!(event["type"], "stream_started");
        assert_eq!(event["seq"], "1");
        first.lock().await.close_event_channel();
        let closed = next(&mut body).await;
        assert_eq!(closed["chat_id"], "mux-first");
        assert_eq!(closed["type"], "subscription_closed");
        drop(body);
        assert_eq!(sender.receiver_count(), 0);
        assert_eq!(first.lock().await.event_tx.receiver_count(), 0);
    }

    async fn insert_test_session(
        app: &AppState,
        chat_id: &str,
    ) -> Arc<tokio::sync::Mutex<ChatSession>> {
        let session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(chat_id.into())));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.into(), session.clone());
        session
    }

    /// Reads the next `data:` frame; `None` means the body reached EOF. Panics on a body error.
    async fn next_mux_frame(body: &mut Body) -> Option<serde_json::Value> {
        use hyper::body::HttpBody;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let bytes = match body.data().await {
                    None => return None,
                    Some(Ok(bytes)) => bytes,
                    Some(Err(error)) => panic!("multiplexed body must not error: {error}"),
                };
                let text = std::str::from_utf8(&bytes).unwrap();
                if let Some(json) = text.strip_prefix("data: ") {
                    return Some(serde_json::from_str(json.trim()).unwrap());
                }
            }
        })
        .await
        .expect("multiplexed frame must arrive")
    }

    /// After every chat has bootstrapped, any further snapshot is an unexpected re-subscribe.
    async fn next_mux_frame_after_bootstrap(body: &mut Body) -> Option<serde_json::Value> {
        let frame = next_mux_frame(body).await?;
        assert!(
            !matches!(frame["type"].as_str(), Some("snapshot" | "snapshot_end")),
            "unexpected snapshot after bootstrap: {frame}"
        );
        Some(frame)
    }

    #[tokio::test]
    async fn unpolled_snapshot_preparation_releases_process_admission() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = Arc::<str>::from("mux-unpolled-preparation");
        let session = insert_test_session(&app, &chat_id).await;
        session
            .lock()
            .await
            .messages
            .push(ChatMessage::new("user".into(), "snapshot".into()));
        let (entered, release) = park_snapshot_sizing(&chat_id, 0);
        let preparations = Arc::new(Semaphore::new(1));
        let response_slots = Arc::new(Semaphore::new(1));
        let mut prepare = Box::pin(prepare_subscription_snapshot(
            &app,
            &session,
            &chat_id,
            true,
            false,
            &response_slots,
            preparations.clone(),
        ));
        assert!(futures::poll!(&mut prepare).is_pending());
        let timeout = std::time::Duration::from_secs(10);
        tokio::time::timeout(timeout, entered)
            .await
            .unwrap()
            .unwrap();
        assert!(preparations.try_acquire().is_err());
        assert!(response_slots.try_acquire().is_err());

        release.send(()).unwrap();
        let _process_slot = tokio::time::timeout(timeout, preparations.acquire())
            .await
            .expect("an unpolled response must not keep process-wide preparation slots")
            .unwrap();
        assert!(response_slots.try_acquire().is_err());
        drop(prepare);
        assert!(response_slots.try_acquire().is_ok());
    }

    #[tokio::test]
    async fn multiplex_live_sibling_progresses_during_initial_and_recovery_snapshot_sizing() {
        const LARGE: &str = "mux-sizing-chat";
        const SIBLING: &str = "mux-sizing-sibling";
        const INITIAL_SEQ: u64 = 9_007_199_254_740_993;
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let large = insert_test_session(&app, LARGE).await;
        let sibling = insert_test_session(&app, SIBLING).await;
        let make_message = |id: &str, content: String| {
            let mut message = ChatMessage::new("user".into(), content);
            message.message_id = id.into();
            message
        };
        let initial = vec![
            make_message(
                "oversized",
                "\"\n".repeat(SNAPSHOT_CHUNK_TARGET_BYTES / 4 + 1),
            ),
            make_message("anchored", "before edit".into()),
        ];
        {
            let mut session = large.lock().await;
            session.event_tx = broadcast::channel(1).0;
            session.event_seq = INITIAL_SEQ;
            session.messages = initial.clone();
        }
        let (entered, release) = park_snapshot_sizing(LARGE, INITIAL_SEQ);
        let params = HashMap::from([
            (
                "chat_ids".into(),
                serde_json::to_string(&[LARGE, SIBLING]).unwrap(),
            ),
            ("snapshot_chunks".into(), "1".into()),
            ("per_chat_close".into(), "1".into()),
        ]);
        let mut body = handle_v1_chats_subscribe_many(State(app), axum::extract::Query(params))
            .await
            .unwrap()
            .into_body();
        for event_type in ["snapshot", "snapshot_end"] {
            let frame = next_mux_frame(&mut body).await.unwrap();
            assert_eq!(frame["chat_id"], SIBLING);
            assert_eq!(frame["type"], event_type);
        }

        async fn after_entered(entered: tokio::sync::oneshot::Receiver<()>) {
            tokio::time::timeout(std::time::Duration::from_secs(10), entered)
                .await
                .expect("snapshot sizing must have started")
                .unwrap();
        }

        fn emit_started(session: &mut ChatSession, message_id: &str) {
            session.emit(ChatEvent::StreamStarted {
                message_id: message_id.into(),
            });
        }

        async fn expect_snapshot_frames(body: &mut Body, seq: u64, expected: &[ChatMessage]) {
            let seq = seq.to_string();
            let header = next_mux_frame(body).await.unwrap();
            assert_eq!(header["chat_id"], LARGE);
            assert_eq!(header["type"], "snapshot");
            assert_eq!(header["seq"], seq);
            assert_eq!(header["snapshot_chunked"], true);
            assert_eq!(header["messages"], json!([]));
            assert_eq!(header["message_count"], expected.len());
            assert_eq!(header["chunk_count"], 2);
            let mut actual = Vec::new();
            for chunk_index in 0..2 {
                let chunk = next_mux_frame(body).await.unwrap();
                assert_eq!(chunk["chat_id"], LARGE);
                assert_eq!(chunk["type"], "snapshot_chunk");
                assert_eq!(chunk["seq"], seq);
                assert_eq!(chunk["chunk_index"], chunk_index);
                assert_eq!(chunk["chunk_count"], 2);
                actual.extend(chunk["messages"].as_array().unwrap().iter().cloned());
            }
            assert_eq!(json!(actual), json!(expected));
            let end = next_mux_frame(body).await.unwrap();
            assert_eq!(
                end,
                json!({ "chat_id": LARGE, "type": "snapshot_end", "seq": seq,
                "chunk_count": 2, "message_count": expected.len() })
            );
        }

        let (frame, ()) = tokio::join!(next_mux_frame(&mut body), async {
            after_entered(entered).await;
            {
                let mut session = large.lock().await;
                session.messages[1].content = ChatContent::SimpleText("after edit".into());
                emit_started(&mut session, "queued-after-initial");
            }
            emit_started(&mut *sibling.lock().await, "during-initial-sizing");
        });
        assert_eq!(
            frame.unwrap(),
            json!({"chat_id": SIBLING, "type": "stream_started",
            "seq": "1", "message_id": "during-initial-sizing"})
        );
        release.send(()).unwrap();
        expect_snapshot_frames(&mut body, INITIAL_SEQ, &initial).await;
        assert_eq!(
            next_mux_frame(&mut body).await.unwrap(),
            json!({"chat_id": LARGE,
            "type": "stream_started", "seq": (INITIAL_SEQ + 1).to_string(),
            "message_id": "queued-after-initial"})
        );

        let (recovery_seq, recovered) = {
            let mut session = large.lock().await;
            session
                .messages
                .push(make_message("new", "added before recovery".into()));
            for id in ["discarded-1", "discarded-2", "discarded-3"] {
                emit_started(&mut session, id);
            }
            (session.event_seq, session.messages.clone())
        };
        let (entered, release) = park_snapshot_sizing(LARGE, recovery_seq);
        let (frame, ()) = tokio::join!(next_mux_frame(&mut body), async {
            after_entered(entered).await;
            emit_started(&mut *large.lock().await, "queued-after-recovery");
            emit_started(&mut *sibling.lock().await, "during-recovery-sizing");
        });
        assert_eq!(
            frame.unwrap(),
            json!({"chat_id": SIBLING, "type": "stream_started",
            "seq": "2", "message_id": "during-recovery-sizing"})
        );
        release.send(()).unwrap();
        expect_snapshot_frames(&mut body, recovery_seq, &recovered).await;
        assert_eq!(
            next_mux_frame(&mut body).await.unwrap(),
            json!({"chat_id": LARGE,
            "type": "stream_started", "seq": (recovery_seq + 1).to_string(),
            "message_id": "queued-after-recovery"})
        );
        drop(body);
        assert_eq!(large.lock().await.event_tx.receiver_count(), 0);
        assert_eq!(sibling.lock().await.event_tx.receiver_count(), 0);
    }

    #[tokio::test]
    async fn multiplex_ended_chat_closes_only_itself_and_body_reaches_eof() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let ending = insert_test_session(&app, "mux-ending").await;
        let staying = insert_test_session(&app, "mux-staying").await;
        let params = HashMap::from([
            ("chat_ids".into(), r#"["mux-ending","mux-staying"]"#.into()),
            ("per_chat_close".into(), "true".into()),
        ]);
        let mut body =
            handle_v1_chats_subscribe_many(State(app.clone()), axum::extract::Query(params))
                .await
                .unwrap()
                .into_body();

        let mut snapshots = std::collections::HashSet::new();
        while snapshots.len() < 2 {
            let frame = next_mux_frame(&mut body)
                .await
                .expect("snapshot before EOF");
            assert_eq!(frame["type"], "snapshot");
            snapshots.insert(frame["chat_id"].as_str().unwrap().to_string());
        }
        assert_eq!(staying.lock().await.event_tx.receiver_count(), 1);

        ending.lock().await.close_event_channel();
        let closed = next_mux_frame_after_bootstrap(&mut body)
            .await
            .expect("closed frame before EOF");
        assert_eq!(
            closed,
            json!({
                "chat_id": "mux-ending",
                "type": "subscription_closed",
                "reason": "ended",
                "retryable": true,
                "message": "chat subscription ended",
            })
        );

        let sender = staying.lock().await.event_tx.clone();
        sender
            .send(Arc::new(
                r#"{"chat_id":"mux-staying","seq":"1","type":"stream_started","message_id":"after-close"}"#.into(),
            ))
            .unwrap();
        let event = next_mux_frame_after_bootstrap(&mut body)
            .await
            .expect("sibling event after close");
        assert_eq!(event["chat_id"], "mux-staying");
        assert_eq!(event["type"], "stream_started");
        assert_eq!(event["message_id"], "after-close");

        assert_eq!(sender.receiver_count(), 1);
        drop(sender);
        staying.lock().await.close_event_channel();
        let mut closed_by_chat: HashMap<String, usize> = HashMap::new();
        closed_by_chat.insert("mux-ending".into(), 1);
        while let Some(frame) = next_mux_frame_after_bootstrap(&mut body).await {
            if frame["type"] == "subscription_closed" {
                assert_eq!(frame["reason"], "ended");
                *closed_by_chat
                    .entry(frame["chat_id"].as_str().unwrap().to_string())
                    .or_default() += 1;
            }
        }
        assert_eq!(
            closed_by_chat,
            HashMap::from([("mux-ending".into(), 1), ("mux-staying".into(), 1)])
        );
    }

    #[tokio::test]
    async fn multiplex_unavailable_chat_reports_retryability_without_ending_siblings() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let valid = insert_test_session(&app, "mux-valid").await;
        let subscribe_app = app.clone();
        let mut body = multiplex_chat_subscriptions(
            vec![
                "mux-client-error".to_string(),
                "mux-server-error".to_string(),
                "mux-valid".to_string(),
            ],
            true,
            move |id| {
                let app = subscribe_app.clone();
                async move {
                    match id.as_str() {
                        "mux-client-error" => Err(ScratchError::new(
                            StatusCode::NOT_FOUND,
                            "/home/user/secret/path token=abc".into(),
                        )),
                        "mux-server-error" => Err(ScratchError::new(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "/home/user/secret/path token=abc".into(),
                        )),
                        _ => {
                            let params = HashMap::from([("chat_id".to_string(), id)]);
                            handle_v1_chat_subscribe(State(app), axum::extract::Query(params)).await
                        }
                    }
                }
            },
        );

        let mut closed = HashMap::new();
        let mut valid_snapshot = false;
        while closed.len() < 2 || !valid_snapshot {
            let frame = next_mux_frame(&mut body).await.expect("frame before EOF");
            match frame["type"].as_str() {
                Some("snapshot") => {
                    assert_eq!(frame["chat_id"], "mux-valid");
                    valid_snapshot = true;
                }
                Some("subscription_closed") => {
                    assert_eq!(frame["reason"], "unavailable");
                    let message = frame["message"].as_str().unwrap();
                    assert!(!message.contains("/home") && !message.contains("token"));
                    assert!(message.chars().count() <= SUBSCRIPTION_CLOSED_MESSAGE_MAX_CHARS);
                    let previous = closed.insert(
                        frame["chat_id"].as_str().unwrap().to_string(),
                        frame["retryable"].as_bool().unwrap(),
                    );
                    assert!(previous.is_none(), "one closed frame per chat");
                }
                other => panic!("unexpected frame type {other:?}"),
            }
        }
        assert_eq!(
            closed,
            HashMap::from([
                ("mux-client-error".to_string(), false),
                ("mux-server-error".to_string(), true),
            ])
        );
        assert_eq!(valid.lock().await.event_tx.receiver_count(), 1);
        valid
            .lock()
            .await
            .event_tx
            .send(Arc::new(
                r#"{"chat_id":"mux-valid","seq":"1","type":"stream_started","message_id":"still-open"}"#.into(),
            ))
            .unwrap();
        let event = next_mux_frame_after_bootstrap(&mut body)
            .await
            .expect("valid chat keeps streaming");
        assert_eq!(event["message_id"], "still-open");
        valid.lock().await.close_event_channel();
        let mut after = Vec::new();
        while let Some(frame) = next_mux_frame_after_bootstrap(&mut body).await {
            after.push(frame);
        }
        assert_eq!(
            after,
            vec![json!({
                "chat_id": "mux-valid",
                "type": "subscription_closed",
                "reason": "ended",
                "retryable": true,
                "message": "chat subscription ended",
            })],
            "no further frame for the unavailable chats"
        );
    }

    #[test]
    fn subscription_retryability_follows_transient_statuses_and_open_refusals() {
        for (status, retryable) in [
            (StatusCode::REQUEST_TIMEOUT, true),
            (StatusCode::TOO_MANY_REQUESTS, true),
            (StatusCode::BAD_GATEWAY, true),
            (StatusCode::SERVICE_UNAVAILABLE, true),
            (StatusCode::GATEWAY_TIMEOUT, true),
            (StatusCode::BAD_REQUEST, false),
            (StatusCode::NOT_FOUND, false),
            (StatusCode::CONFLICT, false),
            (StatusCode::INTERNAL_SERVER_ERROR, false),
            (StatusCode::NOT_IMPLEMENTED, false),
        ] {
            assert_eq!(subscription_retryable(status), retryable, "{status}");
        }
        use crate::chat::session_runtime::SessionOpenError;
        for (error, retryable) in [
            (SessionOpenError::Closed, true),
            (SessionOpenError::Busy, true),
            (SessionOpenError::Failed("disk".into()), false),
        ] {
            let scratch: ScratchError = error.clone().into();
            let frame = subscription_unavailable_frame("chat", &scratch);
            let json: serde_json::Value = serde_json::from_str(
                frame
                    .strip_prefix("data: ")
                    .unwrap()
                    .trim_end_matches("\n\n"),
            )
            .unwrap();
            assert_eq!(json["retryable"], retryable, "{error:?}");
        }
    }

    #[tokio::test]
    async fn multiplex_body_reaches_eof_when_every_chat_is_unavailable() {
        let mut body = multiplex_chat_subscriptions(
            vec!["mux-a".to_string(), "mux-b".to_string()],
            true,
            |_| async { Err(ScratchError::new(StatusCode::BAD_REQUEST, "bad".into())) },
        );
        let mut chats = Vec::new();
        while let Some(frame) = next_mux_frame(&mut body).await {
            assert_eq!(frame["type"], "subscription_closed");
            assert_eq!(frame["retryable"], false);
            chats.push(frame["chat_id"].as_str().unwrap().to_string());
        }
        chats.sort();
        assert_eq!(chats, vec!["mux-a".to_string(), "mux-b".to_string()]);
    }

    async fn next_mux_item_after_heartbeats(
        body: &mut Body,
    ) -> Option<Result<hyper::body::Bytes, hyper::Error>> {
        use hyper::body::HttpBody;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match body.data().await {
                    Some(Ok(bytes)) => {
                        let heartbeat = std::str::from_utf8(&bytes)
                            .ok()
                            .and_then(|text| text.strip_prefix(": hb "))
                            .and_then(|text| text.strip_suffix("\n\n"))
                            .and_then(|timestamp| timestamp.parse::<i64>().ok());
                        assert!(
                            heartbeat.is_some(),
                            "unexpected multiplexed chunk: {bytes:?}"
                        );
                    }
                    item => return item,
                }
            }
        })
        .await
        .expect("ended chat must surface")
    }

    #[tokio::test]
    async fn multiplex_legacy_abort_after_ready_sibling_heartbeat() {
        let (release, ended) = tokio::sync::oneshot::channel::<()>();
        let ending = Body::wrap_stream(futures::stream::unfold(ended, |ended| async move {
            ended.await.unwrap();
            None::<(Result<&'static str, std::convert::Infallible>, _)>
        }));
        let sibling = Body::wrap_stream(async_stream::stream! {
            yield Ok::<_, std::convert::Infallible>(": hb 123\n\n");
            release.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let bodies = std::sync::Mutex::new(HashMap::from([
            ("ending".to_string(), ending),
            ("sibling".to_string(), sibling),
        ]));
        let mut body =
            multiplex_chat_subscriptions(vec!["ending".into(), "sibling".into()], false, |id| {
                let body = bodies.lock().unwrap().remove(&id).unwrap();
                async move { Ok(Response::new(body)) }
            });
        let item = next_mux_item_after_heartbeats(&mut body).await;
        assert!(
            matches!(item, Some(Err(_))),
            "legacy clients see an aborted body, not a close frame"
        );
    }

    #[tokio::test]
    async fn multiplex_without_per_chat_close_aborts_on_ended_or_unavailable_chat() {
        use hyper::body::HttpBody;
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let ending = insert_test_session(&app, "legacy-ending").await;
        let _staying = insert_test_session(&app, "legacy-staying").await;
        let params = HashMap::from([(
            "chat_ids".into(),
            r#"["legacy-ending","legacy-staying"]"#.into(),
        )]);
        let mut body =
            handle_v1_chats_subscribe_many(State(app.clone()), axum::extract::Query(params))
                .await
                .unwrap()
                .into_body();
        let mut snapshots = 0;
        while snapshots < 2 {
            let frame = next_mux_frame(&mut body).await.expect("snapshot");
            assert_eq!(frame["type"], "snapshot");
            snapshots += 1;
        }
        ending.lock().await.close_event_channel();
        let item = next_mux_item_after_heartbeats(&mut body).await;
        assert!(
            matches!(item, Some(Err(_))),
            "legacy clients see an aborted body, not a close frame"
        );

        let mut body = multiplex_chat_subscriptions(
            vec!["legacy-unavailable".to_string()],
            false,
            |_| async { Err(ScratchError::new(StatusCode::NOT_FOUND, "gone".into())) },
        );
        let item = tokio::time::timeout(std::time::Duration::from_secs(10), body.data())
            .await
            .expect("preparation failure must surface");
        assert!(matches!(item, Some(Err(_))));
    }

    #[tokio::test]
    async fn companion_http_settings_cannot_remove_source_binding() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = "companion-http-guard";
        let mut session = ChatSession::new(chat_id.into());
        session.thread.mode = "no_tools".into();
        session.thread.tool_use = "none".into();
        session.thread.buddy_meta = Some(crate::chat::types::BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "companion".into(),
            workflow_id: None,
            source_chat_id: Some("source-chat".into()),
            source_project: Some(workspace.path().to_string_lossy().into_owned()),
            source_created_at: Some("source-origin".into()),
        });
        let before = serde_json::to_value(&session.thread).unwrap();
        let session = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.into(), session.clone());
        let response = handle_v1_chat_command(
            State(app),
            Path(chat_id.into()),
            serde_json::to_vec(&json!({
                "client_request_id": "replace-binding",
                "type": "set_params",
                "patch": {"mode": "agent", "tool_use": "agent", "buddy_meta": null}
            }))
            .unwrap()
            .into(),
        )
        .await;
        assert_eq!(response.unwrap_err().status_code, StatusCode::FORBIDDEN);
        let session = session.lock().await;
        assert_eq!(serde_json::to_value(&session.thread).unwrap(), before);
        assert!(session.command_queue.is_empty());
    }

    #[tokio::test]
    async fn delivery_command_edits_bypass_busy_processor() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = "live-delivery-edit";
        let session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(chat_id.into())));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.into(), session.clone());
        session.lock().await.start_stream().unwrap();
        let delivery = PendingDelivery::with_id(
            "live",
            vec![ChatMessage::new("user".into(), "notice".into())],
            PushMode::Append,
            "test",
            false,
        );
        for command in [
            ChatCommand::DeliverMessages {
                delivery,
            },
            ChatCommand::UpdatePendingDelivery {
                delivery_id: "live".into(),
                push: Some(PushMode::Preempt),
                cancel: false,
            },
        ] {
            let request = CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: uuid::Uuid::new_v4().to_string(),
                priority: false,
                command,
            };
            let response = handle_v1_chat_command(
                State(app.clone()),
                Path(chat_id.into()),
                serde_json::to_vec(&request).unwrap().into(),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let session = session.lock().await;
        assert!(session.command_queue.is_empty());
        assert!(session.pending_deliveries.is_empty());
        assert!(session.draft_message.is_none());
    }

    #[tokio::test]
    async fn python_commands_bypass_busy_processor_with_errors() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = "python-command-stub";
        let session = insert_test_session(&app, chat_id).await;
        for state in [SessionState::Generating, SessionState::ExecutingTools] {
            session.lock().await.set_runtime_state(state, None);
            for command in [
                ChatCommand::PythonApprovalDecision {
                    request_id: "approval".into(),
                    decision: refact_chat_api::PythonApprovalDecisionKind::Deny,
                },
                ChatCommand::PythonRunControl {
                    tool_call_id: "python".into(),
                    action: refact_chat_api::PythonRunControlAction::Stop,
                },
            ] {
                let request = CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: uuid::Uuid::new_v4().to_string(),
                    priority: false,
                    command,
                };
                let response = handle_v1_chat_command(
                    State(app.clone()),
                    Path(chat_id.into()),
                    serde_json::to_vec(&request).unwrap().into(),
                )
                .await
                .unwrap();
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                let session = session.lock().await;
                assert!(session.command_queue.is_empty());
                assert_eq!(session.runtime.state, state);
                assert!(session.messages.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn abort_command_clears_and_persists_running_wait_tool_call() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = "abort-running-sleep";
        let session_arc = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            chat_id.to_string(),
        )));
        let mut events = {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::ExecutingTools, None);
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "Wait briefly".to_string(),
            ));
            session.messages.push(ChatMessage {
                message_id: "assistant-with-sleep".to_string(),
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Waiting briefly.".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "sleep-call".to_string(),
                    index: Some(0),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "wait".to_string(),
                        arguments: json!({
                            "target": {"kind": "duration", "duration_ms": 30_000},
                            "description": "Wait briefly",
                        })
                        .to_string(),
                    },
                    extra_content: None,
                    started_at_ms: None,
                    completed_at_ms: None,
                }]),
                ..Default::default()
            });
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        let request = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "wake-up-request".to_string(),
            priority: false,
            command: ChatCommand::Abort {},
        };
        let body = hyper::body::Bytes::from(serde_json::to_vec(&request).unwrap());

        let response = handle_v1_chat_command(State(app.clone()), Path(chat_id.to_string()), body)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.messages[1].tool_calls.is_none());
        assert!(!session.trajectory_dirty);
        drop(session);

        let mut saw_sleep_removed = false;
        while let Ok(json) = events.try_recv() {
            let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
            if let ChatEvent::MessageUpdated {
                message, ..
            } = envelope.event
            {
                saw_sleep_removed =
                    message.message_id == "assistant-with-sleep" && message.tool_calls.is_none();
            }
        }
        assert!(saw_sleep_removed);

        let trajectory_path = workspace
            .path()
            .join(".refact")
            .join("trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        let trajectory: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(trajectory_path).await.unwrap())
                .unwrap();
        assert_eq!(
            trajectory["messages"][1]["tool_calls"],
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn cancel_queued_command_clears_queue_wait_timestamp() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cancel-queued";
        let session_arc = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            chat_id.to_string(),
        )));
        {
            let mut session = session_arc.lock().await;
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "queued-request".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            });
            session
                .command_enqueued_at
                .insert("queued-request".to_string(), Instant::now());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        let response = handle_v1_chat_cancel_queued(
            State(app),
            Path((chat_id.to_string(), "queued-request".to_string())),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let session = session_arc.lock().await;
        assert!(session.command_queue.is_empty());
        assert!(session.command_enqueued_at.is_empty());
    }

    #[tokio::test]
    async fn reprioritize_queued_multimodal_command_preserves_payload_and_timestamp() {
        let workspace = tempfile::tempdir().unwrap();
        let app = test_app_with_workspace(workspace.path()).await;
        let chat_id = "reprioritize-queued";
        let session_arc = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            chat_id.to_string(),
        )));
        let enqueued_at = Instant::now();
        {
            let mut session = session_arc.lock().await;
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "priority-first".to_string(),
                priority: true,
                command: ChatCommand::Regenerate {},
            });
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "normal-last".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            });
            session.command_queue.push_back(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "multimodal".to_string(),
                priority: false,
                command: ChatCommand::UserMessage {
                    content: json!([
                        {"type": "text", "text": "describe this"},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,image"}}
                    ]),
                    attachments: vec![json!({"name": "diagram.png", "mime": "image/png"})],
                    context_files: vec![
                        json!({"file_name": "notes.md", "file_content": "context"}),
                    ],
                    suppress_auto_enrichment: true,
                    client_message_id: None,
                    app_context_ids: Vec::new(),
                },
            });
            session
                .command_enqueued_at
                .insert("multimodal".to_string(), enqueued_at);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        let response = handle_v1_chat_reprioritize_queued(
            State(app.clone()),
            Path((chat_id.to_string(), "multimodal".to_string())),
            hyper::body::Bytes::from(r#"{"priority":true}"#),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let session = session_arc.lock().await;
        assert_eq!(
            session
                .command_queue
                .iter()
                .map(|request| request.client_request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["priority-first", "multimodal", "normal-last"]
        );
        let request = &session.command_queue[1];
        assert!(request.priority);
        match &request.command {
            ChatCommand::UserMessage {
                content,
                attachments,
                context_files,
                suppress_auto_enrichment,
                client_message_id,
                ..
            } => {
                assert_eq!(content[0], json!({"type": "text", "text": "describe this"}));
                assert_eq!(content[1]["type"], "image_url");
                assert_eq!(
                    content[1]["image_url"]["url"],
                    "data:image/png;base64,image"
                );
                assert_eq!(
                    attachments,
                    &vec![json!({"name": "diagram.png", "mime": "image/png"})]
                );
                assert_eq!(
                    context_files,
                    &vec![json!({"file_name": "notes.md", "file_content": "context"})]
                );
                assert!(*suppress_auto_enrichment);
                assert_eq!(client_message_id, &None);
            }
            command => panic!("expected user message, got {command:?}"),
        }
        assert_eq!(
            session.command_enqueued_at.get("multimodal"),
            Some(&enqueued_at)
        );
        drop(session);

        let response = handle_v1_chat_reprioritize_queued(
            State(app.clone()),
            Path((chat_id.to_string(), "multimodal".to_string())),
            hyper::body::Bytes::from(r#"{"priority":false}"#),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let session = session_arc.lock().await;
        assert_eq!(
            session
                .command_queue
                .iter()
                .map(|request| request.client_request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["priority-first", "normal-last", "multimodal"]
        );
        assert!(!session.command_queue[2].priority);
        assert_eq!(
            session.command_enqueued_at.get("multimodal"),
            Some(&enqueued_at)
        );
        drop(session);

        let response = handle_v1_chat_reprioritize_queued(
            State(app.clone()),
            Path((chat_id.to_string(), "missing".to_string())),
            hyper::body::Bytes::from(r#"{"priority":false}"#),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let unknown_chat_id = "missing-chat".to_string();
        let response = handle_v1_chat_reprioritize_queued(
            State(app.clone()),
            Path((unknown_chat_id.clone(), "missing".to_string())),
            hyper::body::Bytes::from(r#"{"priority":false}"#),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!app
            .chat
            .sessions
            .read()
            .await
            .contains_key(&unknown_chat_id));
    }
}
