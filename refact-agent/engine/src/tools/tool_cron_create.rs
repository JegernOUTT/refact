use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{json, Value};
use tokio::sync::{Mutex as AMutex, Notify};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::files_correction::get_active_project_path;
use crate::scheduler::schedule::parse_schedule;
use crate::scheduler::{
    human_schedule, next_run_ms, scheduler_timezone, session_cron_store, Action, AgentTarget,
    CronStore, Delivery, Job, JsonFileCronStore, Trigger, delivery_from_value,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub const MAX_CRON_JOBS: usize = 50;
const ONE_YEAR_MS: u64 = 365 * 24 * 60 * 60 * 1000;

pub struct ToolCronCreate {
    pub config_path: String,
}

impl ToolCronCreate {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CronCreateInput {
    pub(crate) cron: Option<String>,
    pub(crate) every: Option<String>,
    pub(crate) at: Option<String>,
    pub(crate) tz: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) command_argv: Option<Vec<String>>,
    pub(crate) cwd: Option<String>,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) delivery: Delivery,
    pub(crate) recurring: Option<bool>,
    pub(crate) durable: bool,
    pub(crate) isolated: bool,
    pub(crate) description: String,
}

#[derive(Clone)]
pub(crate) struct CronCreateRuntime {
    pub(crate) session_store: Arc<dyn CronStore>,
    pub(crate) durable_store: Option<Arc<dyn CronStore>>,
    pub(crate) change_notify: Arc<Notify>,
    pub(crate) now_ms: u64,
    pub(crate) timezone: Tz,
    pub(crate) chat_id: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) model: Option<String>,
}

#[derive(Debug)]
pub(crate) struct CronCreateOutcome {
    pub(crate) task: Job,
    pub(crate) human_schedule: String,
    pub(crate) summary: String,
}

