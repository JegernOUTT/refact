use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::Extension;
use hyper::Body;
use tokio::sync::{broadcast, RwLock as ARwLock};

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

use super::types::*;
use super::session::get_or_create_session_with_trajectory;
use super::content::validate_content_with_attachments;
use super::queue::process_command_queue;
use super::trajectory_ops::sanitize_messages_for_model_switch;
use super::trajectories::validate_trajectory_id;
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

pub async fn handle_v1_chat_subscribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response<Body>, ScratchError> {
    let chat_id = params
        .get("chat_id")
        .ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "chat_id required".to_string()))?
        .clone();
    validate_trajectory_id(&chat_id)?;

    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };

    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;
    let session = session_arc.lock().await;
    let snapshot = session.snapshot();
    let mut rx = session.subscribe();
    let initial_seq = session.event_seq;
    drop(session);

    let initial_envelope = EventEnvelope {
        chat_id: chat_id.clone(),
        seq: initial_seq,
        event: snapshot,
    };

    let session_for_stream = session_arc.clone();
    let chat_id_for_stream = chat_id.clone();

    let stream = async_stream::stream! {
        let json = serde_json::to_string(&initial_envelope).unwrap_or_default();
        yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));

        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(envelope) => {
                            let json = serde_json::to_string(&envelope).unwrap_or_default();
                            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::info!("SSE subscriber lagged, skipped {} events, sending fresh snapshot", skipped);
                            let session = session_for_stream.lock().await;
                            let recovery_envelope = EventEnvelope {
                                chat_id: chat_id_for_stream.clone(),
                                seq: session.event_seq,
                                event: session.snapshot(),
                            };
                            drop(session);
                            let json = serde_json::to_string(&recovery_envelope).unwrap_or_default();
                            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat_interval.tick() => {
                    if session_for_stream.lock().await.closed {
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

pub async fn handle_v1_chat_command(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;

    let request: CommandRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };

    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;
    let mut session = session_arc.lock().await;

    if session.is_duplicate_request(&request.client_request_id) {
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

    if matches!(request.command, ChatCommand::Abort {}) {
        session.abort_stream();
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: true,
            result: Some(serde_json::json!({"aborted": true})),
        });
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"aborted"}"#))
            .unwrap());
    }

    if let ChatCommand::SetParams { ref patch } = request.command {
        let old_model = session.thread.model.clone();
        let old_mode = session.thread.mode.clone();
        let (mut changed, sanitized_patch) =
            super::queue::apply_setparams_patch(&mut session.thread, patch);

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
            let model_id = if session.thread.model.is_empty() { None } else { Some(session.thread.model.as_str()) };
            if let Some(mode_config) = get_mode_config(gcx.clone(), &session.thread.mode, model_id).await {
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
                if let Some(v) = defaults.automatic_patch {
                    if session.thread.automatic_patch != v {
                        session.thread.automatic_patch = v;
                        changed = true;
                    }
                }
            }
        }

        if session.thread.model != old_model {
            sanitize_messages_for_model_switch(&mut session.messages);
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
            if mode_changed {
                obj.insert("mode".to_string(), serde_json::json!(session.thread.mode));
                obj.insert("include_project_info".to_string(), serde_json::json!(session.thread.include_project_info));
                obj.insert("checkpoints_enabled".to_string(), serde_json::json!(session.thread.checkpoints_enabled));
                obj.insert("automatic_patch".to_string(), serde_json::json!(session.thread.automatic_patch));
            }
        }
        session.emit(ChatEvent::ThreadUpdated {
            params: patch_for_chat_sse,
        });
        if changed {
            session.increment_version();
            session.touch();
        }
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: true,
            result: Some(serde_json::json!({"applied": true})),
        });
        drop(session);
        if changed {
            super::trajectories::maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
        }
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"applied"}"#))
            .unwrap());
    }

    let is_critical = (session.runtime.state == SessionState::Paused
        && matches!(
            request.command,
            ChatCommand::ToolDecision { .. } | ChatCommand::ToolDecisions { .. }
        ))
        || (session.runtime.state == SessionState::WaitingIde
            && matches!(request.command, ChatCommand::IdeToolResult { .. }));

    if session.command_queue.len() >= max_queue_size() && !is_critical {
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: false,
            result: Some(serde_json::json!({"error": "queue full"})),
        });
        return Ok(Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"status":"queue_full"}"#))
            .unwrap());
    }

    let validation_error = match &request.command {
        ChatCommand::UserMessage {
            content,
            attachments,
        } => validate_content_with_attachments(content, attachments).err(),
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
        })).unwrap_or_else(|_| r#"{"status":"invalid_content"}"#.to_string());
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap());
    }

    if request.priority {
        let insert_pos = session
            .command_queue
            .iter()
            .position(|r| !r.priority)
            .unwrap_or(session.command_queue.len());
        session.command_queue.insert(insert_pos, request.clone());
    } else {
        session.command_queue.push_back(request.clone());
    }
    session.touch();
    session.emit_queue_update();

    session.emit(ChatEvent::Ack {
        client_request_id: request.client_request_id,
        accepted: true,
        result: Some(serde_json::json!({"queued": true})),
    });

    let queue_notify = session.queue_notify.clone();
    let processor_running = session.queue_processor_running.clone();
    drop(session);

    if !processor_running.swap(true, Ordering::SeqCst) {
        tokio::spawn(process_command_queue(gcx, session_arc, processor_running));
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((chat_id, client_request_id)): Path<(String, String)>,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;

    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };

    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;
    let mut session = session_arc.lock().await;

    let initial_len = session.command_queue.len();
    session
        .command_queue
        .retain(|r| r.client_request_id != client_request_id);

    if session.command_queue.len() < initial_len {
        session.touch();
        session.emit_queue_update();
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
