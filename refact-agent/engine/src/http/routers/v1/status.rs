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
    #[serde(flatten)]
    pub legacy_vecdb: LegacyVecdbStatus,
    pub codegraph: Option<crate::codegraph::cg_status::CodeGraphStatus>,
    pub codegraph_alive: String,
    pub codegraph_error: String,
}

const LEGACY_AST_ALIVE: &str = "turned_off";

#[derive(Serialize, Deserialize, Debug)]
pub struct LegacyVecdbStatus {
    pub vecdb: Option<()>,
    pub vecdb_alive: String,
    pub vec_db_error: String,
}

impl Default for LegacyVecdbStatus {
    fn default() -> Self {
        Self {
            vecdb: None,
            vecdb_alive: "turned_off".into(),
            vec_db_error: String::new(),
        }
    }
}

pub async fn get_rag_status(gcx: SharedGlobalContext) -> RagStatus {
    let codegraph_status = crate::codegraph::cg_status::get_codegraph_status(gcx.clone()).await;
    let codegraph_error = codegraph_status.error.clone();
    let codegraph_message = codegraph_status.state.clone();

    RagStatus {
        ast: None,
        ast_alive: LEGACY_AST_ALIVE.to_string(),
        legacy_vecdb: LegacyVecdbStatus::default(),
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

    use super::*;

    #[test]
    fn rag_status_preserves_flat_legacy_client_fields() {
        let status = RagStatus {
            ast: None,
            ast_alive: LEGACY_AST_ALIVE.into(),
            legacy_vecdb: LegacyVecdbStatus::default(),
            codegraph: None,
            codegraph_alive: "turned_off".into(),
            codegraph_error: String::new(),
        };
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["vecdb"], serde_json::Value::Null);
        assert_eq!(json["vecdb_alive"], "turned_off");
        assert_eq!(json["vec_db_error"], "");
        assert!(json.get("legacy_vecdb").is_none());
        assert_eq!(json["codegraph_alive"], "turned_off");
    }
}
