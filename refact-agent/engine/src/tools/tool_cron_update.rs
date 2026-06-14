use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::scheduler::schedule::parse_schedule;
use crate::scheduler::{active_durable_cron_store, session_cron_store, CronStore, Job, Trigger};
use crate::tools::tool_cron_create::human_schedule_for_trigger;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolCronUpdate {
    pub config_path: String,
    #[cfg(test)]
    test_session_store: Option<Arc<dyn CronStore>>,
    #[cfg(test)]
    test_durable_store: Option<Arc<dyn CronStore>>,
    #[cfg(test)]
    test_now_ms: Option<u64>,
}

impl ToolCronUpdate {
    pub fn new(config_path: String) -> Self {
        Self {
            config_path,
            #[cfg(test)]
            test_session_store: None,
            #[cfg(test)]
            test_durable_store: None,
            #[cfg(test)]
            test_now_ms: None,
        }
    }

    #[cfg(test)]
    fn with_stores(
        config_path: String,
        session_store: Arc<dyn CronStore>,
        durable_store: Option<Arc<dyn CronStore>>,
        now_ms: u64,
    ) -> Self {
        Self {
            config_path,
            test_session_store: Some(session_store),
            test_durable_store: durable_store,
            test_now_ms: Some(now_ms),
        }
    }

    fn session_store(&self) -> Arc<dyn CronStore> {
        #[cfg(test)]
        if let Some(store) = &self.test_session_store {
            return store.clone();
        }
        session_cron_store()
    }

    async fn durable_store(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
    ) -> Result<Option<Arc<dyn CronStore>>, String> {
        #[cfg(test)]
        if self.test_session_store.is_some() {
            return Ok(self.test_durable_store.clone());
        }
        let gcx = ccx.lock().await.global_context.clone();
        active_durable_cron_store(gcx).await
    }

    fn now_ms(&self) -> u64 {
        #[cfg(test)]
        if let Some(now_ms) = self.test_now_ms {
            return now_ms;
        }
        unix_now_ms()
    }
}

