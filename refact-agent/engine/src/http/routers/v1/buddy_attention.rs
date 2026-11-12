//! Read-only attention projection. A row is never itself authorization to act.
use axum::extract::State;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{app_state::AppState, custom_error::ScratchError};
use super::buddy_native_approvals::{native_engine_identity, source_version};

#[derive(Serialize)]
pub struct AttentionRow {
    id: String,
    kind: &'static str,
    chat_id: String,
    source_version: Option<String>,
    reason: String,
    effect: &'static str,
    actionable: bool,
    literal_command: Option<String>,
    cwd: Option<String>,
    tool_call_id: Option<String>,
    question: Option<refact_chat_api::PendingQuestionSet>,
    parent_chat_id: Option<String>,
    run_generation: Option<u64>,
    expires_at: Option<u64>,
    display_command: Option<String>,
    tool_name: Option<String>,
    tool_arguments: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct AttentionSnapshot {
    schema_version: u16,
    project_root: String,
    engine_identity: String,
    coverage: &'static str,
    partial: bool,
    busy_chat_ids: Vec<String>,
    rows: Vec<AttentionRow>,
}

fn identity(root: &str, chat_id: &str, created_at: &str, kind: &str, request: &str) -> String {
    let key = serde_json::json!([root, chat_id, created_at, kind, request]);
    format!("{:x}", Sha256::digest(key.to_string().as_bytes()))
}

pub async fn handle_current_attention(
    State(app): State<AppState>,
) -> Result<axum::Json<AttentionSnapshot>, ScratchError> {
    let root = crate::buddy::companion::project_root(app.gcx.clone())
        .await
        .map_err(|error| ScratchError::new(hyper::StatusCode::SERVICE_UNAVAILABLE, error))?
        .to_string_lossy()
        .into_owned();
    let mut loaded: Vec<_> = app
        .chat
        .sessions
        .read()
        .await
        .iter()
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    loaded.sort_by(|a, b| a.0.cmp(&b.0));
    let loaded_ids: std::collections::HashSet<_> =
        loaded.iter().map(|(id, _)| id.clone()).collect();
    let agents = app.agents.list_all().await;
    let mut rows = Vec::new();
    let mut busy_chat_ids = Vec::new();
    for (chat_id, session) in loaded {
        let Ok(session) = session.try_lock() else {
            busy_chat_ids.push(chat_id);
            continue;
        };
        if session.closed {
            continue;
        }
        let Some(project) = crate::coordination::scope::session_source_root(&session) else {
            busy_chat_ids.push(chat_id);
            continue;
        };
        if crate::coordination::roots::project_identity(&project)
            != crate::coordination::roots::project_identity(std::path::Path::new(&root))
        {
            continue;
        }
        let created = session.created_at.to_string();
        let version = source_version(&session, &root);
        if session.runtime.state == refact_chat_api::SessionState::Paused {
            for reason in &session.runtime.pause_reasons {
                if session
                    .runtime
                    .accepted_tool_ids
                    .contains(&reason.tool_call_id)
                {
                    continue;
                }
                let actionable = reason.reason_type == "confirmation";
                let call = session
                    .runtime
                    .paused_message_index
                    .and_then(|index| session.messages.get(index))
                    .and_then(|message| message.tool_calls.as_ref())
                    .and_then(|calls| calls.iter().find(|call| call.id == reason.tool_call_id));
                let arguments = call.and_then(|call| {
                    serde_json::from_str::<serde_json::Value>(&call.function.arguments).ok()
                });
                let literal_command = call
                    .filter(|call| matches!(call.function.name.as_str(), "shell" | "process_start"))
                    .and_then(|_| {
                        arguments
                            .as_ref()?
                            .get("command")?
                            .as_str()
                            .map(str::to_owned)
                    });
                let effect = if crate::chat::tools::is_shell_privacy_approval(reason) {
                    "share_guarded_output"
                } else if crate::chat::cache_guard::is_cache_guard_pause_reason(reason) {
                    "cache_extra_cost"
                } else {
                    "execute_pending_tool"
                };
                rows.push(AttentionRow {
                    id: identity(&root, &chat_id, &created, "tool", &reason.tool_call_id),
                    kind: if actionable {
                        "tool_permission"
                    } else {
                        "non_approvable_pause"
                    },
                    chat_id: chat_id.clone(),
                    source_version: Some(version.clone()),
                    reason: reason.rule.clone(),
                    effect,
                    actionable,
                    // PauseReason.command is a display string, not a verified literal command.
                    literal_command,
                    cwd: None,
                    tool_call_id: Some(reason.tool_call_id.clone()),
                    question: None,
                    parent_chat_id: None,
                    run_generation: None,
                    expires_at: None,
                    display_command: Some(reason.command.clone()),
                    tool_name: Some(reason.tool_name.clone()),
                    tool_arguments: arguments,
                });
            }
        }
        if let Some(browser) = &session.pending_browser_message {
            rows.push(AttentionRow {
                id: identity(
                    &root,
                    &chat_id,
                    &created,
                    "browser",
                    &browser.pending_message_id,
                ),
                kind: "browser_context",
                chat_id: chat_id.clone(),
                source_version: Some(version),
                reason: "Browser context requested".into(),
                effect: "browser_context_release",
                actionable: true,
                literal_command: None,
                cwd: None,
                tool_call_id: Some(browser.pending_message_id.clone()),
                question: None,
                parent_chat_id: None,
                run_generation: None,
                expires_at: None,
                display_command: None,
                tool_name: None,
                tool_arguments: None,
            });
        }
        for question in &session.pending_questions {
            if question.status != refact_chat_api::PendingQuestionStatus::Pending {
                continue;
            }
            rows.push(AttentionRow {
                id: identity(
                    &root,
                    &chat_id,
                    &created,
                    "question",
                    &format!("{}:{}", question.tool_call_id, question.asked_at_ms),
                ),
                kind: "question",
                chat_id: chat_id.clone(),
                source_version: None,
                reason: "Response requested".into(),
                effect: "question_response",
                actionable: true,
                literal_command: None,
                cwd: None,
                tool_call_id: Some(question.tool_call_id.clone()),
                question: Some(question.clone()),
                parent_chat_id: None,
                run_generation: None,
                expires_at: None,
                display_command: None,
                tool_name: None,
                tool_arguments: None,
            });
        }
        if let Some(error) = session
            .runtime
            .error
            .as_ref()
            .filter(|_| session.runtime.state == refact_chat_api::SessionState::Error)
        {
            rows.push(AttentionRow {
                id: identity(&root, &chat_id, &created, "runtime_error", error),
                kind: "runtime_failure",
                chat_id: chat_id.clone(),
                source_version: None,
                reason: error.clone(),
                effect: "navigation_only",
                actionable: false,
                literal_command: None,
                cwd: None,
                tool_call_id: None,
                question: None,
                parent_chat_id: None,
                run_generation: None,
                expires_at: None,
                display_command: None,
                tool_name: None,
                tool_arguments: None,
            });
        }
        for pending in app
            .agent_approvals
            .list_for_parent(&chat_id, crate::agents::approval::now_ms())
        {
            let execution = pending.execution.as_ref();
            rows.push(AttentionRow {
                id: identity(&root, &chat_id, "", "child_approval", &pending.id),
                kind: "child_permission",
                chat_id: agents
                    .iter()
                    .find(|agent| agent.agent_id == pending.child_work_id)
                    .and_then(|agent| agent.child_chat_id.clone())
                    .unwrap_or_else(|| chat_id.clone()),
                source_version: None,
                reason: format!("{} requires parent-mediated approval", pending.tool_name),
                effect: "navigation_only",
                actionable: false,
                literal_command: execution.map(|e| e.command.clone()),
                cwd: execution.map(|e| e.cwd.clone()),
                tool_call_id: Some(pending.id),
                question: None,
                parent_chat_id: Some(chat_id.clone()),
                run_generation: Some(pending.run_generation),
                expires_at: Some(pending.expires_at),
                display_command: None,
                tool_name: Some(pending.tool_name),
                tool_arguments: Some(pending.args),
            });
        }
    }
    // Discover pending child approvals even when their parent chat has not been opened.
    let mut parents = std::collections::BTreeSet::new();
    for agent in &agents {
        if !agent.status.is_terminal() && !loaded_ids.contains(&agent.parent_chat_id) {
            match app.agent_executions.saved_scope(&agent.agent_id) {
                Some(scope)
                    if crate::coordination::roots::project_identity(&scope)
                        == crate::coordination::roots::project_identity(std::path::Path::new(
                            &root,
                        )) =>
                {
                    parents.insert(agent.parent_chat_id.clone());
                }
                Some(_) => {}
                None => busy_chat_ids.push(agent.parent_chat_id.clone()),
            }
        }
    }
    for parent in parents {
        for pending in app
            .agent_approvals
            .list_for_parent(&parent, crate::agents::approval::now_ms())
        {
            let execution = pending.execution.as_ref();
            rows.push(AttentionRow {
                id: identity(&root, &parent, "", "child_approval", &pending.id),
                kind: "child_permission",
                chat_id: agents
                    .iter()
                    .find(|agent| agent.agent_id == pending.child_work_id)
                    .and_then(|agent| agent.child_chat_id.clone())
                    .unwrap_or_else(|| parent.clone()),
                source_version: None,
                reason: format!("{} requires parent-mediated approval", pending.tool_name),
                effect: "navigation_only",
                actionable: false,
                literal_command: execution.map(|e| e.command.clone()),
                cwd: execution.map(|e| e.cwd.clone()),
                tool_call_id: Some(pending.id),
                question: None,
                parent_chat_id: Some(parent.clone()),
                run_generation: Some(pending.run_generation),
                expires_at: Some(pending.expires_at),
                display_command: None,
                tool_name: Some(pending.tool_name),
                tool_arguments: Some(pending.args),
            });
        }
    }
    Ok(axum::Json(AttentionSnapshot {
        schema_version: 1,
        project_root: root,
        engine_identity: native_engine_identity().into(),
        // Loaded sessions do not represent every running subagent; never claim full coverage.
        coverage: "loaded_native_sessions_and_registered_running_agents",
        partial: true,
        busy_chat_ids,
        rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn attention_includes_late_session_and_excludes_foreign_project() {
        let root =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let foreign =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        for i in 0..202 {
            let id = format!("chat-{i:03}");
            let mut session = crate::chat::types::ChatSession::new(id.clone());
            session.work_project_root = Some(if i == 200 {
                foreign.path().to_path_buf()
            } else {
                root.path().to_path_buf()
            });
            if i >= 200 {
                session.runtime.state = refact_chat_api::SessionState::Error;
                session.runtime.error = Some("needs attention".into());
            }
            app.chat
                .sessions
                .write()
                .await
                .insert(id, Arc::new(Mutex::new(session)));
        }
        let snapshot = handle_current_attention(State(app)).await.unwrap().0;
        assert!(snapshot.rows.iter().any(|row| row.chat_id == "chat-201"));
        assert!(!snapshot.rows.iter().any(|row| row.chat_id == "chat-200"));
    }
}
