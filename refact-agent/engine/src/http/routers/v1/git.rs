use chrono::{Utc, DateTime};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use git2::Repository;
use hyper::Body;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::call_validation::ChatMeta;
use crate::files_correction::{deserialize_path, serialize_path};
use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::custom_error::ScratchError;
use crate::git::FileChange;
use crate::git::operations::{get_configured_author_email_and_name, stage_changes};
use crate::git::checkpoints::{
    preview_changes_for_workspace_checkpoint, preview_changes_for_workspace_checkpoint_for_root,
    restore_workspace_checkpoint, restore_workspace_checkpoint_for_root, Checkpoint,
};
use crate::worktrees::types::WorktreeMeta;

#[derive(Serialize, Deserialize, Debug)]
pub struct GitCommitPost {
    pub commits: Vec<GitCommitRequest>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GitCommitRequest {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub project_path: Option<Url>,
    pub commit_message: String,
    pub staged_changes: Vec<FileChange>,
    pub unstaged_changes: Vec<FileChange>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GitError {
    pub error_message: String,
    pub project_name: String,
    pub project_path: Url,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CheckpointsPost {
    pub checkpoints: Vec<Checkpoint>,
    pub meta: ChatMeta,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CheckpointsPreviewResponse {
    pub reverted_changes: Vec<WorkspaceChanges>,
    pub checkpoints_for_undo: Vec<Checkpoint>,
    #[serde(serialize_with = "serialize_datetime_utc")]
    pub reverted_to: DateTime<Utc>,
    pub error_log: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CheckpointsRestoreResponse {
    pub success: bool,
    pub error_log: Vec<String>,
}

fn serialize_datetime_utc<S: serde::Serializer>(
    dt: &DateTime<Utc>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

async fn checkpoint_worktree_for_meta(
    gcx: Arc<GlobalContext>,
    meta: &ChatMeta,
) -> Option<WorktreeMeta> {
    if let Some(worktree) = &meta.worktree {
        return Some(worktree.clone());
    }
    if meta.chat_id.is_empty() {
        return None;
    }
    let sessions = { gcx.chat_sessions.clone() };
    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(&meta.chat_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        let session = session_arc.lock().await;
        return session.thread.worktree.clone();
    }
    None
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct WorkspaceChanges {
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub workspace_folder: PathBuf,
    pub files_changed: Vec<FileChange>,
}

pub async fn handle_v1_git_commit(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<GitCommitPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let mut error_log = Vec::new();
    let mut commits_applied = Vec::new();

    let abort_flag: Arc<AtomicBool> = gcx.git_operations_abort_flag.clone();
    for commit in post.commits {
        let repo_path = if let Some(root) = commit.root.as_ref() {
            super::git_read::resolve_requested_root(gcx.clone(), root.clone()).await
        } else {
            commit
                .project_path
                .as_ref()
                .ok_or_else(|| "A commit root or project_path is required".to_string())
                .and_then(|project_path| {
                    project_path
                        .to_file_path()
                        .map_err(|_| "Invalid project_path file URL".to_string())
                })
                .map(|path| crate::files_correction::canonical_path(&path.display().to_string()))
        };
        let repo_path = match repo_path {
            Ok(repo_path) => repo_path,
            Err(error_message) => {
                let project_path = commit
                    .project_path
                    .clone()
                    .or_else(|| {
                        commit
                            .root
                            .as_ref()
                            .and_then(|root| Url::from_file_path(root).ok())
                    })
                    .unwrap_or_else(|| Url::parse("file:///").unwrap());
                let project_name = commit
                    .root
                    .as_ref()
                    .and_then(|root| {
                        PathBuf::from(root)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or_default();
                error_log.push(GitError {
                    error_message,
                    project_name,
                    project_path,
                });
                continue;
            }
        };
        let project_path = commit
            .project_path
            .clone()
            .or_else(|| Url::from_file_path(&repo_path).ok())
            .unwrap_or_else(|| Url::parse("file:///").unwrap());
        let project_path_str = project_path.to_string();
        let project_name = repo_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        let git_result: Result<(String, String, String), String> = (|| {
            let repository =
                Repository::open(&repo_path).map_err(|e| format!("Failed to open repo: {}", e))?;
            stage_changes(&repository, &commit.unstaged_changes, &abort_flag)?;
            let (author_email, author_name) = get_configured_author_email_and_name(&repository)?;
            let branch = repository
                .head()
                .map(|reference| git2::Branch::wrap(reference))
                .map_err(|e| format!("Failed to get current branch: {}", e))?;
            let commit_oid = crate::git::operations::commit(
                &repository,
                &branch,
                &commit.commit_message,
                &author_name,
                &author_email,
            )?;
            let buddy_desc: String = commit.commit_message.chars().take(80).collect();
            Ok((commit_oid.to_string(), project_name.clone(), buddy_desc))
        })();

        match git_result {
            Err(e) => {
                error_log.push(GitError {
                    error_message: e,
                    project_name,
                    project_path,
                });
            }
            Ok((oid_str, pname, desc)) => {
                commits_applied.push(serde_json::json!({
                    "project_name": pname,
                    "project_path": project_path_str,
                    "commit_oid": oid_str,
                }));
                let buddy_title = format!("Committed: {}", pname);
                crate::buddy::actor::buddy_apply(
                    crate::app_state::AppState::from_gcx(gcx.clone()).await,
                    crate::buddy::actor::BuddyMutation {
                        runtime_event: Some(crate::buddy::actor::make_runtime_event(
                            "git_commit",
                            &buddy_title,
                            "git",
                            &format!("git_commit_{}", oid_str),
                            "completed",
                            None,
                        )),
                        xp: 20,
                        activity: Some(crate::buddy::types::BuddyActivity {
                            icon: "🔀".to_string(),
                            title: buddy_title,
                            description: desc,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            activity_type: "git_commit".to_string(),
                            chat_id: None,
                            failure_category: None,
                            failure_summary: None,
                        }),
                        mood: Some("proud".to_string()),
                    },
                )
                .await;
            }
        }
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "commits_applied": commits_applied,
                "error_log": error_log,
            }))
            .unwrap(),
        ))
        .unwrap())
}

pub async fn handle_v1_checkpoints_preview(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<CheckpointsPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    if post.checkpoints.is_empty() {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "No checkpoints to restore".to_string(),
        ));
    }
    if post.checkpoints.len() > 1 {
        return Err(ScratchError::new(
            StatusCode::NOT_IMPLEMENTED,
            "Multiple checkpoints to restore not implemented yet".to_string(),
        ));
    }

    let checkpoint = post.checkpoints.first().unwrap();
    let worktree = checkpoint_worktree_for_meta(gcx.clone(), &post.meta).await;
    let preview_result = if let Some(worktree) = worktree.as_ref() {
        preview_changes_for_workspace_checkpoint_for_root(
            gcx.clone(),
            &worktree.root,
            checkpoint,
            &post.meta.chat_id,
        )
        .await
    } else {
        preview_changes_for_workspace_checkpoint(gcx.clone(), checkpoint, &post.meta.chat_id).await
    };

    let response = match preview_result {
        Ok((files_changed, reverted_to, checkpoint_for_undo)) => CheckpointsPreviewResponse {
            reverted_changes: vec![WorkspaceChanges {
                workspace_folder: checkpoint.workspace_folder.clone(),
                files_changed,
            }],
            checkpoints_for_undo: vec![checkpoint_for_undo],
            reverted_to,
            error_log: vec![],
        },
        Err(e) => CheckpointsPreviewResponse {
            error_log: vec![e],
            ..Default::default()
        },
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

pub async fn handle_v1_checkpoints_restore(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<CheckpointsPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    if post.checkpoints.is_empty() {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "No checkpoints to restore".to_string(),
        ));
    }
    if post.checkpoints.len() > 1 {
        return Err(ScratchError::new(
            StatusCode::NOT_IMPLEMENTED,
            "Multiple checkpoints to restore not implemented yet".to_string(),
        ));
    }

    let checkpoint = post.checkpoints.first().unwrap();
    let worktree = checkpoint_worktree_for_meta(gcx.clone(), &post.meta).await;
    let restore_result = if let Some(worktree) = worktree.as_ref() {
        restore_workspace_checkpoint_for_root(
            gcx.clone(),
            &worktree.root,
            checkpoint,
            &post.meta.chat_id,
        )
        .await
    } else {
        restore_workspace_checkpoint(gcx.clone(), checkpoint, &post.meta.chat_id).await
    };

    let response = match restore_result {
        Ok(_) => CheckpointsRestoreResponse {
            success: true,
            error_log: vec![],
        },
        Err(e) => CheckpointsRestoreResponse {
            error_log: vec![e],
            ..Default::default()
        },
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;
    use tower::ServiceExt;

    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        fs::write(dir.path().join("file.txt"), "base\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )
        .unwrap();
        drop(tree);
        fs::write(dir.path().join("file.txt"), "changed\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        drop(index);
        (dir, repo)
    }

    async fn post_commit(gcx: Arc<GlobalContext>, commit: Value) -> Value {
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/git-commit")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "commits": [commit] }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn commit_body(project_field: Value, message: &str) -> Value {
        let mut body = json!({
            "commit_message": message,
            "staged_changes": [],
            "unstaged_changes": [],
        });
        body.as_object_mut()
            .unwrap()
            .extend(project_field.as_object().unwrap().clone());
        body
    }

    #[tokio::test]
    async fn git_commit_accepts_plain_workspace_root() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (dir, repo) = init_repo();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![dir.path().to_path_buf()];

        let response = post_commit(
            gcx,
            commit_body(
                json!({ "root": dir.path().to_string_lossy() }),
                "Plain root commit",
            ),
        )
        .await;

        assert_eq!(response["commits_applied"].as_array().unwrap().len(), 1);
        assert!(response["error_log"].as_array().unwrap().is_empty());
        assert_eq!(
            repo.head().unwrap().peel_to_commit().unwrap().message(),
            Some("Plain root commit")
        );
    }

    #[tokio::test]
    async fn git_commit_maps_plain_workspace_subdir_to_enclosing_repo() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (dir, repo) = init_repo();
        let subdir = dir.path().join("refact-agent").join("engine");
        fs::create_dir_all(&subdir).unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![subdir.clone()];

        let response = post_commit(
            gcx,
            commit_body(
                json!({ "root": subdir.to_string_lossy() }),
                "Subdir root commit",
            ),
        )
        .await;

        assert_eq!(response["commits_applied"].as_array().unwrap().len(), 1);
        assert!(response["error_log"].as_array().unwrap().is_empty());
        assert_eq!(
            repo.head().unwrap().peel_to_commit().unwrap().message(),
            Some("Subdir root commit")
        );
    }

    #[tokio::test]
    async fn git_commit_keeps_legacy_project_path_request_working() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (dir, repo) = init_repo();
        let project_path = Url::from_file_path(dir.path()).unwrap();

        let response = post_commit(
            gcx,
            commit_body(
                json!({ "project_path": project_path }),
                "Legacy path commit",
            ),
        )
        .await;

        assert_eq!(response["commits_applied"].as_array().unwrap().len(), 1);
        assert!(response["error_log"].as_array().unwrap().is_empty());
        assert_eq!(
            repo.head().unwrap().peel_to_commit().unwrap().message(),
            Some("Legacy path commit")
        );
    }
}