#[async_trait]
impl Tool for ToolCronUpdate {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "cron_update".to_string(),
            display_name: "Update Scheduled Task".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Update, pause, resume, or run a scheduled task by ID.".to_string(),
            input_schema: json!({
                "type":"object",
                "properties": {
                    "id": {"type":"string"},
                    "cron": {"type":"string"},
                    "every": {"type":"string"},
                    "at": {"type":"string"},
                    "tz": {"type":"string"},
                    "prompt": {"type":"string"},
                    "description": {"type":"string"},
                    "enabled": {"type":"boolean"},
                    "run_now": {"type":"boolean"}
                },
                "required": ["id"]
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
        let input = CronUpdateInput::from_args(args)?;
        let session_store = self.session_store();
        let durable_store = self.durable_store(ccx).await?;
        let (mut job, store) = load_job_store(&input.id, session_store, durable_store).await?;
        let now_ms = self.now_ms();

        apply_update(&mut job, input, now_ms)?;
        let human_schedule = human_schedule_for_trigger(&job.trigger);
        if !store.replace(job.clone()).await? {
            return Err(format!("Scheduled task `{}` not found", job.id));
        }
        crate::scheduler::runner_change_notify().notify_waiters();

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(
                    json!({
                        "id": job.id,
                        "updated": true,
                        "human_schedule": human_schedule,
                    })
                    .to_string(),
                ),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                tool_failed: Some(false),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[derive(Debug)]
struct CronUpdateInput {
    id: String,
    cron: Option<String>,
    every: Option<String>,
    at: Option<String>,
    tz: Option<String>,
    prompt: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    run_now: bool,
}

impl CronUpdateInput {
    fn from_args(args: &HashMap<String, Value>) -> Result<Self, String> {
        let id = required_string_arg(args, "id")?;
        let description = optional_string_arg(args, "description")?;
        if let Some(description) = &description {
            if description.chars().count() > 80 {
                return Err("description must be at most 80 characters".to_string());
            }
        }
        Ok(Self {
            id,
            cron: optional_string_arg(args, "cron")?,
            every: optional_string_arg(args, "every")?,
            at: optional_string_arg(args, "at")?,
            tz: optional_string_arg(args, "tz")?,
            prompt: optional_string_arg(args, "prompt")?,
            description,
            enabled: optional_bool_arg(args, "enabled")?,
            run_now: optional_bool_arg(args, "run_now")?.unwrap_or(false),
        })
    }

    fn has_schedule_update(&self) -> bool {
        self.cron.is_some() || self.every.is_some() || self.at.is_some() || self.tz.is_some()
    }
}

async fn load_job_store(
    id: &str,
    session_store: Arc<dyn CronStore>,
    durable_store: Option<Arc<dyn CronStore>>,
) -> Result<(Job, Arc<dyn CronStore>), String> {
    if let Some(job) = session_store.get(id).await {
        return Ok((job, session_store));
    }
    if let Some(store) = durable_store {
        if let Some(job) = store.get(id).await {
            return Ok((job, store));
        }
    }
    Err(format!("Scheduled task `{id}` not found"))
}

fn apply_update(job: &mut Job, input: CronUpdateInput, now_ms: u64) -> Result<(), String> {
    if input.has_schedule_update() {
        job.trigger = parse_schedule_update(&input, now_ms)?;
        if matches!(job.trigger, Trigger::Once { .. }) {
            job.recurring = false;
        }
    }
    if let Some(prompt) = input.prompt {
        set_prompt(job, prompt);
    }
    if let Some(description) = input.description {
        job.description = description;
    }
    if let Some(enabled) = input.enabled {
        job.enabled = enabled;
        job.paused_at_ms = if enabled { None } else { Some(now_ms) };
    }
    if input.run_now {
        job.trigger_at_ms = Some(now_ms);
    }
    Ok(())
}

fn parse_schedule_update(input: &CronUpdateInput, now_ms: u64) -> Result<Trigger, String> {
    parse_schedule(
        input.cron.as_deref(),
        input.every.as_deref(),
        input.at.as_deref(),
        input.tz.as_deref(),
        now_ms,
    )
    .map_err(|error| format!("Invalid schedule: {error}"))
}

fn set_prompt(job: &mut Job, prompt: String) {
    if let crate::scheduler::Action::AgentTurn { prompt: target, .. } = &mut job.action {
        *target = prompt;
    }
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

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::scheduler::{InMemoryCronStore, JsonFileCronStore};

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn test_task(id: &str, durable: bool) -> Job {
        let mut task = Job::new_cron_agent_chat(
            "*/5 * * * *".to_string(),
            "Check".to_string(),
            "Check".to_string(),
            true,
            durable,
            1_000,
        );
        task.id = id.to_string();
        task.set_existing_chat(Some("chat".to_string()));
        task.set_mode(Some("agent".to_string()));
        task
    }

    async fn test_ccx() -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            Vec::new(),
            "cron-update-test".to_string(),
            None,
            "model".to_string(),
            None,
            None,
            None,
        )
        .await;
        Arc::new(AMutex::new(ccx))
    }

    async fn run_tool(
        tool: &mut ToolCronUpdate,
        ccx: Arc<AMutex<AtCommandsContext>>,
        items: &[(&str, Value)],
    ) -> Value {
        let (_, contexts) = tool
            .tool_execute(ccx, &"call".to_string(), &args(items))
            .await
            .unwrap();
        let ContextEnum::ChatMessage(message) = contexts.into_iter().next().unwrap() else {
            panic!("expected chat message")
        };
        let ChatContent::SimpleText(text) = message.content else {
            panic!("expected simple text")
        };
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn cron_update_edits_schedule_to_interval() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(test_task("cron_update_interval", false))
            .await
            .unwrap();
        let mut tool =
            ToolCronUpdate::with_stores(String::new(), session_store.clone(), None, 2_000);
        let ccx = test_ccx().await;

        let result = run_tool(
            &mut tool,
            ccx,
            &[
                ("id", json!("cron_update_interval")),
                ("every", json!("30m")),
            ],
        )
        .await;

        let stored = session_store.get("cron_update_interval").await.unwrap();
        assert_eq!(
            stored.trigger,
            Trigger::Interval {
                every_ms: 30 * 60_000
            }
        );
        assert_eq!(result["human_schedule"], json!("every 30 minutes"));
        assert_eq!(result["updated"], json!(true));
    }

    #[tokio::test]
    async fn cron_update_pauses_and_sets_run_now() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(test_task("cron_update_pause", false))
            .await
            .unwrap();
        let mut tool =
            ToolCronUpdate::with_stores(String::new(), session_store.clone(), None, 5_000);
        let ccx = test_ccx().await;

        run_tool(
            &mut tool,
            ccx,
            &[
                ("id", json!("cron_update_pause")),
                ("enabled", json!(false)),
                ("run_now", json!(true)),
            ],
        )
        .await;

        let stored = session_store.get("cron_update_pause").await.unwrap();
        assert!(!stored.enabled);
        assert_eq!(stored.paused_at_ms, Some(5_000));
        assert_eq!(stored.trigger_at_ms, Some(5_000));
    }

    #[tokio::test]
    async fn cron_update_resumes() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut task = test_task("cron_update_resume", false);
        task.enabled = false;
        task.paused_at_ms = Some(1_000);
        session_store.add(task).await.unwrap();
        let mut tool =
            ToolCronUpdate::with_stores(String::new(), session_store.clone(), None, 5_000);
        let ccx = test_ccx().await;

        run_tool(
            &mut tool,
            ccx,
            &[
                ("id", json!("cron_update_resume")),
                ("enabled", json!(true)),
            ],
        )
        .await;

        let stored = session_store.get("cron_update_resume").await.unwrap();
        assert!(stored.enabled);
        assert_eq!(stored.paused_at_ms, None);
    }

    #[tokio::test]
    async fn cron_update_updates_durable_task() {
        let temp = tempfile::tempdir().unwrap();
        let durable_store: Arc<dyn CronStore> =
            Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        durable_store
            .add(test_task("cron_update_durable", true))
            .await
            .unwrap();
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut tool = ToolCronUpdate::with_stores(
            String::new(),
            session_store,
            Some(durable_store.clone()),
            5_000,
        );
        let ccx = test_ccx().await;

        run_tool(
            &mut tool,
            ccx,
            &[
                ("id", json!("cron_update_durable")),
                ("description", json!("Updated")),
            ],
        )
        .await;

        let stored = durable_store.get("cron_update_durable").await.unwrap();
        assert_eq!(stored.description, "Updated");
    }

    #[tokio::test]
    async fn cron_update_unknown_id_errors() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut tool = ToolCronUpdate::with_stores(String::new(), session_store, None, 5_000);
        let ccx = test_ccx().await;

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("id", json!("missing")), ("every", json!("30m"))]),
            )
            .await
            .unwrap_err();

        assert_eq!(err, "Scheduled task `missing` not found");
    }
}
