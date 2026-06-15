use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::custom_error::ScratchError;
use crate::scheduler::{
    active_durable_cron_store, delivery_from_value, session_cron_store, Action, AgentTarget,
    CronRunner, CronStore, Delivery, Job, Trigger,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookFireKind {
    Wake,
    Agent,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HookFireRequest {
    pub kind: HookFireKind,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub hook_id: Option<String>,
    #[serde(default)]
    pub deliver: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct HookFireResponse {
    pub inline: Option<InlineFireResponse>,
    pub hook_jobs: HookJobsFireResponse,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InlineFireResponse {
    Wake { chat_id: String },
    Agent { job_id: String, fired: bool },
}

#[derive(Debug, Serialize)]
pub struct HookJobsFireResponse {
    pub hook_id: Option<String>,
    pub matched: usize,
    pub fired: usize,
    pub jobs: Vec<HookJobFireResult>,
}

#[derive(Debug, Serialize)]
pub struct HookJobFireResult {
    pub id: String,
    pub fired: bool,
}

pub async fn handle_v1_hooks_fire(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<HookFireRequest>,
) -> Result<Json<HookFireResponse>, ScratchError> {
    authorize_worker_hook(&app, &headers)?;
    let now_ms = unix_now_ms();
    let inline = fire_inline(&app, &request, now_ms).await?;
    let hook_id = normalized(request.hook_id.as_deref());
    if inline.is_none() && hook_id.is_none() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "message or hook_id is required".to_string(),
        ));
    }
    let hook_jobs = match hook_id {
        Some(hook_id) => fire_hook_jobs(&app, &hook_id, now_ms).await?,
        None => HookJobsFireResponse {
            hook_id: None,
            matched: 0,
            fired: 0,
            jobs: Vec::new(),
        },
    };
    Ok(Json(HookFireResponse { inline, hook_jobs }))
}

fn authorize_worker_hook(app: &AppState, headers: &HeaderMap) -> Result<(), ScratchError> {
    let Some(expected) = app.gcx.cmdline.daemon_auth_token.as_deref() else {
        return Ok(());
    };
    let actual = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().strip_prefix("Bearer "));
    if actual == Some(expected) {
        return Ok(());
    }
    Err(ScratchError::new(
        StatusCode::UNAUTHORIZED,
        "daemon authorization required".to_string(),
    ))
}

async fn fire_inline(
    app: &AppState,
    request: &HookFireRequest,
    now_ms: u64,
) -> Result<Option<InlineFireResponse>, ScratchError> {
    match request.kind {
        HookFireKind::Wake => {
            let Some(text) = request_text(request) else {
                return Ok(None);
            };
            let chat_id = inject_wake(app, &text).await;
            Ok(Some(InlineFireResponse::Wake { chat_id }))
        }
        HookFireKind::Agent => {
            let Some(message) = request_message(request) else {
                return Ok(None);
            };
            let delivery = request
                .deliver
                .as_ref()
                .map(delivery_from_value)
                .transpose()
                .map_err(|error| ScratchError::new(StatusCode::BAD_REQUEST, error))?
                .unwrap_or(Delivery::Chat);
            if !matches!(delivery, Delivery::Chat) {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "agent hook delivery must be chat".to_string(),
                ));
            }
            let job = inline_agent_job(
                message,
                request
                    .mode
                    .clone()
                    .and_then(|mode| normalized(Some(&mode))),
                request
                    .model
                    .clone()
                    .and_then(|model| normalized(Some(&model))),
                now_ms,
            );
            let job_id = job.id.clone();
            let fired = CronRunner::fire_manual_job(app.gcx.clone(), job, now_ms)
                .await
                .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
            Ok(Some(InlineFireResponse::Agent { job_id, fired }))
        }
    }
}

async fn inject_wake(app: &AppState, text: &str) -> String {
    let chat_id = wake_chat_id(app).await;
    let session_arc =
        get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, &chat_id).await;
    let mut session = session_arc.lock().await;
    session.add_message(event(
        EventSubkind::SystemNotice,
        "daemon.hooks",
        json!({
            "kind": "wake",
        }),
        text.to_string(),
    ));
    chat_id
}

async fn wake_chat_id(app: &AppState) -> String {
    let sessions = app.chat.sessions.read().await;
    let mut ids = sessions.keys().cloned().collect::<Vec<_>>();
    ids.sort();
    ids.into_iter().next().unwrap_or_else(|| {
        let project_id = app.gcx.cmdline.project_id.trim();
        if project_id.is_empty() {
            "project-main".to_string()
        } else {
            format!("project-{project_id}-main")
        }
    })
}

fn inline_agent_job(
    message: String,
    mode: Option<String>,
    model: Option<String>,
    now_ms: u64,
) -> Job {
    let mut job = Job::new_cron_agent_chat(
        String::new(),
        message,
        "Inbound hook agent".to_string(),
        false,
        false,
        now_ms,
    );
    job.id = format!("hook_inline_{}", Uuid::now_v7());
    job.trigger = Trigger::Manual;
    job.action = Action::AgentTurn {
        prompt: job.prompt().unwrap_or_default().to_string(),
        target: AgentTarget::Isolated,
        mode,
        model,
        tools: None,
    };
    job.auto_expire_after_ms = 0;
    job
}

