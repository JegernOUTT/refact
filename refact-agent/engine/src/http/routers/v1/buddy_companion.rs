use axum::extract::{Path, State};
use hyper::StatusCode;

use crate::app_state::AppState;
use crate::buddy::companion::{get_or_create, CompanionResponse};
use crate::custom_error::ScratchError;

pub async fn handle_v1_buddy_companion(
    State(app): State<AppState>,
    Path(source_id): Path<String>,
) -> Result<axum::Json<CompanionResponse>, ScratchError> {
    get_or_create(app, &source_id)
        .await
        .map(axum::Json)
        .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))
}
