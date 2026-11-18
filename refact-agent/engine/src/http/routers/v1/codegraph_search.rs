use axum::response::Result;
use axum::extract::State;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;

#[derive(Serialize, Deserialize, Clone)]
struct CodeGraphSearchPost {
    query: String,
    top_n: usize,
}

#[derive(Serialize)]
struct CodeGraphSearchResult {
    query_text: String,
    results: Vec<refact_codegraph::CodeHit>,
}

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

pub async fn handle_v1_codegraph_search(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<CodeGraphSearchPost>(&body_bytes)
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
