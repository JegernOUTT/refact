use axum::Extension;
use axum::extract::Query;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::stats::reader::{aggregate_summary, read_stats_events_from_dirs};

#[derive(Deserialize)]
pub struct StatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Deserialize)]
pub struct StatsEventsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub chat_id: Option<String>,
    pub success: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(serde::Serialize)]
struct EventsResponse<'a> {
    events: &'a [crate::stats::event::LlmCallEvent],
    total: usize,
    limit: usize,
    offset: usize,
}

pub async fn handle_v1_stats_llm_summary(
    Extension(gcx): Extension<SharedGlobalContext>,
    Query(params): Query<StatsQuery>,
) -> Result<Response<Body>, ScratchError> {
    let stats_dirs = crate::stats::get_stats_dirs_for_read(gcx).await;
    let from = params.from.as_deref();
    let to = params.to.as_deref();
    let events = read_stats_events_from_dirs(&stats_dirs, from, to);
    let summary = aggregate_summary(&events, from, to);
    let body = serde_json::to_string(&summary).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialization error: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

pub async fn handle_v1_stats_llm_events(
    Extension(gcx): Extension<SharedGlobalContext>,
    Query(params): Query<StatsEventsQuery>,
) -> Result<Response<Body>, ScratchError> {
    let stats_dirs = crate::stats::get_stats_dirs_for_read(gcx).await;
    let from = params.from.as_deref();
    let to = params.to.as_deref();
    let mut events = read_stats_events_from_dirs(&stats_dirs, from, to);

    if let Some(ref model_prefix) = params.model {
        events.retain(|e| e.model_id.starts_with(model_prefix.as_str()));
    }
    if let Some(ref provider) = params.provider {
        events.retain(|e| &e.provider == provider);
    }
    if let Some(ref chat_id) = params.chat_id {
        events.retain(|e| &e.chat_id == chat_id);
    }
    if let Some(success) = params.success {
        events.retain(|e| e.success == success);
    }

    let total = events.len();
    let limit = params.limit.unwrap_or(100).min(1000);
    let offset = params.offset.unwrap_or(0);

    let page: &[_] = if offset >= total {
        &[]
    } else {
        let end = (offset + limit).min(total);
        &events[offset..end]
    };

    let resp = EventsResponse {
        events: page,
        total,
        limit,
        offset,
    };
    let body = serde_json::to_string(&resp).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialization error: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}
