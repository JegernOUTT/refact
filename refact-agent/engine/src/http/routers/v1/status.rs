use axum::extract::State;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use refact_core::ast_types::AstStatus;
use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;

#[derive(Serialize, Deserialize, Debug)]
pub struct RagStatus {
    pub ast: Option<AstStatus>,
    pub ast_alive: String,
    pub vecdb: Option<crate::vecdb::vdb_structs::VecDbStatus>,
    pub vecdb_alive: String,
    pub vec_db_error: String,
    pub codegraph: Option<crate::codegraph::cg_status::CodeGraphStatus>,
    pub codegraph_alive: String,
    pub codegraph_error: String,
}

const LEGACY_AST_ALIVE: &str = "turned_off";

fn vecdb_rag_status_fields(
    vecdb_requested: bool,
    stored_vec_db_error: String,
    status_result: std::result::Result<Option<crate::vecdb::vdb_structs::VecDbStatus>, String>,
) -> (
    Option<crate::vecdb::vdb_structs::VecDbStatus>,
    String,
    String,
) {
    match status_result {
        Ok(Some(status)) => {
            let alive = if !stored_vec_db_error.is_empty() {
                "error"
            } else if status.state == "starting" {
                "starting"
            } else {
                "working"
            };
            (Some(status), alive.to_string(), stored_vec_db_error)
        }
        Ok(None) if !vecdb_requested => (None, "turned_off".to_string(), String::new()),
        Ok(None) if !stored_vec_db_error.is_empty() => {
            (None, "error".to_string(), stored_vec_db_error)
        }
        Ok(None) => (None, "starting".to_string(), stored_vec_db_error),
        Err(err) => {
            let vec_db_error = if stored_vec_db_error.is_empty() {
                err
            } else {
                stored_vec_db_error
            };
            (None, "error".to_string(), vec_db_error)
        }
    }
}

pub async fn get_rag_status(gcx: SharedGlobalContext) -> RagStatus {
    let vecdb_requested = gcx.cmdline.vecdb;
    let vec_db_module = gcx.vec_db.clone();
    let stored_vec_db_error = gcx.vec_db_error.lock().unwrap().clone();

    let (maybe_vecdb_status, vecdb_message, vec_db_error) = vecdb_rag_status_fields(
        vecdb_requested,
        stored_vec_db_error,
        crate::vecdb::vdb_highlev::get_status(vec_db_module).await,
    );

    let codegraph_status = crate::codegraph::cg_status::get_codegraph_status(gcx.clone()).await;
    let codegraph_error = codegraph_status.error.clone();
    let codegraph_message = codegraph_status.state.clone();

    RagStatus {
        ast: None,
        ast_alive: LEGACY_AST_ALIVE.to_string(),
        vecdb: maybe_vecdb_status,
        vecdb_alive: vecdb_message,
        vec_db_error,
        codegraph: Some(codegraph_status),
        codegraph_alive: codegraph_message,
        codegraph_error,
    }
}

pub async fn handle_v1_codegraph_status(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let status = crate::codegraph::cg_status::get_codegraph_status(gcx).await;

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

pub async fn handle_v1_file_index_status(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let status = app.gcx.file_index.status();

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

pub async fn handle_v1_rag_status(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let status = get_rag_status(gcx).await;

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

    use crate::vecdb::vdb_structs::VecDbStatus;
}
