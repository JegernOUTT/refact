use std::collections::HashMap;
use std::sync::atomic::Ordering;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use tokio::sync::broadcast;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::worktrees::types::WorktreeMeta;

use super::types::*;
use super::queue::resolve_worktree_setparams_update;
use super::session::get_or_create_session_with_trajectory;
use super::content::{validate_content_with_attachments, validate_context_files};
use super::queue::process_command_queue;
use super::trajectory_ops::sanitize_messages_for_model_switch;
use super::trajectories::validate_trajectory_id;
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

fn worktree_activation_message(worktree: &WorktreeMeta) -> ChatMessage {
    let branch = worktree.branch.as_deref().unwrap_or("unknown");
    let base = worktree.base_branch.as_deref().unwrap_or("unknown");
    ChatMessage {
        role: "cd_instruction".to_string(),
        content: ChatContent::SimpleText(format!(
            "💿 WORKTREE_ENABLED\n\nActive worktree scope is now ON for this chat.\n\n- Worktree id: `{}`\n- Branch: `{}`\n- Base/target branch: `{}`\n- Worktree root: `{}`\n- Source workspace root: `{}`\n\nEffects for this thread:\n- File reads, edits, shell commands, searches, and @file resolution should operate inside the worktree root unless a tool explicitly says otherwise.\n- Treat the main workspace as the merge target and do not edit it directly for this chat.\n- Use relative paths as usual; absolute paths outside the worktree may be rejected or remapped.\n- To merge completed work, call `worktree_merge` or use the Worktrees UI merge action.\n- If you need to leave the isolated scope, ask the user to detach the worktree first.",
            worktree.id,
            branch,
            base,
            worktree.root.display(),
            worktree.source_workspace_root.display()
        )),
        tool_call_id: "worktree_enabled".to_string(),
        ..Default::default()
    }
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

pub async fn handle_v1_chat_subscribe(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response<Body>, ScratchError> {
    let chat_id = params
        .get("chat_id")
        .ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "chat_id required".to_string()))?
        .clone();
    validate_trajectory_id(&chat_id)?;

    let sessions = app.chat.sessions.clone();

    let session_arc = get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await;
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

    let initial_json = match serde_json::to_string(&initial_envelope) {
        Ok(j) => j,
        Err(e) => {
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
        yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", initial_json));

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
                            tracing::info!("SSE subscriber lagged, skipped {} events, sending fresh snapshot", skipped);
                            let session = session_for_stream.lock().await;
                            if session.closed {
                                break;
                            }
                            // Re-subscribe BEFORE capturing event_seq so we don't miss events
                            // emitted between snapshot and the new receiver start.
                            rx = session.subscribe();
                            let recovery_envelope = EventEnvelope {
                                chat_id: chat_id_for_stream.clone(),
                                seq: session.event_seq,
                                event: session.snapshot(),
                            };
                            drop(session);
                            match serde_json::to_string(&recovery_envelope) {
                                Ok(json) => yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json)),
                                Err(e) => {
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

pub async fn handle_v1_chat_command(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&chat_id)?;

    let request: CommandRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = app.chat.sessions.clone();

    let session_arc = get_or_create_session_with_trajectory(app.clone(), &sessions, &chat_id).await;
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
        let old_model = session.thread.model.clone();
        let old_mode = session.thread.mode.clone();
        let (mut changed, sanitized_patch) =
            super::queue::apply_setparams_patch(&mut session.thread, patch);
        let activated_worktree = worktree_update
            .as_ref()
            .filter(|update| update.changed)
            .and_then(|update| update.worktree.clone());
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
        if let Some(worktree) = activated_worktree {
            session.add_message(worktree_activation_message(&worktree));
        }
        session.emit(ChatEvent::Ack {
            client_request_id: request.client_request_id,
            accepted: true,
            result: Some(serde_json::json!({"applied": true})),
        });
        drop(session);
        if changed {
            super::trajectories::maybe_save_trajectory(app.clone(), session_arc.clone()).await;
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
            context_files,
            suppress_auto_enrichment: _,
        } => validate_content_with_attachments(content, attachments)
            .err()
            .or_else(|| validate_context_files(context_files).err()),
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

    if request.priority && matches!(&request.command, ChatCommand::UserMessage { .. }) {
        session.abort_stream();
        session.clear_pending_tool_calls_for_interruption();
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
