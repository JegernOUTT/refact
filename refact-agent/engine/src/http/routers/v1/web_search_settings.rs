use axum::{extract::State, Json};
use hyper::{body::Bytes, StatusCode};
use refact_at_web::web_search::{provider_catalog, test_providers};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    app_state::AppState,
    custom_error::ScratchError,
    web_search_settings::{load_web_search_settings, save_web_search_settings},
};

fn bad_request(message: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::BAD_REQUEST, message.into())
}

pub async fn get_settings(State(app): State<AppState>) -> Json<Value> {
    Json(load_web_search_settings(app.gcx).await.response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsPost {
    config: Value,
}

pub async fn post_settings(
    State(app): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, ScratchError> {
    let post: SettingsPost = serde_json::from_slice(&body)
        .map_err(|e| bad_request(format!("invalid settings payload: {e}")))?;
    Ok(Json(
        save_web_search_settings(app.gcx, post.config)
            .await?
            .response(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestPost {
    query: Option<String>,
    providers: Option<Vec<String>>,
}

pub async fn test_settings(
    State(app): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, ScratchError> {
    let post: TestPost = serde_json::from_slice(&body)
        .map_err(|e| bad_request(format!("invalid test payload: {e}")))?;
    if let Some(ids) = &post.providers {
        for id in ids {
            if !provider_catalog().iter().any(|p| p.id == id) {
                return Err(bad_request(format!("unknown provider: {id}")));
            }
        }
    }
    let query = post
        .query
        .unwrap_or_else(|| "rust tokio select cancellation safety".into());
    if query.trim().is_empty() {
        return Err(bad_request("query must not be empty"));
    }
    let config = load_web_search_settings(app.gcx).await.runtime_config();
    let reports = test_providers(&config, &query, post.providers.as_deref()).await;
    Ok(Json(json!({"query":query,"reports":reports})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn web_search_settings_routes_save_redact_reject_and_test_without_network() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let saved = post_settings(State(app.clone()),Bytes::from_static(br#"{"config":{"providers":{"brave":{"api_key":"route-secret"}},"deadline_ms":7000}}"#)).await.expect("save").0;
        assert_eq!(saved["config"]["providers"]["brave"]["api_key"], "***");
        assert!(!saved.to_string().contains("route-secret"));
        assert_eq!(
            get_settings(State(app.clone())).await.0["config"]["deadline_ms"],
            7000
        );
        for body in [
            br#"{"config":{"deadline_ms":0}}"#.as_slice(),
            br#"{"config":{"providers":{"unknown":{}}}}"#,
            br#"{"config":null}"#,
        ] {
            let error = post_settings(State(app.clone()), Bytes::copy_from_slice(body))
                .await
                .expect_err("reject");
            assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        }
        let error = test_settings(
            State(app.clone()),
            Bytes::from_static(br#"{"providers":["unknown"]}"#),
        )
        .await
        .expect_err("unknown provider");
        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        let tested = test_settings(State(app), Bytes::from_static(br#"{"providers":[]}"#))
            .await
            .expect("empty test")
            .0;
        assert_eq!(
            tested,
            json!({"query":"rust tokio select cancellation safety","reports":[]})
        );
    }
}
