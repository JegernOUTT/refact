//! Scope comes from a live caller and trusted workspace state, never a supplied
//! root path or a guessed work ID. Discovery conveys no mutation authority.

use std::path::{Path, PathBuf};

use crate::app_state::AppState;
use crate::caps::BaseModelRecord;
use crate::chat::types::ChatSession;

use super::error::WorkError;
use super::roots;

#[derive(Clone)]
pub struct WorkCaller {
    pub source_root: PathBuf,
    pub checkout_root: PathBuf,
    pub root_chat_id: Option<String>,
    pub principal: String,
    pub model: Option<BaseModelRecord>,
}

pub fn session_source_root(session: &ChatSession) -> Option<PathBuf> {
    session
        .thread
        .worktree
        .as_ref()
        .map(|w| w.source_workspace_root.clone())
        .or_else(|| session.work_project_root.clone())
}

/// Bind a restored normal trajectory only when its storage location proves
/// project membership. Global/foreign history is not silently adopted by the
/// currently selected project. Worktree metadata is validated on use.
pub async fn restored_source_root(app: &AppState, source_path: &Path) -> Option<PathBuf> {
    let canonical = tokio::fs::canonicalize(source_path).await.ok()?;
    for root in crate::files_correction::get_project_dirs(app.gcx.clone()).await {
        let Ok(root) = tokio::fs::canonicalize(root).await else {
            continue;
        };
        if canonical.starts_with(root.join(".refact")) {
            return Some(dunce::simplified(&root).to_path_buf());
        }
    }
    None
}

pub async fn model_caller(
    app: &AppState,
    chat_id: &str,
    model_id: &str,
) -> Result<WorkCaller, WorkError> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or_else(|| {
            WorkError::new(
                "not_found_or_not_visible",
                "caller chat session not found or not visible",
                false,
            )
        })?;
    let (source, checkout, root, worktree_id) = {
        let session = session.lock().await;
        if !session.thread.model.is_empty() && session.thread.model != model_id {
            return Err(WorkError::new(
                "policy_unavailable",
                "caller model changed; request a new launch",
                false,
            ));
        }
        let source = session_source_root(&session).ok_or_else(|| {
            WorkError::new(
                "scope_denied",
                "caller chat has no trusted project scope",
                false,
            )
        })?;
        let checkout = session
            .thread
            .worktree
            .as_ref()
            .map(|w| w.root.clone())
            .unwrap_or_else(|| source.clone());
        let root = session
            .thread
            .root_chat_id
            .clone()
            .unwrap_or_else(|| session.chat_id.clone());
        (
            source,
            checkout,
            root,
            session.thread.worktree.as_ref().map(|w| w.id.clone()),
        )
    };
    let source_root = roots::resolve_source_workspace_root(app.gcx.clone(), source.to_str())
        .await
        .map_err(|_| {
            WorkError::new(
                "scope_denied",
                "caller project scope is not available or authorized",
                false,
            )
        })?;
    if let Some(id) = worktree_id {
        let roots = roots::load_project_roots(app.gcx.clone(), source_root.clone()).await;
        let registered = roots
            .get(&id)
            .and_then(|root| root.path.as_ref())
            .ok_or_else(|| {
                WorkError::new("scope_denied", "caller worktree is not registered", false)
            })?;
        if canonical_candidate(Path::new(registered))? != canonical_candidate(&checkout)? {
            return Err(WorkError::new(
                "scope_denied",
                "caller checkout does not match its registered worktree",
                false,
            ));
        }
    }
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .map_err(|_| {
            WorkError::new(
                "policy_unavailable",
                "caller model capabilities could not be loaded",
                true,
            )
        })?;
    let model = crate::caps::resolve_chat_model(caps, model_id)
        .map_err(|_| {
            WorkError::new(
                "policy_unavailable",
                "caller model is not available in the current catalog",
                true,
            )
        })?
        .base
        .clone();
    Ok(WorkCaller {
        source_root,
        checkout_root: checkout,
        root_chat_id: Some(root),
        principal: format!("chat:{chat_id}"),
        model: Some(model),
    })
}

/// Only authenticated/local-transport HTTP handlers may construct this view.
pub async fn operator_caller(app: &AppState) -> Result<WorkCaller, WorkError> {
    let source_root = roots::resolve_source_workspace_root(app.gcx.clone(), None)
        .await
        .map_err(|_| WorkError::scope_denied())?;
    Ok(WorkCaller {
        checkout_root: source_root.clone(),
        source_root,
        root_chat_id: None,
        principal: "local_operator".into(),
        model: None,
    })
}

