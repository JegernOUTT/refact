use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::scheduler::schedule::parse_schedule;
use crate::scheduler::{
    active_durable_cron_store, cron_task_response, delivery_kind, human_schedule_for_trigger,
    next_run_ms, scheduler_timezone, session_cron_store, Action, AgentTarget, CronStore,
    CronTaskResponse, Delivery, DeliveryResponse, Job, Trigger, delivery_from_value,
};
use crate::tools::tool_cron_create::MAX_CRON_JOBS;

const ONE_YEAR_MS: u64 = 365 * 24 * 60 * 60 * 1000;

#[derive(Debug, Deserialize)]
pub struct CronCreateRequest {
    pub cron: Option<String>,
    pub every: Option<String>,
    pub at: Option<String>,
    pub tz: Option<String>,
    pub trigger: Option<serde_json::Value>,
    pub hook_id: Option<String>,
    pub prompt: Option<String>,
    pub command: Option<String>,
    pub command_argv: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub timeout_secs: Option<u64>,
    pub delivery: Option<serde_json::Value>,
    #[serde(default)]
    pub recurring: Option<bool>,
    #[serde(default)]
    pub durable: Option<bool>,
    #[serde(default)]
    pub isolated: Option<bool>,
    pub description: String,
    #[serde(default)]
    pub chat_id: String,
    pub mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CronCreateResponse {
    pub id: String,
    pub human_schedule: String,
    pub recurring: bool,
    pub durable: bool,
    pub action_kind: String,
    pub delivery_kind: String,
    pub delivery: DeliveryResponse,
}

#[derive(Debug, Serialize)]
pub struct CronDeleteResponse {
    pub removed: bool,
}

#[derive(Debug, Deserialize)]
pub struct CronUpdateRequest {
    pub cron: Option<String>,
    pub every: Option<String>,
    pub at: Option<String>,
    pub tz: Option<String>,
    pub prompt: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub run_now: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CronUpdateResponse {
    pub id: String,
    pub updated: bool,
    pub human_schedule: String,
}

#[derive(Debug, Serialize)]
pub struct CronRunResponse {
    pub id: String,
    pub triggered: bool,
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
    validate_create_target(&app, &request).await?;
    let durable_store = active_durable_cron_store(app.gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let task = create_http_cron_job(request, durable_store)
        .await
        .map_err(|error| ScratchError::new(StatusCode::BAD_REQUEST, error))?;
    let human = human_schedule_for_trigger(&task.trigger);
    let action_kind = task.action_kind().to_string();
    let delivery_kind = delivery_kind(&task.delivery).to_string();
    let delivery = DeliveryResponse::from_delivery(&task.delivery);

    Ok(Json(CronCreateResponse {
        id: task.id,
        human_schedule: human,
        recurring: task.recurring,
        durable: task.durable,
        action_kind,
        delivery_kind,
        delivery,
    }))
}

pub async fn handle_v1_scheduler_cron_patch(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CronUpdateRequest>,
) -> Result<Json<CronUpdateResponse>, ScratchError> {
    let now_ms = unix_now_ms();
    let (store, mut task) = find_task_store(&app, &id).await?;
    apply_update(&mut task, request, now_ms)
        .map_err(|error| ScratchError::new(StatusCode::BAD_REQUEST, error))?;
    if !store
        .replace(task.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Scheduled task `{id}` not found"),
        ));
    }
    crate::scheduler::runner_change_notify().notify_waiters();
    let human_schedule = human_schedule_for_trigger(&task.trigger);
    Ok(Json(CronUpdateResponse {
        id: task.id,
        updated: true,
        human_schedule,
    }))
}

pub async fn handle_v1_scheduler_cron_run(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CronRunResponse>, ScratchError> {
    let (store, mut task) = find_task_store(&app, &id).await?;
    task.trigger_at_ms = Some(unix_now_ms());
    if !store
        .replace(task.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("Scheduled task `{id}` not found"),
        ));
    }
    crate::scheduler::runner_change_notify().notify_waiters();
    Ok(Json(CronRunResponse {
        id: task.id,
        triggered: true,
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

fn task_response(task: Job, now_ms: u64, tz: chrono_tz::Tz) -> CronTaskResponse {
    let next_fire_at_ms = next_run_ms(&task, now_ms, tz).unwrap_or(0);
    cron_task_response(&task, next_fire_at_ms)
}

async fn create_http_cron_job(
    request: CronCreateRequest,
    durable_store: Option<std::sync::Arc<dyn CronStore>>,
) -> Result<Job, String> {
    validate_http_action_args(&request)?;
    let now_ms = unix_now_ms();
    let trigger = request_trigger(&request, now_ms)?;
    let recurring = if matches!(trigger, Trigger::Once { .. }) {
        false
    } else {
        request
            .recurring
            .unwrap_or_else(|| !matches!(trigger, Trigger::Once { .. }))
    };
    let durable = request.durable.unwrap_or_else(|| durable_store.is_some());
    let mut task = Job::new_cron_agent_chat(
        trigger_cron_expr(&trigger).unwrap_or_default().to_string(),
        request.prompt.clone().unwrap_or_default(),
        request.description.clone(),
        recurring,
        durable,
        now_ms,
    );
    task.set_trigger(trigger);
    task.delivery = request_delivery(&request)?;
    apply_http_action(&mut task, &request)?;
    validate_next_fire(&task, now_ms)?;

    let session_store = session_cron_store();
    let durable_count = match &durable_store {
        Some(store) => store.list().await.len(),
        None => 0,
    };
    let total_tasks = session_store.list().await.len() + durable_count;
    if total_tasks >= MAX_CRON_JOBS {
        return Err(format!(
            "Too many scheduled jobs (max {MAX_CRON_JOBS}). Cancel one first."
        ));
    }
    if task.durable && durable_store.is_none() {
        return Err("No project root available for durable scheduled jobs".to_string());
    }
    let store = if task.durable {
        durable_store.as_ref().unwrap().clone()
    } else {
        session_store
    };
    store.add(task.clone()).await?;
    crate::scheduler::runner_change_notify().notify_waiters();
    Ok(task)
}

fn validate_http_action_args(request: &CronCreateRequest) -> Result<(), String> {
    if request
        .command_argv
        .as_ref()
        .is_some_and(|argv| argv.iter().any(|item| item.trim().is_empty()))
    {
        return Err("command_argv contains an empty argument".to_string());
    }
    let agent_turn = request
        .prompt
        .as_ref()
        .is_some_and(|prompt| !prompt.trim().is_empty());
    let command = request
        .command
        .as_ref()
        .is_some_and(|command| !command.trim().is_empty());
    let command_argv = request
        .command_argv
        .as_ref()
        .is_some_and(|argv| !argv.is_empty());
    let delivery = request_delivery(request)?;
    if agent_turn && !matches!(delivery, Delivery::Chat) {
        return Err("non-chat delivery is only supported for command jobs".to_string());
    }
    match (agent_turn, usize::from(command) + usize::from(command_argv)) {
        (true, 0) | (false, 1) => Ok(()),
        (false, 0) => Err("one of `prompt`, `command`, or `command_argv` is required".to_string()),
        (true, _) => Err("exactly one action is allowed: prompt XOR command".to_string()),
        (false, _) => Err("exactly one of `command` or `command_argv` is allowed".to_string()),
    }
}

fn request_delivery(request: &CronCreateRequest) -> Result<Delivery, String> {
    request
        .delivery
        .as_ref()
        .map(delivery_from_value)
        .transpose()
        .map(|delivery| delivery.unwrap_or(Delivery::Chat))
}

fn apply_http_action(task: &mut Job, request: &CronCreateRequest) -> Result<(), String> {
    if let Some(prompt) = request
        .prompt
        .clone()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        task.action = Action::AgentTurn {
            prompt: prompt.trim().to_string(),
            target: AgentTarget::ExistingChat {
                chat_id: String::new(),
            },
            mode: request.mode.clone(),
            model: None,
            tools: None,
        };
        apply_http_agent_target(task, request);
        return Ok(());
    }
    task.action = Action::Command {
        argv: http_command_argv(request)?,
        target: AgentTarget::ExistingChat {
            chat_id: request.chat_id.clone(),
        },
        cwd: request.cwd.clone(),
        env: None,
        timeout_secs: request.timeout_secs,
    };
    Ok(())
}

fn apply_http_agent_target(task: &mut Job, request: &CronCreateRequest) {
    if request.isolated == Some(true) {
        task.set_isolated();
    } else {
        task.set_existing_chat(Some(request.chat_id.clone()));
    }
}

fn http_command_argv(request: &CronCreateRequest) -> Result<Vec<String>, String> {
    if let Some(argv) = request.command_argv.clone() {
        if argv.iter().any(|item| item.trim().is_empty()) {
            return Err("command_argv contains an empty argument".to_string());
        }
        return Ok(argv);
    }
    let command = request
        .command
        .as_deref()
        .ok_or_else(|| "command action requires `command` or `command_argv`".to_string())?;
    shell_words::split(command).map_err(|error| format!("failed to parse command: {error}"))
}

async fn find_task_store(
    app: &AppState,
    id: &str,
) -> Result<(std::sync::Arc<dyn CronStore>, Job), ScratchError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "id is required".to_string(),
        ));
    }
    let session_store = session_cron_store();
    if let Some(task) = session_store.get(id).await {
        return Ok((session_store, task));
    }
    let durable = active_durable_cron_store(app.gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if let Some(store) = durable {
        if let Some(task) = store.get(id).await {
            return Ok((store, task));
        }
    }
    Err(ScratchError::new(
        StatusCode::BAD_REQUEST,
        format!("Scheduled task `{id}` not found"),
    ))
}