async fn fire_hook_jobs(
    app: &AppState,
    hook_id: &str,
    now_ms: u64,
) -> Result<HookJobsFireResponse, ScratchError> {
    let mut matches = Vec::<(std::sync::Arc<dyn CronStore>, Job)>::new();
    let session_store = session_cron_store();
    matches.extend(
        session_store
            .jobs_by_hook_id(hook_id)
            .await
            .into_iter()
            .map(|job| (session_store.clone(), job)),
    );
    let durable = active_durable_cron_store(app.gcx.clone())
        .await
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    if let Some(store) = durable {
        matches.extend(
            store
                .jobs_by_hook_id(hook_id)
                .await
                .into_iter()
                .map(|job| (store.clone(), job)),
        );
    }

    let matched = matches.len();
    let mut jobs = Vec::with_capacity(matched);
    for (store, job) in matches {
        let fired = CronRunner::fire_store_job_now(store, app.gcx.clone(), &job.id, now_ms)
            .await
            .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        jobs.push(HookJobFireResult { id: job.id, fired });
    }
    let fired = jobs.iter().filter(|job| job.fired).count();
    Ok(HookJobsFireResponse {
        hook_id: Some(hook_id.to_string()),
        matched,
        fired,
        jobs,
    })
}

fn request_text(request: &HookFireRequest) -> Option<String> {
    normalized(request.text.as_deref()).or_else(|| normalized(request.message.as_deref()))
}

