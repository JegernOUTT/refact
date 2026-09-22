use axum::extract::{Path, State};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use crate::{app_state::AppState, chat::types::ChatSession, custom_error::ScratchError};

pub(crate) fn native_engine_identity() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

pub(crate) fn source_version(session: &ChatSession, root: &str) -> String {
    let source = serde_json::json!([
        root,
        native_engine_identity(),
        session.chat_id,
        session.created_at,
        session.runtime.state,
        session.runtime.paused,
        session.runtime.pause_reasons,
        session.runtime.accepted_tool_ids,
        session.runtime.paused_message_index,
        session
            .pending_browser_message
            .as_ref()
            .map(|p| &p.pending_message_id),
        session.closed,
        format!(
            "{:?}",
            session
                .confirmation_paused_at
                .as_ref()
                .map(|(instant, _)| instant)
        ),
        session
            .messages
            .iter()
            .filter(|message| message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|call| session
                    .runtime
                    .pause_reasons
                    .iter()
                    .any(|reason| reason.tool_call_id == call.id))))
            .map(|message| (&message.message_id, &message.tool_calls))
            .collect::<Vec<_>>()
    ]);
    format!("{:x}", Sha256::digest(source.to_string().as_bytes()))
}

pub(crate) fn validate_native_approval(
    session: &ChatSession,
    root: &str,
    engine_identity: &str,
    version: &str,
) -> Result<(), String> {
    if session.closed
        || engine_identity != native_engine_identity()
        || version != source_version(session, root)
    {
        return Err("Native approval source changed; refresh before responding".into());
    }
    Ok(())
}

