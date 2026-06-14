use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;
use hyper::{Body, Request, StatusCode};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use tower::ServiceExt;

use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::http::routers::v1::scheduler::{
    handle_v1_scheduler_cron_delete, handle_v1_scheduler_cron_get, handle_v1_scheduler_cron_patch,
    handle_v1_scheduler_cron_post, handle_v1_scheduler_cron_run,
};
use crate::scheduler::{Action, AgentTarget, CronRunRecord, Delivery, Job, Trigger};

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
            delete(handle_v1_scheduler_cron_delete).patch(handle_v1_scheduler_cron_patch),
        )
        .route(
            "/scheduler/cron/:id/run",
            post(handle_v1_scheduler_cron_run),
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

fn interval_job(id: &str, every_ms: u64) -> Job {
    Job {
        id: id.to_string(),
        description: "Interval frog check".to_string(),
        enabled: true,
        durable: false,
        created_at_ms: 1_000,
        recurring: true,
        trigger: Trigger::Interval { every_ms },
        action: Action::AgentTurn {
            prompt: "Check interval frogs".to_string(),
            target: AgentTarget::ExistingChat {
                chat_id: "active-chat".to_string(),
            },
            mode: Some("agent".to_string()),
            model: None,
            tools: None,
        },
        delivery: Delivery::Chat,
        last_fired_at_ms: Some(2_000),
        fire_count: 2,
        last_status: Some("fired".to_string()),
        last_error: None,
        recent_runs: vec![CronRunRecord {
            at_ms: 2_000,
            status: "fired".to_string(),
            error: None,
        }],
        paused_at_ms: None,
        trigger_at_ms: None,
        auto_expire_after_ms: crate::scheduler::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
    }
}

fn paused_job(id: &str) -> Job {
    let mut job = interval_job(id, 10 * 60_000);
    job.description = "Paused frog check".to_string();
    job.enabled = false;
    job.paused_at_ms = Some(3_000);
    job.last_status = Some("deferred".to_string());
    job.last_error = Some("busy".to_string());
    job.recent_runs = vec![CronRunRecord {
        at_ms: 3_000,
        status: "deferred".to_string(),
        error: Some("busy".to_string()),
    }];
    job
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
    assert_eq!(listed_task["action_kind"], json!("agent_turn"));
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

#[tokio::test]
async fn scheduler_cron_http_post_command_creates_command_job() {
    let (_temp, app_state, app) = test_app().await;
    add_open_session(&app_state, "command-chat").await;

    let (status, created) = json_request(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "*/5 * * * *",
                    "command": "printf 'hi frog'",
                    "description": "Print frog",
                    "chat_id": "command-chat",
                    "cwd": ".",
                    "timeout_secs": 9
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["action_kind"], json!("command"));
    let id = created["id"].as_str().unwrap();

    let tasks = crate::scheduler::session_cron_store().list().await;
    let task = tasks.iter().find(|task| task.id == id).unwrap();
    match &task.action {
        Action::Command {
            argv,
            target,
            cwd,
            timeout_secs,
            ..
        } => {
            assert_eq!(argv, &vec!["printf".to_string(), "hi frog".to_string()]);
            assert_eq!(
                target,
                &AgentTarget::ExistingChat {
                    chat_id: "command-chat".to_string()
                }
            );
            assert_eq!(cwd.as_deref(), Some("."));
            assert_eq!(*timeout_secs, Some(9));
        }
        _ => panic!("expected command action"),
    }

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
    let listed_task = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == json!(id))
        .unwrap();
    assert_eq!(listed_task["action_kind"], json!("command"));
}

#[tokio::test]
async fn scheduler_cron_http_post_webhook_delivery_round_trips() {
    let (_temp, _app_state, app) = test_app().await;

    let (status, created) = json_request(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "*/5 * * * *",
                    "command": "printf 'hi frog'",
                    "description": "Print frog",
                    "delivery": {"kind": "webhook", "url": "http://127.0.0.1/hook", "token": "secret"}
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["action_kind"], json!("command"));
    assert_eq!(created["delivery"]["kind"], json!("webhook"));
    assert_eq!(created["delivery"]["url"], json!("http://127.0.0.1/hook"));
    assert_eq!(created["delivery"]["has_token"], json!(true));
    assert_eq!(created["delivery"].get("token"), None);
    let id = created["id"].as_str().unwrap();

    let tasks = crate::scheduler::session_cron_store().list().await;
    let task = tasks.iter().find(|task| task.id == id).unwrap();
    assert_eq!(
        task.delivery,
        Delivery::Webhook {
            url: "http://127.0.0.1/hook".to_string(),
            token: Some("secret".to_string()),
        }
    );

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
    let listed_task = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == json!(id))
        .unwrap();
    assert_eq!(listed_task["delivery"]["kind"], json!("webhook"));
    assert_eq!(
        listed_task["delivery"]["url"],
        json!("http://127.0.0.1/hook")
    );
    assert_eq!(listed_task["delivery"]["has_token"], json!(true));
    assert_eq!(listed_task["delivery"].get("token"), None);
}

