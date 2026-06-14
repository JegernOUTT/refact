use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::scheduler::{
    active_durable_cron_store, human_schedule, next_run_ms, scheduler_timezone, session_cron_store,
};
use crate::tools::tool_cron_create::{create_cron_job, CronCreateInput, CronCreateRuntime};

#[derive(Debug, Serialize)]
pub struct CronTaskResponse {
    pub id: String,
    pub cron: String,
    pub human_schedule: String,
    pub description: String,
    pub prompt: String,
    pub recurring: bool,
    pub durable: bool,
    pub next_fire_at_ms: u64,
    pub fire_count: u32,
    pub created_at_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct CronCreateRequest {
    pub cron: String,
    pub prompt: String,
    #[serde(default = "default_recurring")]
    pub recurring: bool,
    #[serde(default)]
    pub durable: bool,
    pub description: String,
    pub chat_id: String,
    pub mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CronCreateResponse {
    pub id: String,
    pub human_schedule: String,
    pub recurring: bool,
    pub durable: bool,
}

#[derive(Debug, Serialize)]
pub struct CronDeleteResponse {
    pub removed: bool,
}

fn default_recurring() -> bool {
    true
}

pub async fn handle_v1_scheduler_cron_get(
    State(app): State<AppState>,
) -> Result<Json<Vec<CronTaskResponse>>, ScratchError> {
    let now_ms = Utc::now().timestamp_millis().max(0) as u64;
    let tz = scheduler_timezone();

    let mut tasks = session_cron_store()
        .list()
        .await
        .into_iter()
        .map(|task| task_response(task, now_ms, tz))
        .collect::<Vec<_>>();

    let durable = active_durable_cron_store(app.gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if let Some(store) = durable {
        tasks.extend(
            store
                .list()
                .await
                .into_iter()
                .map(|task| task_response(task, now_ms, tz)),
        );
    }

    tasks.sort_by(|a, b| {
        a.next_fire_at_ms
            .cmp(&b.next_fire_at_ms)
            .then(a.id.cmp(&b.id))
    });
    Ok(Json(tasks))
}

pub async fn handle_v1_scheduler_cron_post(
    State(app): State<AppState>,
    Json(request): Json<CronCreateRequest>,
) -> Result<Json<CronCreateResponse>, ScratchError> {
    validate_chat_target(&app, &request.chat_id).await?;
    let durable_store = active_durable_cron_store(app.gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let runtime = CronCreateRuntime {
        session_store: session_cron_store(),
        durable_store,
        change_notify: crate::scheduler::runner_change_notify(),
        now_ms: unix_now_ms(),
        timezone: scheduler_timezone(),
        chat_id: Some(request.chat_id),
        mode: request.mode,
    };
    let outcome = create_cron_job(
        CronCreateInput {
            cron: request.cron,
            prompt: request.prompt,
            recurring: request.recurring,
            durable: request.durable,
            description: request.description,
        },
        runtime,
    )
    .await
    .map_err(|error| ScratchError::new(StatusCode::BAD_REQUEST, error))?;

    Ok(Json(CronCreateResponse {
        id: outcome.task.id,
        human_schedule: outcome.human_schedule,
        recurring: outcome.task.recurring,
        durable: outcome.task.durable,
    }))
}

pub async fn handle_v1_scheduler_cron_delete(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CronDeleteResponse>, ScratchError> {
    let mut removed = session_cron_store()
        .remove(&id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if !removed {
        let durable = active_durable_cron_store(app.gcx.clone())
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        if let Some(store) = durable {
            removed = store
                .remove(&id)
                .await
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        }
    }

    if removed {
        crate::scheduler::runner_change_notify().notify_waiters();
    }

    Ok(Json(CronDeleteResponse { removed }))
}

fn task_response(task: crate::scheduler::Job, now_ms: u64, tz: chrono_tz::Tz) -> CronTaskResponse {
    let cron = task.cron_expr().unwrap_or_default().to_string();
    let human_schedule = human_schedule(&cron);
    let prompt = first_chars(task.prompt().unwrap_or_default(), 200);
    let next_fire_at_ms = next_run_ms(&task, now_ms, tz).unwrap_or(0);
    CronTaskResponse {
        id: task.id,
        cron,
        human_schedule,
        description: task.description,
        prompt,
        recurring: task.recurring,
        durable: task.durable,
        next_fire_at_ms,
        fire_count: task.fire_count,
        created_at_ms: task.created_at_ms,
    }
}

fn first_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

async fn validate_chat_target(app: &AppState, chat_id: &str) -> Result<(), ScratchError> {
    if chat_id.trim().is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "chat_id is required".to_string(),
        ));
    }
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("chat session `{chat_id}` not found"),
        ));
    };
    let session = session_arc.lock().await;
    if session.closed {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("chat session `{chat_id}` is closed"),
        ));
    }
    Ok(())
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod scheduler_tests;