#[derive(Serialize)]
pub struct NativeApprovalSession {
    chat_id: String,
    source_version: String,
    tools: Vec<refact_chat_api::PauseReason>,
    browser_request_id: Option<String>,
    warnings: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct NativeApprovalsSnapshot {
    schema_version: u16,
    project_root: String,
    engine_identity: String,
    coverage: &'static str,
    truncated: bool,
    warnings: Vec<&'static str>,
    sessions: Vec<NativeApprovalSession>,
}

pub async fn handle_current_approvals(
    State(app): State<AppState>,
) -> Result<axum::Json<NativeApprovalsSnapshot>, ScratchError> {
    let root = super::buddy_decisions::decision_root(&app).await?;
    let mut warnings = Vec::new();
    if !app.gcx.caps_state.read().await.last_error.is_empty() {
        warnings.push("native_provider_error");
    }
    if app
        .gcx
        .privacy_policy_load
        .read()
        .map(|load| load.error.is_some())
        .unwrap_or(true)
    {
        warnings.push("native_privacy_policy_error");
    }
    let mut loaded: Vec<_> = app
        .chat
        .sessions
        .read()
        .await
        .iter()
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    loaded.sort_by(|a, b| a.0.cmp(&b.0));
    let mut truncated = loaded.len() > 200;
    loaded.truncate(200);
    let mut sessions = Vec::new();
    for (chat_id, session) in loaded {
        let session = session.try_lock().map_err(|_| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Native approvals are busy; refresh status".into(),
            )
        })?;
        if session.closed {
            continue;
        }
        let mut warnings = Vec::new();
        if session.trajectory_save_error.is_some() {
            warnings.push("native_storage_error");
        }
        if session.runtime.error.is_some() {
            warnings.push("native_runtime_error");
        }
        if session.provider_usage_stale {
            warnings.push("provider_usage_stale");
        }
        let tools: Vec<_> = if session.runtime.state == refact_chat_api::SessionState::Paused {
            session
                .runtime
                .pause_reasons
                .iter()
                .filter(|reason| {
                    reason.reason_type == "confirmation"
                        && !session
                            .runtime
                            .accepted_tool_ids
                            .contains(&reason.tool_call_id)
                })
                .take(101)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let mut tools = tools;
        if tools.len() > 100 {
            tools.truncate(100);
            truncated = true;
        }
        sessions.push(NativeApprovalSession {
            chat_id,
            source_version: source_version(&session, &root),
            tools,
            browser_request_id: session
                .pending_browser_message
                .as_ref()
                .map(|p| p.pending_message_id.clone()),
            warnings,
        });
    }
    Ok(axum::Json(NativeApprovalsSnapshot {
        schema_version: 1,
        project_root: root,
        engine_identity: native_engine_identity().into(),
        coverage: "loaded_native_sessions",
        truncated,
        warnings,
        sessions,
    }))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApprovalResponse {
    Tool {
        tool_call_id: String,
        accepted: bool,
    },
    Browser {
        pending_message_id: String,
        include_actions: bool,
        include_console: bool,
        include_network: bool,
        include_mutations: bool,
        include_screenshot: bool,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeApprovalRequest {
    project_root: String,
    engine_identity: String,
    source_version: String,
    client_request_id: String,
    response: ApprovalResponse,
}

fn response_command(
    session: &ChatSession,
    response: ApprovalResponse,
) -> Result<refact_chat_api::ChatCommand, String> {
    match response {
        ApprovalResponse::Tool {
            tool_call_id,
            accepted,
        } => {
            if session.runtime.state != refact_chat_api::SessionState::Paused
                || session.runtime.accepted_tool_ids.contains(&tool_call_id)
                || !session
                    .runtime
                    .pause_reasons
                    .iter()
                    .any(|r| r.reason_type == "confirmation" && r.tool_call_id == tool_call_id)
            {
                return Err(
                    "Tool approval is not pending; IDE callbacks and denials are not approvals"
                        .into(),
                );
            }
            Ok(refact_chat_api::ChatCommand::ToolDecision {
                tool_call_id,
                accepted,
            })
        }
        ApprovalResponse::Browser {
            pending_message_id,
            include_actions,
            include_console,
            include_network,
            include_mutations,
            include_screenshot,
        } => {
            if !session
                .pending_browser_message
                .as_ref()
                .is_some_and(|p| p.pending_message_id == pending_message_id)
            {
                return Err("Browser context request is no longer pending".into());
            }
            Ok(refact_chat_api::ChatCommand::BrowserContextDecision {
                pending_message_id,
                include_actions,
                include_console,
                include_network,
                include_mutations,
                include_screenshot,
                last_n_actions: None,
                last_n_console: None,
                last_n_network: None,
            })
        }
    }
}

pub async fn handle_approval_response(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    axum::Json(request): axum::Json<NativeApprovalRequest>,
) -> Result<hyper::Response<hyper::Body>, ScratchError> {
    let root = super::buddy_decisions::decision_root(&app).await?;
    if root != request.project_root {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "Native project changed".into(),
        ));
    }
    if request.client_request_id.trim().is_empty() || request.client_request_id.len() > 256 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid request identity".into(),
        ));
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Native chat is not loaded".into())
        })?;
    let command = {
        let session = session.lock().await;
        validate_native_approval(
            &session,
            &root,
            &request.engine_identity,
            &request.source_version,
        )
        .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?;
        response_command(&session, request.response)
            .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?
    };
    let mut body = serde_json::to_value(command)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    body["client_request_id"] = serde_json::json!(request.client_request_id);
    body["native_approval_guard"] = serde_json::json!({
        "project_root": root, "engine_identity": request.engine_identity, "source_version": request.source_version,
    });
    crate::chat::handle_v1_chat_command(
        State(app),
        Path(chat_id),
        hyper::body::Bytes::from(body.to_string()),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paused() -> ChatSession {
        let mut session = ChatSession::new("chat".into());
        session.runtime.state = refact_chat_api::SessionState::Paused;
        session
            .runtime
            .pause_reasons
            .push(refact_chat_api::PauseReason {
                reason_type: "confirmation".into(),
                tool_call_id: "tool".into(),
                tool_name: "shell".into(),
                command: "echo hello".into(),
                rule: "confirm".into(),
                integr_config_path: None,
            });
        session
    }

    #[test]
    fn stale_root_engine_version_and_cross_client_state_never_consume() {
        let mut session = paused();
        let version = source_version(&session, "/project");
        assert!(
            validate_native_approval(&session, "/project", native_engine_identity(), &version)
                .is_ok()
        );
        assert!(
            validate_native_approval(&session, "/other", native_engine_identity(), &version)
                .is_err()
        );
        assert!(validate_native_approval(&session, "/project", "old-engine", &version).is_err());
        assert!(validate_native_approval(
            &session,
            "/project",
            native_engine_identity(),
            "old-version"
        )
        .is_err());
        assert!(response_command(
            &session,
            ApprovalResponse::Tool {
                tool_call_id: "wrong".into(),
                accepted: true
            }
        )
        .is_err());
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        session.runtime.accepted_tool_ids.push("tool".into());
        assert!(
            validate_native_approval(&session, "/project", native_engine_identity(), &version)
                .is_err()
        );
        assert_eq!(session.runtime.pause_reasons.len(), 1);
    }

    #[tokio::test]
    async fn queue_acceptance_preserves_pending_and_forwards_native_guard() {
        let root = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let root = root.path().to_string_lossy().into_owned();
        let session = std::sync::Arc::new(tokio::sync::Mutex::new(paused()));
        session
            .lock()
            .await
            .queue_processor_running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        app.chat
            .sessions
            .write()
            .await
            .insert("chat".into(), session.clone());
        let snapshot = handle_current_approvals(State(app.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(snapshot.coverage, "loaded_native_sessions");
        assert_eq!(snapshot.sessions[0].tools.len(), 1);
        for (project_root, tool_call_id, version) in [
            (
                "/wrong".to_string(),
                "tool",
                snapshot.sessions[0].source_version.clone(),
            ),
            (
                root.clone(),
                "wrong",
                snapshot.sessions[0].source_version.clone(),
            ),
            (root.clone(), "tool", "old".into()),
        ] {
            assert!(handle_approval_response(
                State(app.clone()),
                Path("chat".into()),
                axum::Json(NativeApprovalRequest {
                    project_root,
                    engine_identity: snapshot.engine_identity.clone(),
                    source_version: version,
                    client_request_id: "invalid".into(),
                    response: ApprovalResponse::Tool {
                        tool_call_id: tool_call_id.into(),
                        accepted: true
                    },
                })
            )
            .await
            .is_err());
        }
        assert!(session.lock().await.command_queue.is_empty());
        let result = handle_approval_response(
            State(app),
            Path("chat".into()),
            axum::Json(NativeApprovalRequest {
                project_root: root.clone(),
                engine_identity: snapshot.engine_identity,
                source_version: snapshot.sessions[0].source_version.clone(),
                client_request_id: "queued".into(),
                response: ApprovalResponse::Tool {
                    tool_call_id: "tool".into(),
                    accepted: false,
                },
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.status(), StatusCode::ACCEPTED);
        let mut session = session.lock().await;
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        let queued = session.command_queue.pop_front().unwrap();
        let wire = serde_json::to_value(&queued).unwrap();
        assert_eq!(wire["native_approval_guard"]["project_root"], root);
        assert!(matches!(
            queued.command,
            refact_chat_api::ChatCommand::ToolDecision {
                accepted: false,
                ..
            }
        ));
        session
            .queue_processor_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn browser_selection_is_typed_and_stale_responses_leave_request_available() {
        let mut session = paused();
        session.pending_browser_message = Some(crate::chat::types::PendingBrowserMessage {
            pending_message_id: "current".into(),
            content: serde_json::json!("message"),
            attachments: vec![],
            client_message_id: None,
            checkpoints: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            skill_activation_name: None,
            skill_context_msg: None,
        });
        let response = |id: &str| ApprovalResponse::Browser {
            pending_message_id: id.into(),
            include_actions: false,
            include_console: false,
            include_network: false,
            include_mutations: false,
            include_screenshot: true,
        };
        assert!(response_command(&session, response("stale")).is_err());
        assert_eq!(
            session
                .pending_browser_message
                .as_ref()
                .unwrap()
                .pending_message_id,
            "current"
        );
        let command = response_command(&session, response("current")).unwrap();
        assert!(matches!(
            command,
            refact_chat_api::ChatCommand::BrowserContextDecision {
                include_actions: false,
                include_console: false,
                include_network: false,
                include_mutations: false,
                include_screenshot: true,
                ..
            }
        ));
        assert_eq!(
            session
                .pending_browser_message
                .as_ref()
                .unwrap()
                .pending_message_id,
            "current"
        );
        let version = source_version(&session, "/project");
        session.pending_browser_message = None;
        assert!(
            validate_native_approval(&session, "/project", native_engine_identity(), &version)
                .is_err()
        );
        assert!(
            serde_json::from_value::<ApprovalResponse>(serde_json::json!({
                "kind": "browser", "pending_message_id": "current"
            }))
            .is_err()
        );
    }

    #[test]
    fn unknown_denial_and_ide_states_are_not_permission() {
        let mut session = paused();
        for reason in ["denial", "ide_callback", "unknown"] {
            session.runtime.pause_reasons[0].reason_type = reason.into();
            assert!(response_command(
                &session,
                ApprovalResponse::Tool {
                    tool_call_id: "tool".into(),
                    accepted: true
                }
            )
            .is_err());
        }
        session.runtime.pause_reasons[0].reason_type = "confirmation".into();
        session.runtime.state = refact_chat_api::SessionState::WaitingIde;
        assert!(response_command(
            &session,
            ApprovalResponse::Tool {
                tool_call_id: "tool".into(),
                accepted: true
            }
        )
        .is_err());
    }
}