#[tokio::test]
async fn scheduler_cron_http_post_rejects_prompt_plus_command() {
    let (_temp, app_state, app) = test_app().await;
    add_open_session(&app_state, "command-chat").await;

    let (status, _) = json_request(
        app,
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "*/5 * * * *",
                    "prompt": "Check frogs",
                    "command": "printf hi",
                    "description": "Bad frogs",
                    "chat_id": "command-chat"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn scheduler_cron_http_get_returns_interval_and_paused_fields() {
    let (_temp, _app_state, app) = test_app().await;
    let interval_id = format!("cron_http_interval_{}", uuid::Uuid::now_v7());
    let paused_id = format!("cron_http_paused_{}", uuid::Uuid::now_v7());
    let store = crate::scheduler::session_cron_store();
    store
        .add(interval_job(&interval_id, 30 * 60_000))
        .await
        .unwrap();
    store.add(paused_job(&paused_id)).await.unwrap();

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
    let list = listed.as_array().unwrap();
    let interval = list
        .iter()
        .find(|task| task["id"] == json!(interval_id))
        .unwrap();
    assert_eq!(interval["cron"], json!(""));
    assert_eq!(interval["trigger_kind"], json!("interval"));
    assert_eq!(interval["every_ms"], json!(30 * 60_000));
    assert_eq!(interval["at_ms"], Value::Null);
    assert_eq!(interval["tz"], Value::Null);
    assert_eq!(interval["enabled"], json!(true));
    assert_eq!(interval["paused"], json!(false));
    assert_eq!(interval["last_status"], json!("fired"));
    assert_eq!(interval["last_error"], Value::Null);
    assert_eq!(interval["recent_runs"][0]["status"], json!("fired"));

    let paused = list
        .iter()
        .find(|task| task["id"] == json!(paused_id))
        .unwrap();
    assert_eq!(paused["enabled"], json!(false));
    assert_eq!(paused["paused"], json!(true));
    assert_eq!(paused["last_status"], json!("deferred"));
    assert_eq!(paused["last_error"], json!("busy"));
    assert_eq!(paused["recent_runs"][0]["error"], json!("busy"));
}

#[tokio::test]
async fn scheduler_cron_http_post_every_creates_interval_job() {
    let (_temp, app_state, app) = test_app().await;
    add_open_session(&app_state, "interval-chat").await;

    let (status, created) = json_request(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "every": "30m",
                    "prompt": "Check interval frogs",
                    "description": "Interval frog check",
                    "chat_id": "interval-chat"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["human_schedule"], json!("every 30m"));

    let id = created["id"].as_str().unwrap();
    let tasks = crate::scheduler::session_cron_store().list().await;
    let task = tasks.iter().find(|task| task.id == id).unwrap();
    assert_eq!(
        task.trigger,
        Trigger::Interval {
            every_ms: 30 * 60_000
        }
    );
    assert_eq!(task.chat_id(), Some("interval-chat"));

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
    let listed_task = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == json!(id))
        .unwrap();
    assert_eq!(listed_task["trigger_kind"], json!("interval"));
    assert_eq!(listed_task["every_ms"], json!(30 * 60_000));
}

#[tokio::test]
async fn scheduler_cron_http_patch_pauses_resumes_and_changes_schedule() {
    let (_temp, app_state, app) = test_app().await;
    add_open_session(&app_state, "patch-chat").await;

    let (status, created) = json_request(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/scheduler/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "cron": "*/15 * * * *",
                    "prompt": "Patch frogs",
                    "description": "Patch frog check",
                    "chat_id": "patch-chat"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap().to_string();

    let (status, paused) = json_request(
        app.clone(),
        Request::builder()
            .method("PATCH")
            .uri(format!("/scheduler/cron/{id}"))
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "enabled": false }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        paused,
        json!({ "id": id.clone(), "updated": true, "human_schedule": "every 15 minutes" })
    );
    let stored = crate::scheduler::session_cron_store()
        .get(&id)
        .await
        .unwrap();
    assert!(!stored.enabled);
    assert!(stored.paused_at_ms.is_some());

    let (status, updated) = json_request(
        app.clone(),
        Request::builder()
            .method("PATCH")
            .uri(format!("/scheduler/cron/{id}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({ "enabled": true, "every": "45m", "description": "Updated patch frogs" })
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["human_schedule"], json!("every 45m"));
    let stored = crate::scheduler::session_cron_store()
        .get(&id)
        .await
        .unwrap();
    assert!(stored.enabled);
    assert_eq!(stored.paused_at_ms, None);
    assert_eq!(
        stored.trigger,
        Trigger::Interval {
            every_ms: 45 * 60_000
        }
    );
    assert_eq!(stored.description, "Updated patch frogs");
}

#[tokio::test]
async fn scheduler_cron_http_run_sets_trigger() {
    let (_temp, _app_state, app) = test_app().await;
    let id = format!("cron_http_run_{}", uuid::Uuid::now_v7());
    let store = crate::scheduler::session_cron_store();
    store.add(interval_job(&id, 30 * 60_000)).await.unwrap();

    let before_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let (status, triggered) = json_request(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("/scheduler/cron/{id}/run"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triggered, json!({ "id": id.clone(), "triggered": true }));
    let stored = store.get(&id).await.unwrap();
    assert!(stored.trigger_at_ms.unwrap() >= before_ms);
}

#[tokio::test]
async fn scheduler_cron_http_bad_patch_unknown_id_returns_4xx() {
    let (_temp, _app_state, app) = test_app().await;

    let (status, _) = json_request(
        app,
        Request::builder()
            .method("PATCH")
            .uri("/scheduler/cron/cron_missing_for_patch")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "enabled": false }).to_string()))
            .unwrap(),
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}