fn request_message(request: &HookFireRequest) -> Option<String> {
    normalized(request.message.as_deref()).or_else(|| normalized(request.text.as_deref()))
}

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::routing::post;
    use hyper::{Body, Request};
    use tokio::sync::Mutex as AMutex;
    use tower::ServiceExt;

    use super::*;
    use crate::chat::types::{ChatCommand, ChatSession};
    use crate::scheduler::InMemoryCronStore;

    async fn test_app() -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        AppState::from_gcx(gcx).await
    }

    fn router(app: AppState) -> axum::Router {
        axum::Router::new()
            .route("/hooks/fire", post(handle_v1_hooks_fire))
            .with_state(app)
    }

    async fn request_json(
        router: axum::Router,
        body: Value,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/hooks/fire")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        let response = router
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn add_session(app: &AppState, chat_id: &str) {
        app.gcx.chat_sessions.write().await.insert(
            chat_id.to_string(),
            Arc::new(AMutex::new(ChatSession::new(chat_id.to_string()))),
        );
    }

    #[cfg(not(target_os = "windows"))]
    fn stdout_command() -> Vec<String> {
        vec!["printf".to_string(), "hi".to_string()]
    }

    #[cfg(target_os = "windows")]
    fn stdout_command() -> Vec<String> {
        vec!["cmd".to_string(), "/C".to_string(), "echo hi".to_string()]
    }

    #[tokio::test]
    async fn hooks_fire_auth_requires_daemon_bearer_when_configured() {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cmdline.daemon_auth_token = Some("secret".to_string());
        let app = AppState::from_gcx(gcx).await;
        let router = router(app);

        let (status, _) =
            request_json(router.clone(), json!({"kind":"wake","text":"hello"}), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, value) = request_json(
            router,
            json!({"kind":"wake","text":"hello"}),
            Some("secret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["inline"]["kind"], json!("wake"));
    }

    #[tokio::test]
    async fn hooks_fire_wake_injects_system_notice() {
        let app = test_app().await;
        add_session(&app, "chat-existing").await;
        let router = router(app.clone());

        let (status, value) =
            request_json(router, json!({"kind":"wake","text":"wake up"}), None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["inline"]["chat_id"], json!("chat-existing"));
        let session_arc = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("chat-existing")
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        let message = session.messages.last().unwrap();
        assert_eq!(message.role, "event");
        assert_eq!(message.extra["event"]["subkind"], json!("system_notice"));
        assert_eq!(message.content.content_text_only(), "wake up");
    }

    #[tokio::test]
    async fn hooks_fire_agent_creates_isolated_turn() {
        let app = test_app().await;
        let router = router(app.clone());

        let (status, value) = request_json(
            router,
            json!({"kind":"agent","message":"ship it","mode":"agent","model":"model-1"}),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["inline"]["kind"], json!("agent"));
        let job_id = value["inline"]["job_id"].as_str().unwrap();
        let prefix = format!("cron_{job_id}_");
        let sessions = app.gcx.chat_sessions.read().await;
        let chat_id = sessions
            .keys()
            .find(|chat_id| chat_id.starts_with(&prefix))
            .cloned()
            .unwrap();
        let session_arc = sessions.get(&chat_id).cloned().unwrap();
        drop(sessions);
        let session = session_arc.lock().await;
        let queued = session.command_queue.iter().any(|request| {
            matches!(&request.command, ChatCommand::UserMessage { content, .. } if content.as_str() == Some("ship it"))
        });
        let added = session.messages.iter().any(|message| {
            message.role == "user" && message.content.content_text_only() == "ship it"
        });
        assert!(queued || added);
    }

    #[tokio::test]
    async fn hooks_fire_hook_id_runs_matching_webhook_jobs_only() {
        let app = test_app().await;
        let store = session_cron_store();
        let hook_id = format!("deploy-{}", Uuid::now_v7());
        let mut matching = Job::new_cron_agent_chat(
            String::new(),
            String::new(),
            "Run hook command".to_string(),
            true,
            false,
            unix_now_ms(),
        );
        matching.id = format!("hook_test_{}", Uuid::now_v7());
        matching.trigger = Trigger::Webhook {
            hook_id: hook_id.clone(),
        };
        matching.action = Action::Command {
            argv: stdout_command(),
            target: AgentTarget::Isolated,
            cwd: None,
            env: None,
            timeout_secs: Some(5),
        };
        matching.delivery = Delivery::None;
        let matching_id = matching.id.clone();
        store.add(matching).await.unwrap();

        let mut other = Job::new_cron_agent_chat(
            String::new(),
            String::new(),
            "Other hook command".to_string(),
            true,
            false,
            unix_now_ms(),
        );
        other.id = format!("hook_test_other_{}", Uuid::now_v7());
        other.trigger = Trigger::Webhook {
            hook_id: "other".to_string(),
        };
        other.action = Action::Command {
            argv: stdout_command(),
            target: AgentTarget::Isolated,
            cwd: None,
            env: None,
            timeout_secs: Some(5),
        };
        other.delivery = Delivery::None;
        let other_id = other.id.clone();
        store.add(other).await.unwrap();

        let router = router(app);
        let (status, value) =
            request_json(router, json!({"kind":"wake","hook_id": hook_id}), None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["inline"], Value::Null);
        assert_eq!(value["hook_jobs"]["matched"], json!(1));
        assert_eq!(value["hook_jobs"]["fired"], json!(1));
        let matching = store.get(&matching_id).await.unwrap();
        assert_eq!(matching.last_status.as_deref(), Some("fired"));
        assert_eq!(matching.fire_count, 1);
        assert_eq!(matching.trigger_at_ms, None);
        let other = store.get(&other_id).await.unwrap();
        assert_eq!(other.last_status, None);
        store.remove(&matching_id).await.unwrap();
        store.remove(&other_id).await.unwrap();
    }

    #[tokio::test]
    async fn hooks_fire_agent_and_hook_id_do_both() {
        let app = test_app().await;
        let store = session_cron_store();
        let hook_id = format!("deploy-{}", Uuid::now_v7());
        let mut matching = Job::new_cron_agent_chat(
            String::new(),
            String::new(),
            "Run hook command".to_string(),
            true,
            false,
            unix_now_ms(),
        );
        matching.id = format!("hook_test_both_{}", Uuid::now_v7());
        matching.trigger = Trigger::Webhook {
            hook_id: hook_id.clone(),
        };
        matching.action = Action::Command {
            argv: stdout_command(),
            target: AgentTarget::Isolated,
            cwd: None,
            env: None,
            timeout_secs: Some(5),
        };
        matching.delivery = Delivery::None;
        let matching_id = matching.id.clone();
        store.add(matching).await.unwrap();

        let router = router(app.clone());
        let (status, value) = request_json(
            router,
            json!({"kind":"agent","message":"ship both","hook_id": hook_id}),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["inline"]["kind"], json!("agent"));
        assert_eq!(value["hook_jobs"]["matched"], json!(1));
        assert_eq!(value["hook_jobs"]["fired"], json!(1));
        let matching = store.get(&matching_id).await.unwrap();
        assert_eq!(matching.last_status.as_deref(), Some("fired"));
        let job_id = value["inline"]["job_id"].as_str().unwrap();
        let prefix = format!("cron_{job_id}_");
        let sessions = app.gcx.chat_sessions.read().await;
        assert!(sessions.keys().any(|chat_id| chat_id.starts_with(&prefix)));
        drop(sessions);
        store.remove(&matching_id).await.unwrap();
    }

    #[tokio::test]
    async fn jobs_by_hook_id_matches_webhook_triggers_only() {
        let store = Arc::new(InMemoryCronStore::new());
        let mut matching = Job::new_cron_agent_chat(
            "*/5 * * * *".to_string(),
            "Check".to_string(),
            "Check".to_string(),
            true,
            false,
            1,
        );
        matching.id = "matching".to_string();
        matching.trigger = Trigger::Webhook {
            hook_id: "deploy".to_string(),
        };
        let mut nonmatching = matching.clone();
        nonmatching.id = "nonmatching".to_string();
        nonmatching.trigger = Trigger::Webhook {
            hook_id: "other".to_string(),
        };
        let mut timed = matching.clone();
        timed.id = "timed".to_string();
        timed.trigger = Trigger::Cron {
            expr: "*/5 * * * *".to_string(),
            tz: None,
        };
        store.add(matching).await.unwrap();
        store.add(nonmatching).await.unwrap();
        store.add(timed).await.unwrap();

        let jobs = store.jobs_by_hook_id("deploy").await;

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "matching");
    }
}
