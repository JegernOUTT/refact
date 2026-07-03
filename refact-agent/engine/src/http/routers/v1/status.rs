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
    pub codegraph_error: String,
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

    let codegraph_status = crate::codegraph::cg_status::get_codegraph_status(gcx.clone()).await;
    let codegraph_error = codegraph_status.error.clone();
    let codegraph_message = codegraph_status.state.clone();

    RagStatus {
        ast: None,
        ast_alive: "turned_off".to_string(),
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
    use std::sync::Arc;

    use super::*;
    use crate::codegraph::CodeGraphService;

    #[tokio::test]
    async fn rag_status_codegraph_state_tracks_queue_and_errors() {
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let status = get_rag_status(gcx.clone()).await;
        assert_eq!(status.codegraph_alive, "turned_off");
        let codegraph = status.codegraph.expect("codegraph status must be present");
        assert_eq!(codegraph.state, "turned_off");
        assert_eq!(codegraph.queued, 0);
        assert_eq!(codegraph.cross_file_edges, 0);
        assert!(!codegraph.cross_file_ready);
        assert_eq!(codegraph.throughput_files_per_min, 0.0);
        assert_eq!(codegraph.eta_seconds, None);
        assert_eq!(codegraph.error, "");
        assert_eq!(status.codegraph_error, "");

        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        service.enqueue_files(&["src/main.rs".to_string()]);
        *gcx.codegraph.lock().await = Some(service.clone());

        let status = get_rag_status(gcx.clone()).await;
        assert_eq!(status.codegraph_alive, "indexing");
        let codegraph = status.codegraph.expect("codegraph status must be present");
        assert_eq!(codegraph.state, "indexing");
        assert_eq!(codegraph.queued, 1);
        assert!(!codegraph.cross_file_ready);
        assert_eq!(codegraph.throughput_files_per_min, 0.0);
        assert_eq!(codegraph.eta_seconds, None);
        assert_eq!(codegraph.error, "");

        service.drain_batch(10);
        service
            .index_file(
                "src/main.rs",
                "fn main() { helper(); }\nfn helper() {}",
                "rust",
            )
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        service.mark_initial_index_done();

        let status = get_rag_status(gcx.clone()).await;
        assert_eq!(status.codegraph_alive, "working");
        let codegraph = status.codegraph.expect("codegraph status must be present");
        assert_eq!(codegraph.state, "working");
        assert_eq!(codegraph.queued, 0);
        assert_eq!(codegraph.throughput_files_per_min, 0.0);
        assert_eq!(codegraph.eta_seconds, None);
        assert_eq!(codegraph.counts.files, 1);
        assert_eq!(codegraph.cross_file_edges, 0);
        assert!(codegraph.cross_file_ready);
        assert_eq!(codegraph.error, "");

        *gcx.codegraph_error.lock().unwrap() = "store unavailable".to_string();

        let status = get_rag_status(gcx.clone()).await;
        assert_eq!(status.codegraph_alive, "error");
        assert_eq!(status.codegraph_error, "store unavailable");
        let codegraph = status.codegraph.expect("codegraph status must be present");
        assert_eq!(codegraph.state, "error");
        assert_eq!(codegraph.error, "store unavailable");

        *gcx.codegraph_error.lock().unwrap() = String::new();

        let status = get_rag_status(gcx.clone()).await;
        assert_eq!(status.codegraph_alive, "working");
        assert_eq!(status.codegraph_error, "");
        let codegraph = status.codegraph.expect("codegraph status must be present");
        assert_eq!(codegraph.state, "working");
        assert_eq!(codegraph.error, "");
    }

    #[tokio::test]
    async fn rag_status_codegraph_open_error_surfaces_without_service() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.codegraph_error.lock().unwrap() = "open failed".to_string();

        let status = get_rag_status(gcx.clone()).await;

        assert_eq!(status.codegraph_alive, "error");
        assert_eq!(status.codegraph_error, "open failed");
        let codegraph = status.codegraph.expect("error status must be present");
        assert_eq!(codegraph.state, "error");
        assert_eq!(codegraph.error, "open failed");
        assert_eq!(codegraph.queued, 0);
        assert_eq!(codegraph.cross_file_edges, 0);
        assert!(!codegraph.cross_file_ready);
        assert_eq!(codegraph.throughput_files_per_min, 0.0);
        assert_eq!(codegraph.eta_seconds, None);
        assert_eq!(codegraph.counts, refact_codegraph::Counts::default());
    }

    #[tokio::test]
    async fn throughput_and_eta_fields_present() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        service.enqueue_files(&[
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
        ]);
        service.record_index_completions(6);
        *gcx.codegraph.lock().await = Some(service);

        let status = get_rag_status(gcx).await;
        let codegraph = status.codegraph.expect("codegraph status must be present");

        assert_eq!(codegraph.queued, 3);
        assert!(codegraph.throughput_files_per_min > 0.0);
        assert!(codegraph.eta_seconds.is_some());
        let json = serde_json::to_value(&codegraph).unwrap();
        assert!(json.get("throughput_files_per_min").is_some());
        assert!(json.get("eta_seconds").is_some());
    }
}
