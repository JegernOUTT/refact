use axum::extract::Path;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio::sync::broadcast;

use crate::app_state::AppState;
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

struct SnapshotBootstrap {
    initial_json: String,
    chunks: Vec<Vec<crate::call_validation::ChatMessage>>,
    chunked: bool,
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
    messages: &'a [crate::call_validation::ChatMessage],
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

fn chunk_snapshot_messages(
    messages: Vec<crate::call_validation::ChatMessage>,
) -> Vec<Vec<crate::call_validation::ChatMessage>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_bytes = 0usize;

    for message in messages {
        let message_bytes = serde_json::to_vec(&message)
            .map(|encoded| encoded.len())
            .unwrap_or(SNAPSHOT_CHUNK_TARGET_BYTES);
        if !current.is_empty()
            && (current.len() >= SNAPSHOT_CHUNK_MAX_MESSAGES
                || current_bytes.saturating_add(message_bytes) > SNAPSHOT_CHUNK_TARGET_BYTES)
        {
            chunks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes = current_bytes.saturating_add(message_bytes);
        current.push(message);
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn serialize_snapshot_bootstrap(
    chat_id: &str,
    seq: u64,
    event: ChatEvent,
    messages: Option<Vec<crate::call_validation::ChatMessage>>,
) -> Result<SnapshotBootstrap, serde_json::Error> {
    let Some(messages) = messages else {
        return Ok(SnapshotBootstrap {
            initial_json: serde_json::to_string(&EventEnvelope {
                chat_id: chat_id.to_string(),
                seq,
                event,
            })?,
            chunks: Vec::new(),
            chunked: false,
        });
    };

    let message_count = messages.len();
    let chunks = chunk_snapshot_messages(messages);
    let mut initial = serde_json::to_value(EventEnvelope {
        chat_id: chat_id.to_string(),
        seq,
        event,
    })?;
    if let Some(object) = initial.as_object_mut() {
        object.insert("snapshot_chunked".to_string(), serde_json::json!(true));
        object.insert("chunk_count".to_string(), serde_json::json!(chunks.len()));
        object.insert(
            "message_count".to_string(),
            serde_json::json!(message_count),
        );
    }
    Ok(SnapshotBootstrap {
        initial_json: serde_json::to_string(&initial)?,
        chunks,
        chunked: true,
    })
}

fn snapshot_chunk_json(
    chat_id: &str,
    seq: u64,
    chunk_index: usize,
    chunk_count: usize,
    messages: &[crate::call_validation::ChatMessage],
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SnapshotChunkEnvelope {
        chat_id,
        seq,
        event_type: "snapshot_chunk",
        chunk_index,
        chunk_count,
        messages,
    })
}

fn snapshot_end_json(
    chat_id: &str,
    seq: u64,
    chunk_count: usize,
    message_count: usize,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SnapshotEndEnvelope {
        chat_id,
        seq,
        event_type: "snapshot_end",
        chunk_count,
        message_count,
    })
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

pub async fn handle_v1_chats_subscribe_many(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response<Body>, ScratchError> {
    use futures::StreamExt;
    type ChatBodyStream = std::pin::Pin<
        Box<
            dyn futures::Stream<
                    Item = Result<hyper::body::Bytes, Box<dyn std::error::Error + Send + Sync>>,
                > + Send,
        >,
    >;

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
    let streams = chat_ids.into_iter().filter(|id| seen.insert(id.clone())).map(|id| {
        let app = app.clone();
        let mut params = params.clone();
        params.insert("chat_id".into(), id);
        Box::pin(async_stream::stream! {
            match handle_v1_chat_subscribe(State(app), axum::extract::Query(params)).await {
                Ok(response) => {
                    let mut body = response.into_body();
                    while let Some(chunk) = body.next().await {
                        yield chunk.map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>);
                    }
                    yield Err(Box::new(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "chat subscription ended")) as Box<dyn std::error::Error + Send + Sync>);
                }
                Err(error) => {
                    tracing::warn!(%error, "multiplex chat subscription preparation failed");
                    yield Err(Box::new(std::io::Error::other("chat subscription preparation failed")) as Box<dyn std::error::Error + Send + Sync>);
                }
            }
        }) as ChatBodyStream
    });
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(futures::stream::select_all(streams)))
        .unwrap())
}

