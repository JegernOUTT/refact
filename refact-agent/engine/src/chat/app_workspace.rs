use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use refact_chat_api::{
    apply_app_workspace_cas, newest_app_workspace, AppWorkspace, AppWorkspaceSetError,
    StoredAppWorkspace,
};

use crate::app_state::AppState;
use crate::chat::trajectories::maybe_save_trajectory_with_intent;
use crate::chat::types::{ChatEvent, ChatSession, TrajectoryCommitIntent};

pub(crate) const APP_WORKSPACE_KEY: &str = "app_workspace";

/// The only mutation path for a live session's workspace. Leaves the session
/// untouched on any error.
pub(crate) fn apply_set_app_workspace_locked(
    session: &mut ChatSession,
    expected_revision: u64,
    workspace: AppWorkspace,
) -> Result<AppWorkspace, AppWorkspaceSetError> {
    if crate::buddy::companion::is_companion(&session.thread) {
        return Err(AppWorkspaceSetError::Unavailable(
            "companion conversations cannot host apps".to_string(),
        ));
    }
    let stored = apply_app_workspace_cas(
        &mut session.thread.app_workspace,
        expected_revision,
        workspace,
    )?;
    session.emit(ChatEvent::ThreadUpdated {
        params: serde_json::json!({ APP_WORKSPACE_KEY: &stored }),
    });
    session.increment_version();
    session.touch();
    Ok(stored)
}

pub(crate) async fn set_app_workspace_in_session(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    expected_revision: u64,
    workspace: AppWorkspace,
) -> Result<AppWorkspace, AppWorkspaceSetError> {
    let lease = AppWorkspaceMutationLease::acquire(session_arc).await;
    lease.commit(app, expected_revision, workspace).await
}