fn apply_update(task: &mut Job, request: CronUpdateRequest, now_ms: u64) -> Result<(), String> {
    let schedule_requested =
        request.cron.is_some() || request.every.is_some() || request.at.is_some();
    if schedule_requested {
        let trigger = parse_schedule(
            request.cron.as_deref(),
            request.every.as_deref(),
            request.at.as_deref(),
            request.tz.as_deref(),
            now_ms,
        )?;
        if matches!(trigger, Trigger::Once { .. }) {
            task.recurring = false;
            task.auto_expire_after_ms = 0;
        }
        task.set_trigger(trigger);
        validate_next_fire(task, now_ms)?;
    } else if request.tz.is_some() {
        return Err("tz can only be changed with a cron schedule".to_string());
    }
    if let Some(prompt) = request.prompt {
        set_job_prompt(task, prompt)?;
    }
    if let Some(description) = request.description {
        task.description = description;
    }
    if let Some(enabled) = request.enabled {
        task.enabled = enabled;
        task.paused_at_ms = if enabled { None } else { Some(now_ms) };
    }
    if request.run_now.unwrap_or(false) {
        task.trigger_at_ms = Some(now_ms);
    }
    Ok(())
}

fn set_job_prompt(task: &mut Job, value: String) -> Result<(), String> {
    match &mut task.action {
        Action::AgentTurn { prompt, .. } => {
            *prompt = value;
            Ok(())
        }
        Action::Command { .. } => Err("prompt can only be changed for agent-turn jobs".to_string()),
    }
}

