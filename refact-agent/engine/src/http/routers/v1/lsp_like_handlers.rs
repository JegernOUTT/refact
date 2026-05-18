use std::path::PathBuf;

use axum::response::Result;
use axum::extract::State;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_in_workspace;
use crate::lsp::{
    add_workspace_root_to_set, canonical_workspace_roots, remove_workspace_root_from_set,
    workspace_roots_changed,
};

#[derive(Serialize, Deserialize, Clone)]
pub struct LspLikeInit {
    pub project_roots: Vec<Url>,
}

#[derive(Serialize, Deserialize, Clone)]
struct LspLikeDidChange {
    pub uri: Url,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct LspLikeSetActiveDocument {
    pub uri: Url,
}

#[derive(Serialize, Deserialize, Clone)]
struct LspLikeAddFolder {
    pub uri: Url,
}

pub async fn handle_v1_lsp_initialize(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<LspLikeInit>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;

    let mut workspace_dirs: Vec<PathBuf> = vec![];
    for x in post.project_roots {
        let file_path = x.to_file_path().map_err(|_| {
            ScratchError::new(StatusCode::BAD_REQUEST, format!("not a file:// URI: {}", x))
        })?;
        workspace_dirs.push(crate::files_correction::canonical_path(
            file_path.to_string_lossy().into_owned(),
        ));
    }

    let workspace_dirs = canonical_workspace_roots(&workspace_dirs);
    let changed = {
        let mut folders = app.gcx.documents_state.workspace_folders.lock().unwrap();
        if workspace_roots_changed(&folders, &workspace_dirs) {
            *folders = workspace_dirs;
            true
        } else {
            false
        }
    };

    let files_count = if changed {
        let n = files_in_workspace::on_workspaces_init(global_context.clone()).await;
        if let Some(tx) = global_context.workspace_changed_tx.as_ref() {
            let _ = tx.send(());
        }
        n
    } else {
        global_context.documents_state
            .workspace_files
            .lock()
            .unwrap()
            .len() as i32
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(
            json!({"success": 1, "files_found": files_count}).to_string(),
        ))
        .unwrap())
}

pub async fn handle_v1_lsp_did_change(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<LspLikeDidChange>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    let file_path = post.uri.to_file_path().map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("not a file:// URI: {}", post.uri),
        )
    })?;
    let cpath = crate::files_correction::canonical_path(file_path.to_string_lossy().into_owned());
    files_in_workspace::on_did_change(global_context.clone(), &cpath, &post.text).await;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}

pub async fn handle_v1_set_active_document(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<LspLikeSetActiveDocument>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    let file_path = post.uri.to_file_path().map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("not a file:// URI: {}", post.uri),
        )
    })?;
    let path = crate::files_correction::canonical_path(file_path.to_string_lossy().into_owned());
    tracing::info!(
        "ACTIVE_DOC {:?}",
        crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30)
    );
    *global_context.documents_state
        .active_file_path
        .lock()
        .await = Some(path);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": true}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_add_folder(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<LspLikeAddFolder>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    let file_path = post.uri.to_file_path().map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("not a file:// URI: {}", post.uri),
        )
    })?;
    let cpath = crate::files_correction::canonical_path(file_path.to_string_lossy().into_owned());
    let changed = {
        let mut folders = app.gcx.documents_state.workspace_folders.lock().unwrap();
        add_workspace_root_to_set(&mut folders, cpath)
    };
    if changed {
        files_in_workspace::on_workspaces_init(global_context.clone()).await;
        if let Some(tx) = global_context.workspace_changed_tx.as_ref() {
            let _ = tx.send(());
        }
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_remove_folder(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<LspLikeAddFolder>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    let file_path = post.uri.to_file_path().map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("not a file:// URI: {}", post.uri),
        )
    })?;
    let cpath = crate::files_correction::canonical_path(file_path.to_string_lossy().into_owned());
    let changed = {
        let mut folders = app.gcx.documents_state.workspace_folders.lock().unwrap();
        remove_workspace_root_from_set(&mut folders, &cpath)
    };
    if changed {
        files_in_workspace::on_workspaces_init(global_context.clone()).await;
        if let Some(tx) = global_context.workspace_changed_tx.as_ref() {
            let _ = tx.send(());
        }
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}
