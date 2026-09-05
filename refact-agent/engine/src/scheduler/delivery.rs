use std::time::{Duration, SystemTime, UNIX_EPOCH};

use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};
use serde::Serialize;
use serde_json::json;

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::internal_roles::{event, EventSubkind};

use super::types::{Action, AgentTarget, Delivery, Job};

const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize)]
struct WebhookPayload<'a> {
    job_id: &'a str,
    description: &'a str,
    status: &'a str,
    output: &'a str,
    ts: u64,
}

pub async fn deliver(app: &AppState, job: &Job, output: &str) -> Result<(), String> {
    match &job.delivery {
        Delivery::Chat => deliver_chat(app, job, output).await,
        Delivery::Webhook { url, token } => {
            deliver_webhook(app, job, output, url, token.as_deref()).await
        }
        Delivery::Notifier {
            integration_id,
            target,
        } => deliver_notifier(app, output, integration_id, target.as_deref()).await,
        Delivery::None => Ok(()),
    }
}

/// Command results are notices: they go through the shared delivery layer so a
/// busy chat queues the whole batch at its boundary instead of dropping it.
async fn deliver_chat(app: &AppState, job: &Job, output: &str) -> Result<(), String> {
    if output.is_empty() {
        return Ok(());
    }
    let chat_id = match job_target(job)? {
        AgentTarget::ExistingChat { chat_id } => {
            if !app.gcx.chat_sessions.read().await.contains_key(chat_id) {
                return Err(format!("Chat session {chat_id} not found"));
            }
            chat_id.clone()
        }
        AgentTarget::Isolated => {
            let chat_id = isolated_chat_id(job);
            let _ =
                get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, &chat_id)
                    .await;
            chat_id
        }
    };
    let messages = if job.last_status.as_deref() == Some("error") {
        vec![error_notice_message(job, output)]
    } else if matches!(job.action, Action::Command { .. }) {
        vec![cron_fire_message(job), output_message(job, output)]
    } else {
        vec![output_message(job, output)]
    };
    let outcome = deliver_job_messages(app, &chat_id, job, messages, false).await?;
    if outcome == DeliveryOutcome::Duplicate {
        tracing::debug!(
            "scheduled task {} chat delivery deduped for chat {}",
            job.id,
            chat_id
        );
    }
    Ok(())
}

/// Stable per-fire id so a scheduler retry of the same run cannot double-post.
pub(crate) fn delivery_id_for_fire(job_id: &str, fired_at_ms: u64, suffix: &str) -> String {
    format!("cron-{job_id}-{fired_at_ms}-{suffix}")
}

pub(crate) fn job_delivery_id(job: &Job, suffix: &str) -> String {
    delivery_id_for_fire(
        &job.id,
        job.last_fired_at_ms.unwrap_or(job.created_at_ms),
        suffix,
    )
}

pub(crate) async fn deliver_job_messages(
    app: &AppState,
    chat_id: &str,
    job: &Job,
    messages: Vec<ChatMessage>,
    wake: bool,
) -> Result<DeliveryOutcome, String> {
    let delivery = PendingDelivery::with_id(
        job_delivery_id(job, if wake { "turn" } else { "output" }),
        messages,
        job.push,
        "scheduler.cron".to_string(),
        wake,
    );
    crate::chat::delivery::deliver_to_chat(app.clone(), chat_id, delivery).await
}

/// Deliver a scheduled agent turn: the fire notice plus the prompt land as one
/// batch, at `job.push`, waking the chat once the batch is in the history.
pub(crate) async fn deliver_scheduled_turn(
    app: &AppState,
    chat_id: &str,
    job: &Job,
    fired_at_ms: u64,
    messages: Vec<ChatMessage>,
) -> Result<DeliveryOutcome, String> {
    // Fire count advances only after acceptance; retries (including restart
    // recovery) therefore reuse the same id independently of wall clock time.
    let mut delivery = PendingDelivery::with_id(
        format!("cron-{}-{}-turn", job.id, job.fire_count),
        messages,
        job.push,
        "scheduler.cron".to_string(),
        true,
    );
    delivery.thread_patch = super::runner::scheduled_thread_patch(job);
    let _ = fired_at_ms;
    crate::chat::delivery::deliver_to_chat(app.clone(), chat_id, delivery).await
}

