use std::net::SocketAddr;
use axum::{
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use crate::{
    app_state::AppState,
    coordination::scope,
    organization::{service, types::*},
};

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.code {
            "invalid_argument" => StatusCode::BAD_REQUEST,
            "conflict" => StatusCode::CONFLICT,
            "scope_denied" => StatusCode::FORBIDDEN,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(self)).into_response()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaxonomyQuery {
    kind: TaxonomyKind,
    limit: Option<usize>,
    cursor: Option<String>,
}
async fn caller(
    app: &AppState,
    headers: &HeaderMap,
    peer: Option<ConnectInfo<SocketAddr>>,
) -> Result<scope::WorkCaller, Response> {
    super::work::authorize(app, headers, peer.map(|p| p.0)).map_err(IntoResponse::into_response)?;
    scope::operator_caller(app)
        .await
        .map_err(IntoResponse::into_response)
}
pub async fn taxonomy(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Query(q): Query<TaxonomyQuery>,
) -> Result<Json<TaxonomyResponse>, Response> {
    let caller = caller(&app, &headers, peer).await?;
    service::taxonomy(
        &app,
        &caller,
        q.kind,
        q.limit.unwrap_or(200),
        q.cursor.as_deref(),
    )
    .await
    .map(Json)
    .map_err(IntoResponse::into_response)
}
pub async fn items(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(request): Json<ItemsRequest>,
) -> Result<Json<ItemsResponse>, Response> {
    let caller = caller(&app, &headers, peer).await?;
    service::items(&app, &caller, request)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}
pub async fn apply(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(request): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, Response> {
    let caller = caller(&app, &headers, peer).await?;
    service::apply(&app, &caller, request)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}