#[async_trait]
impl Tool for ToolCronCreate {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "cron_create".to_string(),
            display_name: "Create Scheduled Prompt".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Schedule a prompt to be enqueued later. Use a standard 5-field cron expression (`minute hour day-of-month month day-of-week`) evaluated in the local timezone. Set `recurring` to true for repeated prompts or false for a one-shot prompt that is removed after it fires. Set `durable` to true when the job should survive engine restarts in the current project; leave it false for a session-only in-memory schedule. Scheduler jitter is applied automatically so jobs may run shortly after the exact cron instant. Recurring jobs auto-expire after 30 days unless canceled earlier.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cron": { "type": "string", "description": "Standard 5-field cron expression in local time. Required unless every or at is set." },
                    "every": { "type": "string", "description": "Interval such as 30m, 2h, or 1d. Mutually exclusive with cron and at." },
                    "at": { "type": "string", "description": "One-shot time as RFC3339 or relative duration such as in 30m. Mutually exclusive with cron and every." },
                    "tz": { "type": "string", "description": "IANA timezone for cron schedules, such as UTC or Asia/Kolkata." },
                    "prompt": { "type": "string", "description": "Prompt enqueued at each fire time. Mutually exclusive with command and command_argv." },
                    "command": { "type": "string", "description": "Command line to shell-split and run without an agent turn. Mutually exclusive with prompt and command_argv." },
                    "command_argv": { "type": "array", "items": { "type": "string" }, "description": "Command argv to run without an agent turn. Mutually exclusive with prompt and command." },
                    "cwd": { "type": "string", "description": "Optional command working directory, resolved under the active project." },
                    "timeout_secs": { "type": "integer", "description": "Optional command timeout in seconds." },
                    "delivery": {
                        "oneOf": [
                            { "type": "string", "enum": ["chat", "none"] },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string", "enum": ["webhook"] },
                                    "url": { "type": "string" },
                                    "token": { "type": "string" }
                                },
                                "required": ["url"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string", "enum": ["notifier"] },
                                    "integration_id": { "type": "string" },
                                    "target": { "type": "string" }
                                },
                                "required": ["integration_id"]
                            }
                        ],
                        "description": "Delivery target: chat (default), none, webhook {url, token?}, or notifier {integration_id, target?}."
                    },
                    "recurring": { "type": "boolean", "default": true },
                    "durable": { "type": "boolean", "default": false },
                    "isolated": { "type": "boolean", "default": false, "description": "Create a fresh isolated chat session for each fire instead of enqueueing into the current chat." },
                    "description": { "type": "string", "description": "Short description (≤80 chars) shown in cron_list UI." }
                },
                "required": ["description"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let input = input_from_args(args)?;
        let (app, chat_id, project_root, mode, model) = cron_tool_context(ccx).await;
        let runtime = runtime_for_project(
            project_root,
            Some(chat_id.clone()),
            mode,
            model,
            unix_now_ms(),
            scheduler_timezone(),
        )?;
        let outcome = create_cron_job(input, runtime).await?;

        emit_created_notice(app, &chat_id, &outcome.task, &outcome.summary).await;

        let output = json!({
            "id": outcome.task.id,
            "human_schedule": outcome.human_schedule,
            "recurring": outcome.task.recurring,
            "durable": outcome.task.durable,
            "action_kind": outcome.task.action_kind(),
            "delivery": delivery_output(&outcome.task.delivery),
            "isolated": job_is_isolated(&outcome.task),
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output.to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn delivery_output(delivery: &Delivery) -> Value {
    match delivery {
        Delivery::Chat => json!({"kind": "chat"}),
        Delivery::Webhook { url, token } => json!({
            "kind": "webhook",
            "url": url,
            "has_token": token.as_ref().is_some_and(|token| !token.trim().is_empty()),
        }),
        Delivery::Notifier {
            integration_id,
            target,
        } => json!({
            "kind": "notifier",
            "integration_id": integration_id,
            "target": target,
        }),
        Delivery::None => json!({"kind": "none"}),
    }
}

fn input_from_args(args: &HashMap<String, Value>) -> Result<CronCreateInput, String> {
    let cron = optional_string_arg(args, "cron")?;
    let every = optional_string_arg(args, "every")?;
    let at = optional_string_arg(args, "at")?;
    let tz = optional_string_arg(args, "tz")?;
    if cron.is_none() && every.is_none() && at.is_none() {
        return Err("argument `cron` is required".to_string());
    }
    let prompt = optional_string_arg(args, "prompt")?;
    let command = optional_string_arg(args, "command")?;
    let command_argv = optional_string_array_arg(args, "command_argv")?;
    validate_action_args(prompt.as_ref(), command.as_ref(), command_argv.as_ref())?;
    let description = required_string_arg(args, "description")?;
    if description.chars().count() > 80 {
        return Err("description must be at most 80 characters".to_string());
    }
    Ok(CronCreateInput {
        cron,
        every,
        at,
        tz,
        prompt,
        command,
        command_argv,
        cwd: optional_string_arg(args, "cwd")?,
        timeout_secs: optional_u64_arg(args, "timeout_secs")?,
        delivery: delivery_arg(args)?,
        recurring: optional_bool_arg(args, "recurring")?,
        durable: optional_bool_arg(args, "durable")?.unwrap_or(false),
        isolated: optional_bool_arg(args, "isolated")?.unwrap_or(false),
        description,
    })
}

fn delivery_arg(args: &HashMap<String, Value>) -> Result<Delivery, String> {
    args.get("delivery")
        .map(delivery_from_value)
        .transpose()
        .map(|delivery| delivery.unwrap_or(Delivery::Chat))
}

fn required_string_arg(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(Value::String(_)) | Some(Value::Null) | None => {
            Err(format!("argument `{name}` is required"))
        }
        Some(value) => Err(format!("argument `{name}` is not a string: {value:?}")),
    }
}

fn optional_string_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_string()))
        }
        Some(Value::String(_)) | Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{name}` is not a string: {value:?}")),
    }
}

fn optional_bool_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<bool>, String> {
    match args.get(name) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{name}` is not a boolean: {value:?}")),
    }
}

fn optional_u64_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<u64>, String> {
    match args.get(name) {
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("argument `{name}` is not an unsigned integer: {value:?}")),
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{name}` is not an integer: {value:?}")),
    }
}

fn optional_string_array_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<Vec<String>>, String> {
    match args.get(name) {
        Some(Value::Array(values)) => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                match value {
                    Value::String(item) if !item.trim().is_empty() => out.push(item.clone()),
                    Value::String(_) => {
                        return Err(format!("argument `{name}` contains an empty string"));
                    }
                    other => {
                        return Err(format!(
                            "argument `{name}` contains a non-string value: {other:?}"
                        ));
                    }
                }
            }
            if out.is_empty() {
                Err(format!("argument `{name}` must not be empty"))
            } else {
                Ok(Some(out))
            }
        }
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{name}` is not an array: {value:?}")),
    }
}