/// Pure notices (catch-up, auto-expiry) never wake a chat.
pub(crate) async fn deliver_notice(
    app: &AppState,
    chat_id: &str,
    id: String,
    message: ChatMessage,
    push: PushMode,
) -> Result<DeliveryOutcome, String> {
    crate::chat::delivery::deliver_to_chat(
        app.clone(),
        chat_id,
        PendingDelivery::with_id(id, vec![message], push, "scheduler.cron".to_string(), false),
    )
    .await
}

async fn deliver_webhook(
    app: &AppState,
    job: &Job,
    output: &str,
    url: &str,
    token: Option<&str>,
) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("webhook url is required".to_string());
    }
    let payload = WebhookPayload {
        job_id: &job.id,
        description: &job.description,
        status: job.last_status.as_deref().unwrap_or("fired"),
        output,
        ts: current_timestamp_ms(),
    };
    let mut request = app
        .runtime
        .http_client
        .post(url)
        .json(&payload)
        .timeout(WEBHOOK_TIMEOUT);
    if let Some(token) = token.filter(|token| !token.trim().is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = tokio::time::timeout(WEBHOOK_TIMEOUT, request.send())
        .await
        .map_err(|_| "webhook delivery timed out".to_string())?
        .map_err(|error| format!("webhook delivery failed: {error}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "webhook delivery returned status {}",
            response.status()
        ))
    }
}

async fn deliver_notifier(
    app: &AppState,
    output: &str,
    integration_id: &str,
    target: Option<&str>,
) -> Result<(), String> {
    let notifier =
        crate::integrations::notifier::configured_notifier_backend(app.gcx.clone(), integration_id)
            .await?;
    notifier.send(target, output).await
}

fn job_target(job: &Job) -> Result<&AgentTarget, String> {
    match &job.action {
        Action::AgentTurn { target, .. } | Action::Command { target, .. } => Ok(target),
    }
}

fn isolated_chat_id(job: &Job) -> String {
    format!(
        "cron_{}_{}",
        job.id,
        job.last_fired_at_ms.unwrap_or_else(current_timestamp_ms)
    )
}

fn cron_fire_message(job: &Job) -> ChatMessage {
    let fired_at_ms = job.last_fired_at_ms.unwrap_or_else(current_timestamp_ms);
    event(
        EventSubkind::CronFire,
        "scheduler.cron",
        json!({
            "task_id": job.id,
            "cron": job.cron_expr().unwrap_or_default(),
            "recurring": job.recurring,
            "fire_count": job.fire_count.saturating_add(1),
            "final": job.recurring
                && job.auto_expire_after_ms > 0
                && fired_at_ms.saturating_sub(job.created_at_ms) > job.auto_expire_after_ms,
            "action_kind": job.action_kind(),
        }),
        job.description.clone(),
    )
}

fn output_message(job: &Job, output: &str) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "scheduler_delivery".to_string(),
        json!({
            "task_id": job.id,
            "delivery": "chat",
            "status": job.last_status.as_deref().unwrap_or("fired"),
        }),
    );
    if matches!(job.action, Action::Command { .. }) {
        extra.insert(
            "scheduler_command".to_string(),
            json!({
                "task_id": job.id,
                "status": job.last_status.as_deref().unwrap_or("fired"),
                "exit_code": null,
            }),
        );
    }
    ChatMessage {
        role: "plain_text".to_string(),
        content: ChatContent::SimpleText(output.to_string()),
        extra,
        ..Default::default()
    }
}

