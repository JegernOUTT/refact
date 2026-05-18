use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;

pub async fn handle_v1_get_app_searchable_id(
    State(app): State<AppState>,
    _body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(
            serde_json::to_string(
                &serde_json::json!({ "app_searchable_id": gcx.app_searchable_id }),
            )
            .unwrap(),
        ))
        .unwrap())
}