fn validate_next_fire(task: &Job, now_ms: u64) -> Result<(), String> {
    if matches!(
        task.trigger,
        Trigger::Webhook { .. } | Trigger::Manual | Trigger::OnProcessExit { .. }
    ) {
        return Ok(());
    }
    let next =
        next_run_ms(task, now_ms, scheduler_timezone()).ok_or_else(no_match_in_year_error)?;
    if matches!(task.trigger, Trigger::Cron { .. }) && next.saturating_sub(now_ms) > ONE_YEAR_MS {
        return Err(no_match_in_year_error());
    }
    Ok(())
}

fn trigger_cron_expr(trigger: &Trigger) -> Option<&str> {
    match trigger {
        Trigger::Cron { expr, .. } => Some(expr.as_str()),
        _ => None,
    }
}

fn request_trigger(request: &CronCreateRequest, now_ms: u64) -> Result<Trigger, String> {
    if let Some(trigger) = request_webhook_trigger(request)? {
        if request.cron.is_some()
            || request.every.is_some()
            || request.at.is_some()
            || request.tz.is_some()
        {
            return Err(
                "webhook trigger is mutually exclusive with cron, every, at, and tz".to_string(),
            );
        }
        return Ok(trigger);
    }
    parse_schedule(
        request.cron.as_deref(),
        request.every.as_deref(),
        request.at.as_deref(),
        request.tz.as_deref(),
        now_ms,
    )
}

fn request_webhook_trigger(request: &CronCreateRequest) -> Result<Option<Trigger>, String> {
    let hook_id = request
        .hook_id
        .as_deref()
        .map(str::trim)
        .filter(|hook_id| !hook_id.is_empty())
        .map(str::to_string);
    let Some(trigger) = request.trigger.as_ref() else {
        return Ok(hook_id.map(|hook_id| Trigger::Webhook { hook_id }));
    };
    match trigger {
        serde_json::Value::Object(map) => {
            let kind = map
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("webhook");
            if kind != "webhook" {
                return Err(format!("unsupported trigger `{kind}`"));
            }
            let hook_id = map
                .get("hook_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|hook_id| !hook_id.is_empty())
                .map(str::to_string)
                .or(hook_id)
                .ok_or_else(|| "trigger webhook requires `hook_id`".to_string())?;
            Ok(Some(Trigger::Webhook { hook_id }))
        }
        serde_json::Value::String(kind) if kind == "webhook" => {
            let hook_id =
                hook_id.ok_or_else(|| "trigger webhook requires `hook_id`".to_string())?;
            Ok(Some(Trigger::Webhook { hook_id }))
        }
        serde_json::Value::Null => Ok(hook_id.map(|hook_id| Trigger::Webhook { hook_id })),
        other => Err(format!("argument `trigger` is invalid: {other:?}")),
    }
}

fn no_match_in_year_error() -> String {
    "matches no calendar date in the next year".to_string()
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

async fn validate_create_target(
    app: &AppState,
    request: &CronCreateRequest,
) -> Result<(), ScratchError> {
    let delivery = request_delivery(request)
        .map_err(|error| ScratchError::new(StatusCode::BAD_REQUEST, error))?;
    let agent_turn = request
        .prompt
        .as_ref()
        .is_some_and(|prompt| !prompt.trim().is_empty());
    if agent_turn && request.isolated == Some(true) {
        return Ok(());
    }
    if agent_turn || matches!(delivery, Delivery::Chat) {
        validate_chat_target(app, &request.chat_id).await?;
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
