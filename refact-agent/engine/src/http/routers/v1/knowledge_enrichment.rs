use crate::app_state::AppState;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::knowledge::enrichment::{model_gather_and_rewrite, EnrichmentItem, MAX_QUERY_LENGTH};

/// Request body for the manual memory enrichment preview endpoint.
#[derive(Deserialize)]
pub struct MemoryEnrichmentPreviewRequest {
    pub text: String,
}

/// Response shape for the wand-preview endpoint.
#[derive(Serialize)]
pub struct MemoryEnrichmentPreviewResponse {
    pub query_used: String,
    pub rewritten_text: String,
    pub items: Vec<EnrichmentItem>,
}

/// POST /v1/chats/:chat_id/memory-enrichment/preview
pub async fn handle_v1_memory_enrichment_preview(
    Path(_chat_id): Path<String>,
    State(app): State<AppState>,
    Json(payload): Json<MemoryEnrichmentPreviewRequest>,
) -> impl IntoResponse {
    let gcx = app.gcx.clone();
    let text = payload.text.trim().to_string();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail": "text must not be empty"})),
        )
            .into_response();
    }

    let query = if text.len() > MAX_QUERY_LENGTH {
        text.chars().take(MAX_QUERY_LENGTH).collect::<String>()
    } else {
        text.clone()
    };

    match model_gather_and_rewrite(gcx.clone(), &query).await {
        Ok((rewritten_text, items)) => {
            let resp = MemoryEnrichmentPreviewResponse {
                query_used: query,
                rewritten_text,
                items,
            };
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("memory enrichment preview failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"detail": e})),
            )
                .into_response()
        }
    }
}
