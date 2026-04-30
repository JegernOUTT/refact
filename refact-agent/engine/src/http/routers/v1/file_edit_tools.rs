use std::sync::Arc;

use crate::call_validation::DiffChunk;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use axum::http::{Response, StatusCode};
use axum::Extension;
use hyper::Body;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock as ARwLock;

#[derive(Deserialize)]
pub struct FileEditDryRunPost {
    pub tool_name: String,
    pub tool_args: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct FileEditDryRunResponse {
    file_before: String,
    file_after: String,
    chunks: Vec<DiffChunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileEditResult>>,
}

#[derive(Serialize)]
pub struct FileEditResult {
    path: String,
    action: String,
    file_before: String,
    file_after: String,
    chunks: Vec<DiffChunk>,
}

fn build_response(resp: FileEditDryRunResponse) -> Result<Response<Body>, ScratchError> {
    let json = serde_json::to_string_pretty(&resp)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_v1_file_edit_tool_dry_run(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<FileEditDryRunPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    if post.tool_name == "apply_patch" {
        let result = crate::tools::file_edit::tool_apply_patch::tool_apply_patch_exec(
            global_context.clone(),
            &post.tool_args,
            true,
            None,
        )
        .await
        .map_err(|x| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, x))?;

        let files: Vec<FileEditResult> = result
            .file_results
            .iter()
            .map(|r| FileEditResult {
                path: r.path.to_string_lossy().to_string(),
                action: r.action.to_string(),
                file_before: r.before.clone(),
                file_after: r.after.clone(),
                chunks: r.chunks.clone(),
            })
            .collect();

        let (file_before, file_after) = if result.file_results.len() == 1 {
            let first = &result.file_results[0];
            (first.before.clone(), first.after.clone())
        } else {
            let before = result
                .file_results
                .iter()
                .map(|r| format!("=== {} ===\n{}", r.path.display(), r.before))
                .collect::<Vec<_>>()
                .join("\n");
            let after = result
                .file_results
                .iter()
                .map(|r| format!("=== {} ===\n{}", r.path.display(), r.after))
                .collect::<Vec<_>>()
                .join("\n");
            (before, after)
        };

        return build_response(FileEditDryRunResponse {
            file_before,
            file_after,
            chunks: result.all_chunks,
            files: Some(files),
        });
    }

    let (file_before, file_after, chunks, _) = match post.tool_name.as_str() {
        "create_textdoc" => {
            crate::tools::file_edit::tool_create_textdoc::tool_create_text_doc_exec(
                global_context.clone(), &post.tool_args, true, None,
            ).await.map_err(|x| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, x))?
        }
        "update_textdoc" => {
            crate::tools::file_edit::tool_update_textdoc::tool_update_text_doc_exec(
                global_context.clone(), &post.tool_args, true, None,
            ).await.map_err(|x| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, x))?
        }
        "update_textdoc_regex" => {
            crate::tools::file_edit::tool_update_textdoc_regex::tool_update_text_doc_regex_exec(
                global_context.clone(), &post.tool_args, true, None,
            ).await.map_err(|x| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, x))?
        }
        "update_textdoc_by_lines" => {
            crate::tools::file_edit::tool_update_textdoc_by_lines::tool_update_text_doc_by_lines_exec(
                global_context.clone(), &post.tool_args, true, None,
            ).await.map_err(|x| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, x))?
        }
        "update_textdoc_anchored" => {
            crate::tools::file_edit::tool_update_textdoc_anchored::tool_update_text_doc_anchored_exec(
                global_context.clone(), &post.tool_args, true, None,
            ).await.map_err(|x| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, x))?
        }
        _ => return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("Unknown tool: {}", post.tool_name))),
    };

    build_response(FileEditDryRunResponse {
        file_before,
        file_after,
        chunks,
        files: None,
    })
}
