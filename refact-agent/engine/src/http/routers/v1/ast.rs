use std::collections::HashSet;
use axum::extract::State;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;
use serde_json::json;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_in_workspace::{Document, get_file_text_from_memory_or_disk};
use crate::postprocessing::pp_context_files::pp_color_lines;
use crate::postprocessing::pp_utils::{context_msgs_from_paths, pp_ast_markup_files};
use crate::call_validation::PostprocessSettings;

#[derive(Serialize, Deserialize, Clone)]
struct AstFileUrlPost {
    file_url: Url,
}

#[derive(Serialize, Deserialize, Clone)]
struct FileNameOnlyPost {
    file_name: String,
}

pub async fn handle_v1_ast_file_dump(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<FileNameOnlyPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;

    let candidates = crate::files_correction::correct_to_nearest_filename(
        global_context.clone(),
        &post.file_name,
        false,
        1,
    )
    .await;
    if candidates.len() != 1 {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(serde_json::to_string_pretty(&json!({"detail": format!("file not found or ambiguous, candidates {:?}", candidates)})).unwrap()))
            .unwrap());
    }
    let mut files_set: HashSet<String> = HashSet::new();
    files_set.insert(candidates[0].clone());
    let mut context_file_vec = context_msgs_from_paths(global_context.clone(), files_set).await;

    let files_markup = pp_ast_markup_files(global_context.clone(), &mut context_file_vec).await;
    let mut settings = PostprocessSettings::new();
    settings.close_small_gaps = false;
    let lines_in_files = pp_color_lines(&vec![], files_markup, &settings).await;
    let mut result = "".to_string();
    for linevec in lines_in_files.values() {
        for lineref in linevec {
            result.push_str(
                format!(
                    "{}:{:04} {:<43} {:>7.3} {}\n",
                    crate::nicer_logs::last_n_chars(&lineref.file_ref.cpath, 30),
                    lineref.line_n,
                    crate::nicer_logs::first_n_chars(&lineref.line_content, 40),
                    lineref.useful,
                    lineref.color,
                )
                .as_str(),
            );
        }
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(result))
        .unwrap())
}

pub async fn handle_v1_ast_file_symbols(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<AstFileUrlPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;

    let corrected = crate::files_correction::correct_to_nearest_filename(
        global_context.clone(),
        &post
            .file_url
            .to_file_path()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        false,
        1,
    )
    .await;

    if corrected.len() == 0 {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(
                serde_json::to_string_pretty(&json!({"detail": "File not found"})).unwrap(),
            ))
            .unwrap());
    }

    let cpath = corrected[0].clone();
    let mut doc = Document::new(&cpath.into());
    let file_text = get_file_text_from_memory_or_disk(global_context.clone(), &doc.doc_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    doc.update_text(&file_text);

    let codegraph_opt = global_context.codegraph.lock().await.clone();
    let search_res = match codegraph_opt {
        Some(service) => service
            .doc_defs(&doc.doc_path.to_string_lossy().to_string())
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?,
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "codegraph is not available".to_string(),
            ));
        }
    };
    let json_string = serde_json::to_string_pretty(&search_res).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization problem: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json_string))
        .unwrap())
}

pub async fn handle_v1_ast_status(
    State(app): State<AppState>,
    _: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let status = crate::codegraph::cg_status::get_codegraph_status(global_context).await;
    let json_string = serde_json::to_string_pretty(&status).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization problem: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json_string))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::State;
    use hyper::body::to_bytes;

    use super::*;

    async fn app_with_workspace_file(text: &str) -> (AppState, tempfile::TempDir, PathBuf) {
        let workspace = tempfile::tempdir().unwrap();
        let file_path = workspace.path().join("empty.rs");
        std::fs::write(&file_path, text).unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked: vec![],
            },
            loaded_ts: chrono::Utc::now().timestamp() as u64 + 60,
        });
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![file_path.clone()];

        (AppState::from_gcx(gcx).await, workspace, file_path)
    }

    fn ast_file_symbols_body(file_path: &PathBuf) -> hyper::body::Bytes {
        let body = serde_json::json!({
            "file_url": Url::from_file_path(file_path).unwrap(),
        });
        hyper::body::Bytes::from(serde_json::to_vec(&body).unwrap())
    }

    #[tokio::test]
    async fn ast_file_symbols_empty_doc_defs_stays_200() {
        let (app, _workspace, file_path) = app_with_workspace_file("").await;
        let codegraph = refact_codegraph::CodeGraphService::open_in_memory().unwrap();
        *app.gcx.codegraph.lock().await = Some(Arc::new(codegraph));

        let response = handle_v1_ast_file_symbols(State(app), ast_file_symbols_body(&file_path))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn ast_file_symbols_doc_defs_error_returns_500() {
        let (app, _workspace, file_path) = app_with_workspace_file("fn present() {}\n").await;
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("codegraph.sqlite");
        let codegraph = refact_codegraph::CodeGraphService::open(db_path.clone()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP TABLE nodes").unwrap();
        *app.gcx.codegraph.lock().await = Some(Arc::new(codegraph));

        let err =
            match handle_v1_ast_file_symbols(State(app), ast_file_symbols_body(&file_path)).await {
                Ok(response) => panic!("expected doc_defs error, got {}", response.status()),
                Err(err) => err,
            };

        assert_eq!(err.status_code, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
