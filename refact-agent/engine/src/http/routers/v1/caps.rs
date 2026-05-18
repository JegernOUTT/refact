use axum::extract::State;
use axum::extract::Query;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::caps::model_caps;
use crate::app_state::AppState;
use crate::custom_error::ScratchError;

pub async fn handle_v1_ping(
    State(app): State<AppState>,
) -> Response<Body> {
    let gcx = app.gcx.clone();
    let ping_message: String = gcx.cmdline.ping_message.clone();
    Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(ping_message + "\n"))
        .unwrap()
}

pub async fn handle_v1_caps(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let caps_result =
        crate::global_context::try_load_caps_quickly_if_not_present(global_context.clone(), 0)
            .await;
    let caps_arc = match caps_result {
        Ok(x) => x,
        Err(e) => {
            return Err(ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("{}", e),
            ));
        }
    };
    let body = serde_json::to_string_pretty(&*caps_arc).unwrap();
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap();
    Ok(response)
}

#[derive(Deserialize)]
pub struct ModelCapsQuery {
    #[serde(default)]
    pub refresh: bool,
    pub model: Option<String>,
}

pub async fn handle_v1_model_capabilities(
    State(app): State<AppState>,
    Query(query): Query<ModelCapsQuery>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let caps = model_caps::get_model_caps(gcx.clone(), query.refresh)
        .await
        .map_err(|e| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;

    if query.refresh {
        let caps_state = gcx.caps_state.clone();
        let mut caps_state = caps_state.write().await;
        caps_state.caps = None;
        caps_state.last_attempted_ts = 0;
    }

    let body = if let Some(model_name) = query.model {
        match model_caps::resolve_model_caps(&caps, &model_name) {
            Some(resolved) => serde_json::to_string_pretty(&resolved.caps).unwrap(),
            None => {
                return Err(ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("Model '{}' not found in capabilities registry", model_name),
                ));
            }
        }
    } else {
        serde_json::to_string_pretty(&caps).unwrap()
    };

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap();
    Ok(response)
}

pub async fn handle_v1_model_supported(
    State(app): State<AppState>,
    Query(query): Query<ModelCapsQuery>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let model_name = query.model.ok_or_else(|| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Missing 'model' query parameter".to_string(),
        )
    })?;

    let caps = model_caps::get_model_caps(gcx, false)
        .await
        .map_err(|e| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;

    let supported = model_caps::is_model_supported(&caps, &model_name);
    let body = serde_json::json!({
        "model": model_name,
        "supported": supported
    });

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string_pretty(&body).unwrap()))
        .unwrap();
    Ok(response)
}