/// Serializes binding changes with model app effects without holding ChatSession across IO.
pub(crate) struct AppWorkspaceMutationLease {
    session: Arc<AMutex<ChatSession>>,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl AppWorkspaceMutationLease {
    pub(crate) async fn acquire(session: Arc<AMutex<ChatSession>>) -> Self {
        let lock = session.lock().await.app_workspace_mutation_lock.clone();
        let guard = lock.lock_owned().await;
        Self {
            session,
            _guard: guard,
        }
    }

    pub(crate) async fn commit(
        &self,
        app: AppState,
        expected_revision: u64,
        workspace: AppWorkspace,
    ) -> Result<AppWorkspace, AppWorkspaceSetError> {
        let stored = {
            let mut session = self.session.lock().await;
            apply_set_app_workspace_locked(&mut session, expected_revision, workspace)?
        };
        maybe_save_trajectory_with_intent(
            app,
            self.session.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await;
        Ok(stored)
    }
}

/// Revision-checked workspace update for tools and APIs. Callers must perform
/// their own authorization for `chat_id` before calling.
pub async fn set_app_workspace(
    app: AppState,
    chat_id: &str,
    expected_revision: u64,
    workspace: AppWorkspace,
) -> Result<AppWorkspace, AppWorkspaceSetError> {
    crate::chat::trajectories::validate_trajectory_id(chat_id)
        .map_err(|e| AppWorkspaceSetError::Unavailable(e.message))?;
    let sessions = app.chat.sessions.clone();
    let session_arc =
        crate::chat::get_or_create_session_with_trajectory(app.clone(), &sessions, chat_id).await;
    set_app_workspace_in_session(app, session_arc, expected_revision, workspace).await
}

pub async fn get_app_workspace(app: AppState, chat_id: &str) -> Option<StoredAppWorkspace> {
    crate::chat::trajectories::validate_trajectory_id(chat_id).ok()?;
    let sessions = app.chat.sessions.clone();
    let session_arc =
        crate::chat::get_or_create_session_with_trajectory(app.clone(), &sessions, chat_id).await;
    let session = session_arc.lock().await;
    session.thread.app_workspace.clone()
}

pub(crate) fn stored_app_workspace_from_value(
    value: Option<&serde_json::Value>,
) -> Option<StoredAppWorkspace> {
    value
        .filter(|value| !value.is_null())
        .map(|value| StoredAppWorkspace::from_value(value.clone()))
}

/// Keeps the newest persisted workspace when a stale snapshot is written over a
/// file that already holds a later revision.
pub(crate) fn keep_newest_app_workspace_in_trajectory(
    trajectory: &mut serde_json::Value,
    existing: Option<&serde_json::Value>,
) {
    let Some(object) = trajectory.as_object_mut() else {
        return;
    };
    let incoming = stored_app_workspace_from_value(object.get(APP_WORKSPACE_KEY));
    let existing = stored_app_workspace_from_value(existing);
    match newest_app_workspace(incoming, existing) {
        Some(newest) => {
            if let Ok(value) = serde_json::to_value(&newest) {
                object.insert(APP_WORKSPACE_KEY.to_string(), value);
            }
        }
        None => {
            object.remove(APP_WORKSPACE_KEY);
        }
    }
}

/// A new chat derived from `source` (branch or handoff) gets independent bindings.
pub(crate) fn fork_app_workspace_for_new_chat(
    source: Option<&StoredAppWorkspace>,
) -> Option<StoredAppWorkspace> {
    source.and_then(StoredAppWorkspace::fork_for_new_chat)
}

/// Installs a forked workspace into an existing chat without regressing its
/// revision, so stale writers holding the previous revision still conflict.
pub(crate) fn install_forked_app_workspace(session: &mut ChatSession, forked: StoredAppWorkspace) {
    let StoredAppWorkspace::Supported(mut workspace) = forked else {
        return;
    };
    workspace.revision =
        refact_chat_api::app_workspace_revision(session.thread.app_workspace.as_ref())
            .saturating_add(1);
    session.emit(ChatEvent::ThreadUpdated {
        params: serde_json::json!({ APP_WORKSPACE_KEY: &workspace }),
    });
    session.thread.app_workspace = Some(StoredAppWorkspace::Supported(workspace));
}

#[cfg(test)]
mod transport_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    async fn workspace_http_large_save_retains_cas_and_command_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let request = |path: &str, body: Vec<u8>| {
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("host", "localhost:8001")
                .header("origin", "http://localhost:8001")
                .body(Body::from(body))
                .unwrap()
        };
        let workspace = json!({
            "schema_version": 1, "revision": 0,
            "bindings": [{
                "app_id": "11111111-1111-4111-8111-111111111111",
                "binding_id": "22222222-2222-4222-8222-222222222222",
                "params": {}, "state": {"history": "metric-42;".repeat(2 * 1024 * 1024)}
            }],
            "layout": {"kind": "leaf", "tabs": [{"kind": "chat", "ref": "self"}]}
        });
        let mut command = json!({
            "type": "set_app_workspace", "client_request_id": "large-save",
            "expected_revision": 0, "workspace": workspace
        });
        let bytes = serde_json::to_vec(&command).unwrap();
        assert!(bytes.len() > 15 * 1024 * 1024);
        let legacy = router
            .clone()
            .oneshot(request("/v1/chats/workspace-http/commands", bytes.clone()))
            .await
            .unwrap();
        assert_eq!(legacy.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let response = router
            .clone()
            .oneshot(request("/v1/chats/workspace-http/app-workspace", bytes))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value =
            serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
                .unwrap();
        let mut expected = workspace;
        expected["revision"] = json!(1);
        assert_eq!(result, json!({"revision": 1, "app_workspace": expected}));
        command["client_request_id"] = json!("stale-save");
        let conflict = router
            .clone()
            .oneshot(request(
                "/v1/chats/workspace-http/app-workspace",
                serde_json::to_vec(&command).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let refused = router.oneshot(request("/v1/chats/refused-workspace/app-workspace", serde_json::to_vec(&json!({"type": "set_params", "client_request_id": "wrong-command", "patch": {"model": "changed"}})).unwrap())).await.unwrap();
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert!(!app
            .chat
            .sessions
            .read()
            .await
            .contains_key("refused-workspace"));
        assert_eq!(
            serde_json::to_value(
                super::get_app_workspace(app.clone(), "workspace-http")
                    .await
                    .unwrap()
            )
            .unwrap(),
            expected
        );
        app.gcx.apps.shutdown().await;
    }
}