fn validate_action_args(
    prompt: Option<&String>,
    command: Option<&String>,
    command_argv: Option<&Vec<String>>,
) -> Result<(), String> {
    let agent_turn = prompt.is_some();
    let command_count = usize::from(command.is_some()) + usize::from(command_argv.is_some());
    match (agent_turn, command_count) {
        (true, 0) | (false, 1) => Ok(()),
        (false, 0) => Err("one of `prompt`, `command`, or `command_argv` is required".to_string()),
        (true, _) => Err("exactly one action is allowed: prompt XOR command".to_string()),
        (false, _) => Err("exactly one of `command` or `command_argv` is allowed".to_string()),
    }
}

fn validate_delivery(input: &CronCreateInput) -> Result<(), String> {
    if input.prompt.is_some() && !matches!(input.delivery, Delivery::Chat) {
        return Err("non-chat delivery is only supported for command jobs".to_string());
    }
    if input.isolated && !matches!(input.delivery, Delivery::Chat) {
        return Err("isolated jobs only support chat delivery".to_string());
    }
    Ok(())
}

fn runtime_for_project(
    project_root: Option<PathBuf>,
    chat_id: Option<String>,
    mode: Option<String>,
    model: Option<String>,
    now_ms: u64,
    timezone: Tz,
) -> Result<CronCreateRuntime, String> {
    let durable_store = project_root
        .map(|project_root| {
            JsonFileCronStore::new(project_root).map(|store| Arc::new(store) as Arc<dyn CronStore>)
        })
        .transpose()?;
    Ok(CronCreateRuntime {
        session_store: session_cron_store(),
        durable_store,
        change_notify: crate::scheduler::runner_change_notify(),
        now_ms,
        timezone,
        chat_id,
        mode,
        model,
    })
}

async fn cron_tool_context(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> (
    crate::app_state::AppState,
    String,
    Option<PathBuf>,
    Option<String>,
    Option<String>,
) {
    let (app, gcx, chat_id, scoped_root) = {
        let locked = ccx.lock().await;
        (
            locked.app.clone(),
            locked.global_context.clone(),
            locked.chat_id.clone(),
            locked
                .execution_scope
                .as_ref()
                .map(|scope| scope.effective_root().to_path_buf()),
        )
    };
    let project_root = match scoped_root {
        Some(root) => Some(root),
        None => get_active_project_path(gcx.clone()).await,
    };
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(&chat_id).cloned()
    };
    let (mode, model) = if let Some(session_arc) = session_arc {
        let session = session_arc.lock().await;
        let mode = session.thread.mode.clone();
        let mode = if mode.is_empty() { None } else { Some(mode) };
        let model = session.thread.model.clone();
        let model = if model.is_empty() { None } else { Some(model) };
        (mode, model)
    } else {
        (None, None)
    };
    (app, chat_id, project_root, mode, model)
}

pub(crate) async fn create_cron_job(
    input: CronCreateInput,
    runtime: CronCreateRuntime,
) -> Result<CronCreateOutcome, String> {
    validate_delivery(&input)?;
    let trigger = parse_create_schedule(&input, runtime.now_ms)?;
    let recurring = if matches!(trigger, Trigger::Once { .. }) {
        false
    } else {
        input.recurring.unwrap_or(true)
    };

    let durable_count = match &runtime.durable_store {
        Some(store) => store.list().await.len(),
        None => 0,
    };
    let total_tasks = runtime.session_store.list().await.len() + durable_count;
    if total_tasks >= MAX_CRON_JOBS {
        return Err(format!(
            "Too many scheduled jobs (max {MAX_CRON_JOBS}). Cancel one first."
        ));
    }
    if input.durable && runtime.durable_store.is_none() {
        return Err("No project root available for durable scheduled jobs".to_string());
    }

    let mut task = Job::new_cron_agent_chat(
        input.cron.clone().unwrap_or_default(),
        input.prompt.clone().unwrap_or_default(),
        input.description.clone(),
        recurring,
        input.durable,
        runtime.now_ms,
    );
    task.trigger = trigger;
    task.delivery = input.delivery.clone();
    apply_cron_create_action(&mut task, &input, runtime.chat_id, runtime.model)?;
    task.set_mode(runtime.mode);
    validate_next_run(&task, runtime.now_ms, runtime.timezone)?;
    let human = human_schedule_for_trigger(&task.trigger);
    let store = if task.durable {
        runtime.durable_store.as_ref().unwrap().clone()
    } else {
        runtime.session_store.clone()
    };
    store.add(task.clone()).await?;
    runtime.change_notify.notify_waiters();
    let summary = format!("Scheduled {}: {} ({})", task.id, task.description, human);
    Ok(CronCreateOutcome {
        task,
        human_schedule: human,
        summary,
    })
}

