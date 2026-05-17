use axum::response::Result;
use axum::extract::State;
use serde::Serialize;
use std::path::PathBuf;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;

#[derive(Serialize)]
pub struct SetupStatusResponse {
    pub configured: bool,
    pub reasons: Vec<String>,
    pub detail: SetupStatusDetail,
}

#[derive(Serialize)]
pub struct SetupStatusDetail {
    pub project_root: Option<String>,
    pub has_agents_md: bool,
    pub has_knowledge: bool,
    pub has_trajectories: bool,
}

fn first_project_root(project_dirs: &[PathBuf]) -> Option<PathBuf> {
    project_dirs.first().cloned()
}

async fn dir_has_any_entries(dir: PathBuf) -> bool {
    match tokio::fs::read_dir(&dir).await {
        Ok(mut it) => it.next_entry().await.ok().flatten().is_some(),
        Err(_) => false,
    }
}

async fn path_exists(path: PathBuf) -> bool {
    tokio::fs::try_exists(&path).await.unwrap_or(false)
}

pub async fn handle_v1_setup_status(
    State(app): State<AppState>,
) -> Result<axum::Json<SetupStatusResponse>, ScratchError> {
    let gcx = app.gcx.clone();
    let project_dirs = get_project_dirs(gcx).await;
    let project_root = first_project_root(&project_dirs);

    if project_root.is_none() {
        return Ok(axum::Json(SetupStatusResponse {
            configured: true,
            reasons: vec![],
            detail: SetupStatusDetail {
                project_root: None,
                has_agents_md: false,
                has_knowledge: false,
                has_trajectories: false,
            },
        }));
    }

    let root = project_root.unwrap();
    let refact_dir = root.join(".refact");

    let has_agents_md = path_exists(root.join("AGENTS.md")).await;
    let has_knowledge = dir_has_any_entries(refact_dir.join("knowledge")).await;
    let has_trajectories = dir_has_any_entries(refact_dir.join("trajectories")).await;

    let mut reasons = Vec::new();
    if !has_agents_md {
        reasons.push("missing_agents_md".to_string());
    }
    if !has_knowledge {
        reasons.push("no_knowledge".to_string());
    }
    if !has_trajectories {
        reasons.push("no_trajectories".to_string());
    }

    let configured = reasons.is_empty();

    Ok(axum::Json(SetupStatusResponse {
        configured,
        reasons,
        detail: SetupStatusDetail {
            project_root: Some(root.to_string_lossy().to_string()),
            has_agents_md,
            has_knowledge,
            has_trajectories,
        },
    }))
}