/// File selectors use canonical identities (including the nearest existing
/// parent for a not-yet-created path). String substring matching is not scope.
pub fn resolve_path(raw: &str, caller: &WorkCaller) -> Result<PathBuf, WorkError> {
    if raw.trim().is_empty() || raw.chars().any(char::is_control) {
        return Err(WorkError::invalid(
            "path must be nonempty and contain no control characters",
        ));
    }
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        caller.checkout_root.join(path)
    };
    let canonical = canonical_candidate(&path)?;
    let checkout = canonical_candidate(&caller.checkout_root)?;
    let source = canonical_candidate(&caller.source_root)?;
    if !canonical.starts_with(&checkout) && !canonical.starts_with(&source) {
        return Err(WorkError::scope_denied());
    }
    Ok(canonical)
}

pub fn canonical_candidate(path: &Path) -> Result<PathBuf, WorkError> {
    // Reject traversal rather than normalizing before resolving symlinks:
    // `link/../file` need not mean the same thing as `file` on the filesystem.
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(WorkError::scope_denied());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(root) => {
                let mut canonical = dunce::simplified(&root).to_path_buf();
                for part in missing.into_iter().rev() {
                    canonical.push(part);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A dangling symlink must not be mistaken for an absent leaf.
                if std::fs::symlink_metadata(ancestor).is_ok() {
                    return Err(WorkError::scope_denied());
                }
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(WorkError::scope_denied)?
                        .to_os_string(),
                );
                ancestor = ancestor.parent().ok_or_else(WorkError::scope_denied)?;
            }
            Err(_) => return Err(WorkError::scope_denied()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn caller_fixture() -> (
        AppState,
        std::sync::Arc<tokio::sync::Mutex<ChatSession>>,
        tempfile::TempDir,
    ) {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().canonicalize().unwrap()];
        let (root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            std::sync::Arc::new(crate::caps::ChatModelRecord {
                base: BaseModelRecord {
                    id: "test/model".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(std::sync::Arc::new(caps));
            state.loaded_project_root = Some(root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let app = AppState::from_gcx(gcx).await;
        let mut parent = ChatSession::new("parent".into());
        parent.thread.model = "test/model".into();
        parent.work_project_root = Some(workspace.path().into());
        let parent = std::sync::Arc::new(tokio::sync::Mutex::new(parent));
        app.chat
            .sessions
            .write()
            .await
            .insert("parent".into(), parent.clone());
        (app, parent, workspace)
    }

    #[tokio::test]
    async fn model_caller_waits_for_parent_session_update() {
        let (app, parent, workspace) = caller_fixture().await;
        let mut update = parent.lock().await;
        let caller = model_caller(&app, "parent", "test/model");
        tokio::pin!(caller);
        assert!(futures::poll!(&mut caller).is_pending());
        assert!(app.chat.sessions.try_write().is_ok());
        update.last_activity = std::time::Instant::now();
        drop(update);

        let caller = caller.await.unwrap();
        assert_eq!(caller.source_root, workspace.path().canonicalize().unwrap());
        assert_eq!(caller.checkout_root, workspace.path());
        assert_eq!(caller.model.unwrap().id, "test/model");
    }

    #[tokio::test]
    async fn model_caller_rejects_model_change_after_waiting_for_parent() {
        let (app, parent, _workspace) = caller_fixture().await;
        let mut update = parent.lock().await;
        let caller = model_caller(&app, "parent", "test/model");
        tokio::pin!(caller);
        assert!(futures::poll!(&mut caller).is_pending());
        update.thread.model = "different/model".into();
        drop(update);

        let error = caller.await.err().expect("changed caller must be rejected");
        assert_eq!(error.code, "policy_unavailable");
        assert_eq!(error.message, "caller model changed; request a new launch");
        assert!(!error.retryable);
    }

    #[test]
    fn file_scope_rejects_traversal_and_prefix_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let caller = WorkCaller {
            source_root: root.clone(),
            checkout_root: root.clone(),
            root_chat_id: None,
            principal: "test".into(),
            model: None,
        };
        assert_eq!(
            resolve_path("new/sub/file.rs", &caller).unwrap(),
            root.join("new/sub/file.rs")
        );
        assert!(resolve_path("../private", &caller).is_err());
        assert!(resolve_path(
            dir.path().join("project-other/file").to_str().unwrap(),
            &caller
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn absent_leaf_under_foreign_symlink_cannot_bypass_scope() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), dir.path().join("alias")).unwrap();
        let caller = WorkCaller {
            source_root: dir.path().into(),
            checkout_root: dir.path().into(),
            root_chat_id: None,
            principal: "test".into(),
            model: None,
        };
        assert!(resolve_path("alias/not-yet-created", &caller).is_err());
    }
}
