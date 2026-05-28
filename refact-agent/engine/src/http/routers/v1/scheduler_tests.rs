use axum::routing::{delete, get};
use axum::Router;
use hyper::{Body, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::app_state::AppState;
use crate::http::routers::v1::scheduler::{
    handle_v1_scheduler_cron_delete, handle_v1_scheduler_cron_get, handle_v1_scheduler_cron_post,
};

async fn test_app() -> (tempfile::TempDir, Router) {
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
    let app_state = AppState::from_gcx(gcx).await;
    let router = Router::new()
        .route(
            "/scheduler/cron",
            get(handle_v1_scheduler_cron_get).post(handle_v1_scheduler_cron_post),
        )
        .route(
            "/scheduler/cron/:id",
            delete(handle_v1_scheduler_cron_delete),
        )
        .with_state(app_state);
    (temp, router)
}

async fn json_request(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn scheduler_cron_http_get_post_delete_happy_paths() {
    let (_temp, app) = test_app().await;

    let (status, created) = json_request(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "7 * * * *",
                    "prompt": "Check the frogs",
                    "recurring": true,
                    "durable": true,
                    "description": "Hourly frog check"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("cron_"));
    assert_eq!(created["human_schedule"], json!("hourly at :7"));
    assert_eq!(created["recurring"], json!(true));
    assert_eq!(created["durable"], json!(true));

    let (status, listed) = json_request(
        app.clone(),
        Request::builder()
            .method("GET")
            .uri("/scheduler/cron")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = listed.as_array().unwrap();
    let listed_task = list.iter().find(|task| task["id"] == json!(id)).unwrap();
    assert_eq!(listed_task["description"], json!("Hourly frog check"));
    assert_eq!(listed_task["prompt"], json!("Check the frogs"));
    assert_eq!(listed_task["fire_count"], json!(0));
    assert!(listed_task["next_fire_at_ms"].as_u64().unwrap() > 0);

    let (status, deleted) = json_request(
        app.clone(),
        Request::builder()
            .method("DELETE")
            .uri(format!("/scheduler/cron/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted, json!({ "removed": true }));

    let (status, listed) = json_request(
        app,
        Request::builder()
            .method("GET")
            .uri("/scheduler/cron")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!listed
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["id"] == json!(id)));
}
