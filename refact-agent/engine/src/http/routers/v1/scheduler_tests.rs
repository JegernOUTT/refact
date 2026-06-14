use std::sync::Arc;

use axum::routing::{delete, get};
use axum::Router;
use hyper::{Body, Request, StatusCode};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use tower::ServiceExt;

use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::http::routers::v1::scheduler::{
    handle_v1_scheduler_cron_delete, handle_v1_scheduler_cron_get, handle_v1_scheduler_cron_post,
};

async fn test_app() -> (tempfile::TempDir, AppState, Router) {
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
        .with_state(app_state.clone());
    (temp, app_state, router)
}

async fn add_open_session(app: &AppState, chat_id: &str) {
    let session = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
    app.gcx
        .chat_sessions
        .write()
        .await
        .insert(chat_id.to_string(), session);
}

async fn add_closed_session(app: &AppState, chat_id: &str) {
    let mut session = ChatSession::new(chat_id.to_string());
    session.close_event_channel();
    let session_arc = Arc::new(AMutex::new(session));
    app.gcx
        .chat_sessions
        .write()
        .await
        .insert(chat_id.to_string(), session_arc);
}

async fn json_request(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn scheduler_cron_http_get_post_delete_happy_paths() {
    let (_temp, app_state, app) = test_app().await;
    add_open_session(&app_state, "test-chat-1").await;

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
                    "description": "Hourly frog check",
                    "chat_id": "test-chat-1"
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

#[tokio::test]
async fn scheduler_create_rejects_missing_chat_id() {
    let (_temp, _app_state, app) = test_app().await;

    let (status, _) = json_request(
        app,
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "7 * * * *",
                    "prompt": "Check the frogs",
                    "description": "Hourly frog check",
                    "chat_id": ""
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn scheduler_create_rejects_closed_or_missing_chat() {
    let (_temp, app_state, app) = test_app().await;
    add_closed_session(&app_state, "closed-chat").await;

    let (status_missing, _) = json_request(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "7 * * * *",
                    "prompt": "Check the frogs",
                    "description": "Hourly frog check",
                    "chat_id": "nonexistent-chat"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status_missing, StatusCode::BAD_REQUEST);

    let (status_closed, _) = json_request(
        app,
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "7 * * * *",
                    "prompt": "Check the frogs",
                    "description": "Hourly frog check",
                    "chat_id": "closed-chat"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status_closed, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn scheduler_create_with_chat_id_creates_executable_task() {
    let (_temp, app_state, app) = test_app().await;
    add_open_session(&app_state, "active-chat").await;

    let (status, created) = json_request(
        app,
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "*/5 * * * *",
                    "prompt": "Run checks",
                    "description": "Check build",
                    "chat_id": "active-chat",
                    "mode": "agent"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap();
    assert!(id.starts_with("cron_"));

    let tasks = crate::scheduler::session_cron_store().list().await;
    let task = tasks.iter().find(|t| t.id == id).unwrap();
    assert_eq!(task.chat_id(), Some("active-chat"));
    assert_eq!(task.mode(), Some("agent"));
}