fn apply_cron_create_action(
    task: &mut Job,
    input: &CronCreateInput,
    chat_id: Option<String>,
    model: Option<String>,
) -> Result<(), String> {
    if let Some(prompt) = input.prompt.clone() {
        task.action = Action::AgentTurn {
            prompt,
            target: if input.isolated {
                AgentTarget::Isolated
            } else {
                AgentTarget::ExistingChat {
                    chat_id: chat_id.unwrap_or_default(),
                }
            },
            mode: None,
            model: input.isolated.then_some(model).flatten(),
            tools: None,
        };
        return Ok(());
    }
    task.action = Action::Command {
        argv: command_argv_from_input(input)?,
        target: if input.isolated {
            AgentTarget::Isolated
        } else {
            AgentTarget::ExistingChat {
                chat_id: chat_id.unwrap_or_default(),
            }
        },
        cwd: input.cwd.clone(),
        env: None,
        timeout_secs: input.timeout_secs,
    };
    Ok(())
}

fn command_argv_from_input(input: &CronCreateInput) -> Result<Vec<String>, String> {
    if let Some(argv) = input.command_argv.clone() {
        return Ok(argv);
    }
    let command = input
        .command
        .as_deref()
        .ok_or_else(|| "command action requires `command` or `command_argv`".to_string())?;
    shell_words::split(command).map_err(|error| format!("failed to parse command: {error}"))
}

fn job_is_isolated(task: &Job) -> bool {
    matches!(
        &task.action,
        Action::AgentTurn {
            target: AgentTarget::Isolated,
            ..
        } | Action::Command {
            target: AgentTarget::Isolated,
            ..
        }
    )
}

#[cfg(test)]
fn job_model(task: &Job) -> Option<&str> {
    match &task.action {
        Action::AgentTurn { model, .. } => model.as_deref().filter(|model| !model.is_empty()),
        _ => None,
    }
}

fn parse_create_schedule(input: &CronCreateInput, now_ms: u64) -> Result<Trigger, String> {
    parse_schedule(
        input.cron.as_deref(),
        input.every.as_deref(),
        input.at.as_deref(),
        input.tz.as_deref(),
        now_ms,
    )
    .map_err(|error| {
        if input.every.is_none() && input.at.is_none() {
            format!(
                "Invalid cron expression: {}",
                error.trim_start_matches("Invalid cron expression: ")
            )
        } else {
            format!("Invalid schedule: {error}")
        }
    })
}

fn validate_next_run(task: &Job, now_ms: u64, timezone: Tz) -> Result<(), String> {
    let next = next_run_ms(task, now_ms, timezone).ok_or_else(no_match_in_year_error)?;
    if matches!(task.trigger, Trigger::Cron { .. }) && next.saturating_sub(now_ms) > ONE_YEAR_MS {
        return Err(no_match_in_year_error());
    }
    Ok(())
}

pub(crate) fn human_schedule_for_trigger(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Cron { expr, .. } => human_schedule(expr),
        Trigger::Interval { every_ms } => format!("every {}", human_duration(*every_ms)),
        Trigger::Once { at_ms } => Utc
            .timestamp_millis_opt(*at_ms as i64)
            .single()
            .map(|datetime| format!("once at {}", datetime.to_rfc3339()))
            .unwrap_or_else(|| format!("once at {at_ms}")),
        Trigger::Manual => "manual".to_string(),
        Trigger::Webhook { hook_id } => format!("webhook {hook_id}"),
        Trigger::OnProcessExit { match_kind } => format!("on process exit {match_kind}"),
    }
}

