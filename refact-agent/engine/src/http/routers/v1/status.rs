use axum::response::Result;
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
}

pub async fn get_rag_status(gcx: SharedGlobalContext) -> RagStatus {
    let (vec_db_module, vec_db_error) =
        { (gcx.vec_db.clone(), gcx.vec_db_error.lock().unwrap().clone()) };

    let (maybe_vecdb_status, vecdb_message) =
        match crate::vecdb::vdb_highlev::get_status(vec_db_module).await {
            Ok(Some(status)) => (Some(status), "working".to_string()),
            Ok(None) => (None, "turned_off".to_string()),
            Err(err) => (None, err.to_string()),
        };

    let (maybe_codegraph_status, codegraph_message) =
        match crate::codegraph::cg_status::get_codegraph_status(gcx.clone()).await {
            Some(status) => (Some(status), "working".to_string()),
            None => (None, "turned_off".to_string()),
        };

    RagStatus {
        ast: None,
        ast_alive: "turned_off".to_string(),
        vecdb: maybe_vecdb_status,
        vecdb_alive: vecdb_message,
        vec_db_error,
        codegraph: maybe_codegraph_status,
        codegraph_alive: codegraph_message,
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
