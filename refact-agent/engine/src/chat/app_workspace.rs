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