fn human_duration(ms: u64) -> String {
    for (unit_ms, singular, plural) in [
        (24 * 60 * 60 * 1000, "day", "days"),
        (60 * 60 * 1000, "hour", "hours"),
        (60 * 1000, "minute", "minutes"),
        (1000, "second", "seconds"),
    ] {
        if ms >= unit_ms && ms % unit_ms == 0 {
            let amount = ms / unit_ms;
            let unit = if amount == 1 { singular } else { plural };
            return format!("{amount} {unit}");
        }
    }
    format!("{ms}ms")
}

fn no_match_in_year_error() -> String {
    "matches no calendar date in the next year".to_string()
}

async fn emit_created_notice(
    app: crate::app_state::AppState,
    chat_id: &str,
    task: &Job,
    summary: &str,
) {
    let session_arc = crate::chat::get_or_create_session_with_trajectory(
        app.clone(),
        &app.chat.sessions,
        chat_id,
    )
    .await;
    let mut session = session_arc.lock().await;
    session.add_message(event(
        EventSubkind::SystemNotice,
        "scheduler.cron",
        json!({
            "id": task.id,
            "cron": task.cron_expr().unwrap_or_default(),
            "recurring": task.recurring,
            "durable": task.durable,
            "action_kind": task.action_kind(),
            "isolated": job_is_isolated(task),
        }),
        summary.to_string(),
    ));
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::*;
    use crate::scheduler::{scheduled_tasks_path, InMemoryCronStore};

    fn fixed_now_ms() -> u64 {
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn input(items: &[(&str, Value)]) -> CronCreateInput {
        input_from_args(&args(items)).unwrap()
    }

    fn default_input() -> CronCreateInput {
        input(&[
            ("cron", json!("*/5 * * * *")),
            ("prompt", json!("Check the build")),
            ("description", json!("Check build")),
        ])
    }

    fn tool_output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected simple text"),
            },
            _ => panic!("expected chat message"),
        }
    }

    fn runtime(
        session_store: Arc<dyn CronStore>,
        durable_store: Option<Arc<dyn CronStore>>,
        change_notify: Arc<Notify>,
    ) -> CronCreateRuntime {
        CronCreateRuntime {
            session_store,
            durable_store,
            change_notify,
            now_ms: fixed_now_ms(),
            timezone: chrono_tz::UTC,
            chat_id: Some("chat-1".to_string()),
            mode: Some("agent".to_string()),
            model: Some("model-1".to_string()),
        }
    }

    fn test_task(id: &str) -> Job {
        let mut task = Job::new_cron_agent_chat(
            "*/5 * * * *".to_string(),
            "Check".to_string(),
            "Check".to_string(),
            true,
            false,
            fixed_now_ms(),
        );
        task.id = id.to_string();
        task
    }

    #[tokio::test]
    async fn valid_recurring_creates() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "cron-chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ));
        let before = session_cron_store().list().await.len();
        let change_notify = crate::scheduler::runner_change_notify();
        let notified = change_notify.notified();
        let mut tool = ToolCronCreate {
            config_path: String::new(),
        };
        let output = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("cron", json!("*/5 * * * *")),
                    ("prompt", json!("Check the build")),
                    ("description", json!("Check build")),
                ]),
            )
            .await
            .unwrap(),
        );
        let output: Value = serde_json::from_str(&output).unwrap();
        let after = session_cron_store().list().await;
        let created = after
            .iter()
            .find(|task| task.id == output["id"].as_str().unwrap())
            .unwrap();

        assert_eq!(after.len(), before + 1);
        assert!(created.id.starts_with("cron_"));
        assert_eq!(output["human_schedule"], json!("every 5 minutes"));
        assert!(created.recurring);
        assert!(!created.durable);
        assert_eq!(created.chat_id(), Some("cron-chat"));
        tokio::time::timeout(std::time::Duration::from_secs(1), notified)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn invalid_cron_rejected() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let err = create_cron_job(
            input(&[
                ("cron", json!("* * * *")),
                ("prompt", json!("Check")),
                ("description", json!("Check")),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap_err();

        assert!(err.contains("Invalid cron expression"));
    }

    #[tokio::test]
    async fn every_creates_interval() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("every", json!("30m")),
                ("prompt", json!("Check")),
                ("description", json!("Check")),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert_eq!(
            outcome.task.trigger,
            Trigger::Interval {
                every_ms: 30 * 60_000
            }
        );
        assert!(outcome.task.recurring);
        assert_eq!(outcome.human_schedule, "every 30 minutes");
    }

    #[tokio::test]
    async fn at_creates_once() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("at", json!("in 30m")),
                ("prompt", json!("Check")),
                ("description", json!("Check")),
                ("recurring", json!(true)),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert_eq!(
            outcome.task.trigger,
            Trigger::Once {
                at_ms: fixed_now_ms() + 30 * 60_000
            }
        );
        assert!(!outcome.task.recurring);
        assert!(outcome.human_schedule.starts_with("once at "));
    }

    #[tokio::test]
    async fn bad_schedule_combo_rejected() {
        let err = input_from_args(&args(&[
            ("cron", json!("*/5 * * * *")),
            ("every", json!("30m")),
            ("prompt", json!("Check")),
            ("description", json!("Check")),
        ]))
        .and_then(|input| {
            parse_create_schedule(&input, fixed_now_ms())?;
            Ok(())
        })
        .unwrap_err();

        assert!(err.contains("exactly one of cron, every, or at must be set"));
    }

    #[tokio::test]
    async fn cron_only_is_unchanged() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            default_input(),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert_eq!(
            outcome.task.trigger,
            Trigger::Cron {
                expr: "*/5 * * * *".to_string(),
                tz: None,
            }
        );
        assert!(outcome.task.recurring);
        assert_eq!(outcome.human_schedule, "every 5 minutes");
        assert!(!job_is_isolated(&outcome.task));
        assert_eq!(outcome.task.chat_id(), Some("chat-1"));
    }

    #[tokio::test]
    async fn isolated_true_creates_isolated_target() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("prompt", json!("Check")),
                ("description", json!("Check")),
                ("isolated", json!(true)),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert!(job_is_isolated(&outcome.task));
        assert_eq!(outcome.task.chat_id(), None);
        assert_eq!(outcome.task.mode(), Some("agent"));
        assert_eq!(job_model(&outcome.task), Some("model-1"));
    }

    #[tokio::test]
    async fn isolated_default_creates_existing_chat_target() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            default_input(),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert!(!job_is_isolated(&outcome.task));
        assert_eq!(outcome.task.chat_id(), Some("chat-1"));
        assert_eq!(outcome.task.mode(), Some("agent"));
        assert_eq!(job_model(&outcome.task), None);
    }

    #[tokio::test]
    async fn command_string_creates_command_action() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("command", json!("printf 'hi frog'")),
                ("description", json!("Print frog")),
                ("cwd", json!(".")),
                ("timeout_secs", json!(12)),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert_eq!(outcome.task.action_kind(), "command");
        match outcome.task.action {
            Action::Command {
                argv,
                target,
                cwd,
                timeout_secs,
                ..
            } => {
                assert_eq!(argv, vec!["printf".to_string(), "hi frog".to_string()]);
                assert_eq!(
                    target,
                    AgentTarget::ExistingChat {
                        chat_id: "chat-1".to_string()
                    }
                );
                assert_eq!(cwd.as_deref(), Some("."));
                assert_eq!(timeout_secs, Some(12));
            }
            _ => panic!("expected command action"),
        }
    }

    #[tokio::test]
    async fn webhook_delivery_creates_command_job() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("command", json!("printf hi")),
                ("description", json!("Print frog")),
                (
                    "delivery",
                    json!({"kind": "webhook", "url": "http://127.0.0.1/hook", "token": "secret"}),
                ),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert_eq!(outcome.task.action_kind(), "command");
        assert_eq!(
            outcome.task.delivery,
            Delivery::Webhook {
                url: "http://127.0.0.1/hook".to_string(),
                token: Some("secret".to_string()),
            }
        );
        let output = delivery_output(&outcome.task.delivery);
        assert_eq!(output["has_token"], json!(true));
        assert_eq!(output.get("token"), None);
    }

    #[tokio::test]
    async fn notifier_delivery_creates_command_job() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("command", json!("printf hi")),
                ("description", json!("Print frog")),
                (
                    "delivery",
                    json!({"kind": "notifier", "integration_id": "notifier_telegram", "target": "chat-1"}),
                ),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        assert_eq!(outcome.task.action_kind(), "command");
        assert_eq!(
            outcome.task.delivery,
            Delivery::Notifier {
                integration_id: "notifier_telegram".to_string(),
                target: Some("chat-1".to_string()),
            }
        );
        let output = delivery_output(&outcome.task.delivery);
        assert_eq!(output["kind"], json!("notifier"));
        assert_eq!(output["integration_id"], json!("notifier_telegram"));
        assert_eq!(output["target"], json!("chat-1"));
    }

    #[tokio::test]
    async fn webhook_delivery_rejects_prompt_job() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let err = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("prompt", json!("Check")),
                ("description", json!("Check")),
                (
                    "delivery",
                    json!({"kind": "webhook", "url": "http://127.0.0.1/hook"}),
                ),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "non-chat delivery is only supported for command jobs");
    }

    #[tokio::test]
    async fn notifier_delivery_rejects_prompt_job() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let err = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("prompt", json!("Check")),
                ("description", json!("Check")),
                (
                    "delivery",
                    json!({"kind": "notifier", "integration_id": "notifier_telegram"}),
                ),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "non-chat delivery is only supported for command jobs");
    }

    #[tokio::test]
    async fn command_argv_creates_command_action() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let outcome = create_cron_job(
            input(&[
                ("cron", json!("*/5 * * * *")),
                ("command_argv", json!(["printf", "hi frog"])),
                ("description", json!("Print frog")),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap();

        match outcome.task.action {
            Action::Command { argv, .. } => {
                assert_eq!(argv, vec!["printf".to_string(), "hi frog".to_string()]);
            }
            _ => panic!("expected command action"),
        }
    }

    #[test]
    fn prompt_and_command_are_rejected() {
        let err = input_from_args(&args(&[
            ("cron", json!("*/5 * * * *")),
            ("prompt", json!("Check")),
            ("command", json!("printf hi")),
            ("description", json!("Check")),
        ]))
        .unwrap_err();

        assert_eq!(err, "exactly one action is allowed: prompt XOR command");
    }

    #[test]
    fn command_and_command_argv_are_rejected() {
        let err = input_from_args(&args(&[
            ("cron", json!("*/5 * * * *")),
            ("command", json!("printf hi")),
            ("command_argv", json!(["printf", "hi"])),
            ("description", json!("Check")),
        ]))
        .unwrap_err();

        assert_eq!(err, "exactly one of `command` or `command_argv` is allowed");
    }

    #[tokio::test]
    async fn no_match_in_year_rejected() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let err = create_cron_job(
            input(&[
                ("cron", json!("0 0 29 2 *")),
                ("prompt", json!("Leap check")),
                ("description", json!("Leap check")),
            ]),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "matches no calendar date in the next year");
    }

    #[tokio::test]
    async fn cap_enforced() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        for idx in 0..MAX_CRON_JOBS {
            session_store
                .add(test_task(&format!("cron_{idx}")))
                .await
                .unwrap();
        }

        let err = create_cron_job(
            default_input(),
            runtime(session_store, None, Arc::new(Notify::new())),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "Too many scheduled jobs (max 50). Cancel one first.");
    }

    #[tokio::test]
    async fn durable_writes_to_disk() {
        let temp = tempfile::tempdir().unwrap();
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let durable_store: Arc<dyn CronStore> =
            Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        let outcome = create_cron_job(
            input(&[
                ("cron", json!("0 9 * * 1-5")),
                ("prompt", json!("Standup prep")),
                ("description", json!("Standup prep")),
                ("durable", json!(true)),
            ]),
            runtime(
                session_store.clone(),
                Some(durable_store),
                Arc::new(Notify::new()),
            ),
        )
        .await
        .unwrap();

        assert!(outcome.task.durable);
        assert!(session_store.list().await.is_empty());
        assert!(scheduled_tasks_path(temp.path()).is_file());
        let reloaded = JsonFileCronStore::new(temp.path()).unwrap();
        assert_eq!(reloaded.list().await, vec![outcome.task]);
    }
}
