use axum::response::Result;
use axum::extract::State;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;

#[derive(Serialize, Deserialize, Clone)]
struct VecDBPost {
    query: String,
    top_n: usize,
}

#[derive(Serialize)]
struct CodeGraphSearchResult {
    query_text: String,
    results: Vec<refact_codegraph::CodeHit>,
}

const NO_VECDB: &str = "Vector db is not running, check if you have --vecdb parameter and a vectorization model is running on server side.";
const NO_CODEGRAPH: &str =
    "Codegraph is not running, check if project indexing is enabled and finished.";

fn json_response<T: Serialize>(value: &T) -> Result<Response<Body>, ScratchError> {
    let json_string = serde_json::to_string_pretty(value).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization problem: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json_string))
        .unwrap())
}

pub async fn handle_v1_vecdb_search(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post = serde_json::from_slice::<VecDBPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;

    let db = match gcx.vec_db.lock().await.clone() {
        Some(db) => db,
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                NO_VECDB.to_string(),
            ));
        }
    };
    let search_res = db
        .vecdb_search(post.query.to_string(), post.top_n, None)
        .await;

    match search_res {
        Ok(search_res) => json_response(&search_res),
        Err(e) => Err(ScratchError::new(StatusCode::BAD_REQUEST, e)),
    }
}

pub async fn handle_v1_codegraph_search(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<VecDBPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;

    let service = match app.gcx.codegraph.lock().await.clone() {
        Some(service) => service,
        None => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                NO_CODEGRAPH.to_string(),
            ));
        }
    };
    let hits = service
        .search_hybrid(&post.query, post.top_n)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    json_response(&CodeGraphSearchResult {
        query_text: post.query,
        results: hits,
    })
}

pub async fn handle_v1_vecdb_status(
    State(app): State<AppState>,
    _: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let vec_db = gcx.vec_db.clone();
    let status_str = match crate::vecdb::vdb_highlev::get_status(vec_db).await {
        Ok(Some(status)) => serde_json::to_string_pretty(&status).unwrap(),
        Ok(None) => "{\"success\": 0, \"detail\": \"turned_off\"}".to_string(),
        Err(err) => {
            return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, err));
        }
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(status_str))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use super::*;

    struct RecordingVecdb {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl crate::vecdb::vdb_structs::VecdbSearch for RecordingVecdb {
        async fn vecdb_search(
            &self,
            query: String,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<crate::vecdb::vdb_structs::SearchResult, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(crate::vecdb::vdb_structs::SearchResult {
                query_text: query,
                results: vec![crate::vecdb::vdb_structs::VecdbRecord {
                    vector: None,
                    file_path: PathBuf::from("memory.md"),
                    start_line: 1,
                    end_line: 2,
                    distance: 0.25,
                    usefulness: 75.0,
                }],
            })
        }

        async fn get_status(&self) -> Result<crate::vecdb::vdb_structs::VecDbStatus, String> {
            Ok(crate::vecdb::vdb_structs::VecDbStatus {
                files_unprocessed: 0,
                files_total: 1,
                requests_made_since_start: 1,
                vectors_made_since_start: 1,
                db_size: 0,
                db_cache_size: 0,
                state: "done".to_string(),
                queue_additions: false,
                vecdb_max_files_hit: false,
                vecdb_errors: Default::default(),
                cache_writes_failed: 0,
                vectors_missing_from_cache: 0,
            })
        }

        async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
            Ok(())
        }

        async fn vectorizer_enqueue_files(
            &self,
            _documents: &[String],
            _process_immediately: bool,
            _roots: refact_core::memory_plane::MemoryPlaneRoots,
        ) {
        }

        fn current_constants(&self) -> (crate::vecdb::vdb_structs::EmbeddingModelConfig, usize) {
            (
                crate::vecdb::vdb_structs::EmbeddingModelConfig {
                    model_id: String::new(),
                    endpoint: String::new(),
                    endpoint_style: String::new(),
                    embedding_endpoint_style: String::new(),
                    api_key: String::new(),
                    model_name: String::new(),
                    embedding_size: 0,
                    dimensions: None,
                    query_prefix: String::new(),
                    document_prefix: String::new(),
                    rejection_threshold: 0.0,
                    embedding_batch: 1,
                    n_ctx: 0,
                },
                0,
            )
        }

        async fn embed_query(&self, _query: &str) -> Result<Vec<f32>, String> {
            Ok(Vec::new())
        }

        async fn vecdb_search_with_embedding(
            &self,
            _embedding: &Vec<f32>,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<Vec<crate::vecdb::vdb_structs::VecdbRecord>, String> {
            Ok(Vec::new())
        }
    }
}