fn error_notice_message(job: &Job, output: &str) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "scheduler.cron",
        json!({
            "task_id": job.id,
            "action_kind": job.action_kind(),
            "status": job.last_status.as_deref().unwrap_or("error"),
            "exit_code": null,
        }),
        output.to_string(),
    )
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::Value;
    use tokio::sync::Mutex as AMutex;

    use super::*;
    use crate::chat::internal_roles::EVENT_ROLE;
    use crate::chat::types::ChatSession;
    use crate::scheduler::types::{CronRunRecord, Trigger};

    fn command_job(delivery: Delivery) -> Job {
        Job {
            id: "delivery-job".to_string(),
            description: "Deliver frogs".to_string(),
            enabled: true,
            durable: false,
            created_at_ms: 1_000,
            recurring: true,
            trigger: Trigger::Cron {
                expr: "*/5 * * * *".to_string(),
                tz: None,
            },
            action: Action::Command {
                argv: vec!["printf".to_string(), "frogs".to_string()],
                target: AgentTarget::ExistingChat {
                    chat_id: "chat-1".to_string(),
                },
                cwd: None,
                env: None,
                timeout_secs: None,
            },
            delivery,
            push: Default::default(),
            last_fired_at_ms: Some(2_000),
            fire_count: 2,
            last_status: Some("fired".to_string()),
            last_error: None,
            last_delivery_error: None,
            recent_runs: vec![CronRunRecord {
                at_ms: 2_000,
                status: "fired".to_string(),
                error: None,
            }],
            paused_at_ms: None,
            trigger_at_ms: None,
            auto_expire_after_ms: crate::scheduler::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
            retry_attempts: 0,
        }
    }

    async fn app_with_session() -> (tempfile::TempDir, AppState) {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let session = Arc::new(AMutex::new(ChatSession::new("chat-1".to_string())));
        gcx.chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), session);
        (workspace, AppState::from_gcx(gcx).await)
    }

    #[test]
    fn old_job_defaults_to_append_and_explicit_push_roundtrips() {
        let job = command_job(Delivery::Chat);
        let mut raw = serde_json::to_value(&job).unwrap();
        raw.as_object_mut().unwrap().remove("push");
        assert_eq!(
            serde_json::from_value::<Job>(raw.clone()).unwrap().push,
            PushMode::Append
        );
        for (wire, mode) in [
            ("preempt", PushMode::Preempt),
            ("append", PushMode::Append),
            ("when_idle", PushMode::WhenIdle),
        ] {
            raw["push"] = json!(wire);
            let job: Job = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(job.push, mode);
            assert_eq!(
                serde_json::from_value::<Job>(serde_json::to_value(&job).unwrap()).unwrap(),
                job
            );
        }
    }

    #[tokio::test]
    async fn busy_command_output_and_error_are_ordered_deduplicated_notices() {
        for failed in [false, true] {
            let (_workspace, app) = app_with_session().await;
            let arc = app
                .chat
                .sessions
                .read()
                .await
                .get("chat-1")
                .cloned()
                .unwrap();
            arc.lock().await.start_stream().unwrap();
            let mut job = command_job(Delivery::Chat);
            if failed {
                job.last_status = Some("error".into());
            }
            deliver(&app, &job, "result").await.unwrap();
            deliver(&app, &job, "result").await.unwrap();
            let mut session = arc.lock().await;
            assert_eq!(session.pending_deliveries.len(), 1);
            assert!(!session.pending_deliveries[0].wake);
            session.finish_stream(Some("stop".into()));
            session.drain_pending_deliveries();
            if failed {
                assert_eq!(session.messages.len(), 1);
                assert_eq!(
                    session.messages[0].extra["event"]["subkind"],
                    "system_notice"
                );
            } else {
                assert_eq!(session.messages.len(), 2);
                assert_eq!(session.messages[0].extra["event"]["subkind"], "cron_fire");
                assert_eq!(session.messages[1].content.content_text_only(), "result");
            }
        }
    }

    #[tokio::test]
    async fn chat_delivery_injects_command_output() {
        let (_workspace, app) = app_with_session().await;
        let job = command_job(Delivery::Chat);

        deliver(&app, &job, "hello frogs").await.unwrap();

        let session_arc = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("chat-1")
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert!(session.messages.iter().any(|message| {
            message.role == EVENT_ROLE
                && message.extra["event"]["subkind"].as_str() == Some("cron_fire")
                && message.extra["event"]["payload"]["task_id"].as_str() == Some("delivery-job")
        }));
        let output = session
            .messages
            .iter()
            .find(|message| message.role == "plain_text")
            .unwrap();
        assert_eq!(output.content.content_text_only(), "hello frogs");
    }

    #[tokio::test]
    async fn chat_delivery_empty_output_is_silent() {
        let (_workspace, app) = app_with_session().await;
        let job = command_job(Delivery::Chat);

        deliver(&app, &job, "").await.unwrap();

        let session_arc = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("chat-1")
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert!(session.messages.is_empty());
    }

    #[tokio::test]
    async fn none_delivery_is_noop() {
        let (_workspace, app) = app_with_session().await;
        let job = command_job(Delivery::None);

        deliver(&app, &job, "hello frogs").await.unwrap();

        let session_arc = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("chat-1")
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert!(session.messages.is_empty());
    }

    #[tokio::test]
    async fn webhook_delivery_posts_json_with_auth() {
        let received = Arc::new(AMutex::new(Vec::<(Option<String>, Value)>::new()));
        let handler_received = received.clone();
        let router = Router::new().route(
            "/hook",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let handler_received = handler_received.clone();
                    async move {
                        let auth = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        handler_received.lock().await.push((auth, body));
                        axum::http::StatusCode::OK
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(router.into_make_service());
        let server_task = tokio::spawn(server);
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let job = command_job(Delivery::Webhook {
            url: format!("http://127.0.0.1:{port}/hook"),
            token: Some("secret".to_string()),
        });

        deliver(&app, &job, "hello webhook").await.unwrap();

        let received = received.lock().await;
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0.as_deref(), Some("Bearer secret"));
        assert_eq!(received[0].1["job_id"], json!("delivery-job"));
        assert_eq!(received[0].1["description"], json!("Deliver frogs"));
        assert_eq!(received[0].1["status"], json!("fired"));
        assert_eq!(received[0].1["output"], json!("hello webhook"));
        assert!(received[0].1["ts"].as_u64().unwrap() > 0);
        server_task.abort();
    }

    #[tokio::test]
    async fn notifier_delivery_posts_to_configured_telegram() {
        let temp = tempfile::tempdir().unwrap();
        let received = Arc::new(AMutex::new(Vec::<Value>::new()));
        let handler_received = received.clone();
        let router = Router::new().route(
            "/botsecret-token/sendMessage",
            post(move |Json(body): Json<Value>| {
                let handler_received = handler_received.clone();
                async move {
                    handler_received.lock().await.push(body);
                    axum::http::StatusCode::OK
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(router.into_make_service());
        let server_task = tokio::spawn(server);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![crate::files_correction::canonicalize_normalized_path(
                temp.path().to_path_buf(),
            )];
        let config_dir = temp.path().join(".refact").join("integrations.d");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("notifier_telegram.yaml"),
            format!(
                "bot_token: secret-token\ndefault_chat_id: default-chat\napi_base_for_test: http://127.0.0.1:{port}\n"
            ),
        )
        .unwrap();
        let app = AppState::from_gcx(gcx).await;
        let job = command_job(Delivery::Notifier {
            integration_id: "notifier_telegram".to_string(),
            target: Some("target-chat".to_string()),
        });

        deliver(&app, &job, "hello notifier").await.unwrap();

        let received = received.lock().await;
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["chat_id"], json!("target-chat"));
        assert_eq!(received[0]["text"], json!("hello notifier"));
        server_task.abort();
    }

    #[tokio::test]
    async fn notifier_delivery_missing_integration_errors() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let job = command_job(Delivery::Notifier {
            integration_id: "notifier_telegram".to_string(),
            target: None,
        });

        let err = deliver(&app, &job, "hello notifier").await.unwrap_err();

        assert!(err.contains("integration `notifier_telegram` not found"));
    }
}
