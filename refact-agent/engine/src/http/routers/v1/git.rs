use chrono::{Utc, DateTime};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use axum::Extension;
use axum::http::{Response, StatusCode};
use git2::Repository;
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use url::Url;

use crate::call_validation::ChatMeta;
use crate::files_correction::{deserialize_path, serialize_path};
use crate::custom_error::ScratchError;
use crate::git::{CommitInfo, FileChange};
use crate::git::operations::{get_configured_author_email_and_name, stage_changes};
use crate::git::checkpoints::{
    preview_changes_for_workspace_checkpoint, preview_changes_for_workspace_checkpoint_for_root,
    restore_workspace_checkpoint, restore_workspace_checkpoint_for_root, Checkpoint,
};
use crate::global_context::GlobalContext;
use crate::worktrees::types::WorktreeMeta;

#[derive(Serialize, Deserialize, Debug)]
pub struct GitCommitPost {
    pub commits: Vec<CommitInfo>,
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
    gcx: Arc<ARwLock<GlobalContext>>,
    meta: &ChatMeta,
) -> Option<WorktreeMeta> {
    if let Some(worktree) = &meta.worktree {
        return Some(worktree.clone());
    }
    if meta.chat_id.is_empty() {
        return None;
    }
    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<GitCommitPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let mut error_log = Vec::new();
    let mut commits_applied = Vec::new();

    let abort_flag: Arc<AtomicBool> = gcx.read().await.git_operations_abort_flag.clone();
    for commit in post.commits {
        let project_path_str = commit.project_path.to_string();
        let project_name = commit
            .project_path
            .to_file_path()
            .ok()
            .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();

        let git_result: Result<(String, String, String), String> = (|| {
            let repo_path = crate::files_correction::canonical_path(
                &commit
                    .project_path
                    .to_file_path()
                    .unwrap_or_default()
                    .display()
                    .to_string(),
            );
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
                    project_path: commit.project_path,
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
                    gcx.clone(),
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
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
