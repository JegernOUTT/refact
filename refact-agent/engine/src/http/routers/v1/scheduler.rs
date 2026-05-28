use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use chrono_tz::Tz;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_correction::get_active_project_path;
use crate::scheduler::{human_schedule, next_run_ms, session_cron_store, CronStore, JsonFileCronStore};
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
    let mut tasks = session_cron_store()
        .list()
        .await
        .into_iter()
        .map(|task| CronTaskResponse {
            id: task.id,
            cron: task.cron.clone(),
            human_schedule: human_schedule(&task.cron),
            description: task.description,
            prompt: first_chars(&task.prompt, 200),
            recurring: task.recurring,
            durable: task.durable,
            next_fire_at_ms: next_run_ms(&task.cron, now_ms, local_timezone()).unwrap_or(0),
            fire_count: task.fire_count,
            created_at_ms: task.created_at_ms,
        })
        .collect::<Vec<_>>();

    if let Some(project_root) = active_project_root(&app).await {
        let store = JsonFileCronStore::new(project_root)
            .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        tasks.extend(store.list().await.into_iter().map(|task| CronTaskResponse {
            id: task.id,
            cron: task.cron.clone(),
            human_schedule: human_schedule(&task.cron),
            description: task.description,
            prompt: first_chars(&task.prompt, 200),
            recurring: task.recurring,
            durable: task.durable,
            next_fire_at_ms: next_run_ms(&task.cron, now_ms, local_timezone()).unwrap_or(0),
            fire_count: task.fire_count,
            created_at_ms: task.created_at_ms,
        }));
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
    let project_root = active_project_root(&app).await;
    let durable_store = project_root
        .map(|project_root| {
            JsonFileCronStore::new(project_root).map(|store| Arc::new(store) as Arc<dyn CronStore>)
        })
        .transpose()
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let runtime = CronCreateRuntime {
        session_store: session_cron_store(),
        durable_store,
        change_notify: crate::scheduler::runner_change_notify(),
        now_ms: unix_now_ms(),
        timezone: local_timezone(),
        chat_id: None,
        mode: None,
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
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;

    if !removed {
        if let Some(project_root) = active_project_root(&app).await {
            let store = JsonFileCronStore::new(project_root)
                .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
            removed = store
                .remove(&id)
                .await
                .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        }
    }

    if removed {
        crate::scheduler::runner_change_notify().notify_waiters();
    }

    Ok(Json(CronDeleteResponse { removed }))
}

async fn active_project_root(app: &AppState) -> Option<PathBuf> {
    get_active_project_path(app.gcx.clone()).await
}

fn first_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn local_timezone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|value| value.parse::<Tz>().ok())
        .or_else(|| {
            std::env::var("TZ")
                .ok()
                .and_then(|value| value.trim_start_matches(':').parse::<Tz>().ok())
        })
        .unwrap_or(chrono_tz::UTC)
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod scheduler_tests;