pub async fn handle_v1_chat_subscribe(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
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

    let session_arc = get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await;
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
    let session = session_arc.lock().await;
    let mut rx = session.subscribe();
    let initial_seq = session.event_seq;
    let initial_messages = chunked_snapshot.then(|| session.snapshot_messages());
    let snapshot = snapshot_with_agents_mode(app.clone(), &session, !chunked_snapshot);
    drop(session);
    let (snapshot, background_agents) = snapshot.await;
    {
        let mut session = session_arc.lock().await;
        session.upsert_background_agents(background_agents);
    }

    let initial_serialize_started_at = perf_diagnostics::is_enabled().then(Instant::now);
    let initial_bootstrap =
        match serialize_snapshot_bootstrap(&chat_id, initial_seq, snapshot, initial_messages) {
            Ok(bootstrap) => {
                if let Some(initial_serialize_started_at) = initial_serialize_started_at {
                    record_sse_serialize(
                        &chat_id,
                        PerfOutcome::Success,
                        initial_serialize_started_at
                            .elapsed()
                            .as_micros()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        Some(bootstrap.initial_json.len() as u64),
                    );
                }
                bootstrap
            }
            Err(e) => {
                if let Some(initial_serialize_started_at) = initial_serialize_started_at {
                    record_sse_serialize(
                        &chat_id,
                        PerfOutcome::Failure,
                        initial_serialize_started_at
                            .elapsed()
                            .as_micros()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        None,
                    );
                }
                tracing::error!(
                    "Failed to serialize initial SSE snapshot for {}: {}",
                    chat_id,
                    e
                );
                return Err(ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "snapshot serialization failed".to_string(),
                ));
            }
        };

    let session_for_stream = session_arc.clone();
    let chat_id_for_stream = chat_id.clone();
    let closed_flag = session_arc.lock().await.closed_flag.clone();

    let stream = async_stream::stream! {
        let initial_chunk_count = initial_bootstrap.chunks.len();
        let initial_message_count = initial_bootstrap.chunks.iter().map(Vec::len).sum();
        yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", initial_bootstrap.initial_json));
        for (chunk_index, messages) in initial_bootstrap.chunks.iter().enumerate() {
            match snapshot_chunk_json(
                &chat_id_for_stream,
                initial_seq,
                chunk_index,
                initial_chunk_count,
                messages,
            ) {
                Ok(json) => yield Ok(format!("data: {}\n\n", json)),
                Err(error) => {
                    tracing::error!(%error, "Failed to serialize initial SSE snapshot chunk");
                    return;
                }
            }
        }
        if initial_bootstrap.chunked {
            match snapshot_end_json(
                &chat_id_for_stream,
                initial_seq,
                initial_chunk_count,
                initial_message_count,
            ) {
                Ok(json) => yield Ok(format!("data: {}\n\n", json)),
                Err(error) => {
                    tracing::error!(%error, "Failed to serialize initial SSE snapshot end");
                    return;
                }
            }
        }

        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(json) => {
                            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            let recovery_started_at = perf_diagnostics::is_enabled().then(Instant::now);
                            tracing::info!("SSE subscriber lagged, skipped {} events, sending fresh snapshot", skipped);
                            let session = session_for_stream.lock().await;
                            if session.closed {
                                break;
                            }
                            // Re-subscribe BEFORE capturing event_seq so we don't miss events
                            // emitted between snapshot and the new receiver start.
                            rx = session.subscribe();
                            let recovery_seq = session.event_seq;
                            let recovery_messages = chunked_snapshot.then(|| session.snapshot_messages());
                            let recovery_snapshot =
                                snapshot_with_agents_mode(app.clone(), &session, !chunked_snapshot);
                            drop(session);
                            let (recovery_snapshot, background_agents) = recovery_snapshot.await;
                            {
                                let mut session = session_for_stream.lock().await;
                                session.upsert_background_agents(background_agents);
                            }
                            let recovery_serialize_started_at = recovery_started_at.map(|_| Instant::now());
                            match serialize_snapshot_bootstrap(
                                &chat_id_for_stream,
                                recovery_seq,
                                recovery_snapshot,
                                recovery_messages,
                            ) {
                                Ok(bootstrap) => {
                                    if let (Some(recovery_started_at), Some(recovery_serialize_started_at)) =
                                        (recovery_started_at, recovery_serialize_started_at)
                                    {
                                        record_sse_serialize(
                                            &chat_id_for_stream,
                                            PerfOutcome::Success,
                                            recovery_serialize_started_at
                                                .elapsed()
                                                .as_micros()
                                                .try_into()
                                                .unwrap_or(u64::MAX),
                                            Some(bootstrap.initial_json.len() as u64),
                                        );
                                        record_lagged_recovery(
                                            &chat_id_for_stream,
                                            PerfOutcome::Success,
                                            recovery_started_at
                                                .elapsed()
                                                .as_micros()
                                                .try_into()
                                                .unwrap_or(u64::MAX),
                                            skipped as u64,
                                        );
                                    }
                                    let chunk_count = bootstrap.chunks.len();
                                    let message_count = bootstrap.chunks.iter().map(Vec::len).sum();
                                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", bootstrap.initial_json));
                                    for (chunk_index, messages) in bootstrap.chunks.iter().enumerate() {
                                        match snapshot_chunk_json(
                                            &chat_id_for_stream,
                                            recovery_seq,
                                            chunk_index,
                                            chunk_count,
                                            messages,
                                        ) {
                                            Ok(json) => yield Ok(format!("data: {}\n\n", json)),
                                            Err(error) => {
                                                tracing::error!(%error, "Failed to serialize SSE recovery snapshot chunk");
                                                return;
                                            }
                                        }
                                    }
                                    if bootstrap.chunked {
                                        match snapshot_end_json(
                                            &chat_id_for_stream,
                                            recovery_seq,
                                            chunk_count,
                                            message_count,
                                        ) {
                                            Ok(json) => yield Ok(format!("data: {}\n\n", json)),
                                            Err(error) => {
                                                tracing::error!(%error, "Failed to serialize SSE recovery snapshot end");
                                                return;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    if let (Some(recovery_started_at), Some(recovery_serialize_started_at)) =
                                        (recovery_started_at, recovery_serialize_started_at)
                                    {
                                        record_sse_serialize(
                                            &chat_id_for_stream,
                                            PerfOutcome::Failure,
                                            recovery_serialize_started_at
                                                .elapsed()
                                                .as_micros()
                                                .try_into()
                                                .unwrap_or(u64::MAX),
                                            None,
                                        );
                                        record_lagged_recovery(
                                            &chat_id_for_stream,
                                            PerfOutcome::Failure,
                                            recovery_started_at
                                                .elapsed()
                                                .as_micros()
                                                .try_into()
                                                .unwrap_or(u64::MAX),
                                            skipped as u64,
                                        );
                                    }
                                    tracing::error!("Failed to serialize SSE recovery snapshot for {}: {}", chat_id_for_stream, e);
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat_interval.tick() => {
                    if closed_flag.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    yield Ok::<_, std::convert::Infallible>(format!(": hb {}\n\n", chrono::Utc::now().timestamp()));
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

pub async fn handle_v1_chat_command(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;

    let request: CommandRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let goal_policy = if matches!(
        &request.command,
        ChatCommand::SetGoal { .. } | ChatCommand::UpdateGoal { .. }
    ) {
        Some(super::verify_cmd::VerifyCommandPolicy::load(app.gcx.clone()).await)
    } else {
        None
    };
    let sessions = app.chat.sessions.clone();

    let session_arc = get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await;
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
        && session.runtime.recovery.as_ref().is_some_and(|r| {
            matches!(
                r.state,
                refact_core::recovery::RecoveryState::Waiting
                    | refact_core::recovery::RecoveryState::Attempting
            )
        })
        && session.goal.is_some()
    {
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

    if matches!(request.command, ChatCommand::Abort {}) {
        session.apply_priority_abort();
        let should_save_trajectory = session.trajectory_dirty;
        session.remember_accepted_request(&request.client_request_id);
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: true,
            result: Some(serde_json::json!({"aborted": true})),
        });
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

    let session_arc = get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await;
    let mut session = session_arc.lock().await;

    let initial_len = session.command_queue.len();
    let _native_mutation =
        super::session::NativeMutationGuard::enter_locked(&session_arc, &mut session)
            .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;
    session
        .command_queue
        .retain(|r| r.client_request_id != client_request_id);
    session.command_enqueued_at.remove(&client_request_id);
    session.queued_user_merge_targets.remove(&client_request_id);

    if session.command_queue.len() < initial_len {
        super::app_context::release(&chat_id, &client_request_id);
        session.touch();
        session.emit_queue_update();
        session.queue_notify.notify_one();
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
    use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
    use crate::chat::perf_diagnostics::{self, MemoryPerfSink, PerfClock, PerfRecorder};
    use crate::chat::types::SessionState;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

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
        let chunks = chunk_snapshot_messages(messages);
        let actual = chunks
            .iter()
            .flatten()
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
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match body.data().await {
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => break,
                    None => panic!("constituent EOF must terminate the group with an error"),
                }
            }
        })
        .await
        .expect("closed chat must not be hidden by sibling heartbeats");
        drop(body);
        assert_eq!(sender.receiver_count(), 0);
        assert_eq!(first.lock().await.event_tx.receiver_count(), 0);
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
