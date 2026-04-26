use std::path::PathBuf;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::files_in_workspace;

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
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
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

    let changed = {
        let gcx = global_context.write().await;
        let mut folders = gcx.documents_state.workspace_folders.lock().unwrap();
        if *folders == workspace_dirs {
            false
        } else {
            *folders = workspace_dirs;
            true
        }
    };

    let files_count = if changed {
        let n = files_in_workspace::on_workspaces_init(global_context.clone()).await;
        if let Some(tx) = global_context.read().await.workspace_changed_tx.as_ref() {
            let _ = tx.send(());
        }
        n
    } else {
        global_context
            .read()
            .await
            .documents_state
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
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
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
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
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
    global_context
        .write()
        .await
        .documents_state
        .active_file_path = Some(path);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": true}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_add_folder(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LspLikeAddFolder>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    let file_path = post.uri.to_file_path().map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("not a file:// URI: {}", post.uri),
        )
    })?;
    let cpath = crate::files_correction::canonical_path(file_path.to_string_lossy().into_owned());
    files_in_workspace::add_folder(global_context.clone(), &cpath).await;
    if let Some(tx) = global_context.read().await.workspace_changed_tx.as_ref() {
        let _ = tx.send(());
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}

pub async fn handle_v1_lsp_remove_folder(
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LspLikeAddFolder>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    let file_path = post.uri.to_file_path().map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("not a file:// URI: {}", post.uri),
        )
    })?;
    let cpath = crate::files_correction::canonical_path(file_path.to_string_lossy().into_owned());
    files_in_workspace::remove_folder(global_context.clone(), &cpath).await;
    if let Some(tx) = global_context.read().await.workspace_changed_tx.as_ref() {
        let _ = tx.send(());
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"success": 1}).to_string()))
        .unwrap())
}
