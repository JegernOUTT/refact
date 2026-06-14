use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::scheduler::{
    active_durable_cron_store, human_schedule, next_run_ms, scheduler_timezone, session_cron_store,
    CronStore, Job,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolCronList {
    pub config_path: String,
    #[cfg(test)]
    test_session_store: Option<Arc<dyn CronStore>>,
    #[cfg(test)]
    test_durable_store: Option<Arc<dyn CronStore>>,
}

impl ToolCronList {
    pub fn new(config_path: String) -> Self {
        Self {
            config_path,
            #[cfg(test)]
            test_session_store: None,
            #[cfg(test)]
            test_durable_store: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_stores(
        config_path: String,
        session_store: Arc<dyn CronStore>,
        durable_store: Option<Arc<dyn CronStore>>,
    ) -> Self {
        Self {
            config_path,
            test_session_store: Some(session_store),
            test_durable_store: durable_store,
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CronListScope {
    Session,
    Durable,
    All,
}

#[async_trait]
impl Tool for ToolCronList {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let scope = parse_scope(args)?;
        let now_ms = Utc::now().timestamp_millis().max(0) as u64;
        let tz = scheduler_timezone();

        let mut tasks: Vec<Value> = Vec::new();

        if scope == CronListScope::Session || scope == CronListScope::All {
            let session_tasks = self.session_store().list().await;
            tasks.extend(session_tasks.iter().map(|t| task_value(t, now_ms, tz)));
        }

        if scope == CronListScope::Durable || scope == CronListScope::All {
            if let Some(store) = self.durable_store(ccx).await? {
                let durable_tasks = store.list().await;
                tasks.extend(durable_tasks.iter().map(|t| task_value(t, now_ms, tz)));
            }
        }

        tasks.sort_by(|a, b| {
            a["next_fire_at_ms"]
                .as_u64()
                .unwrap_or(0)
                .cmp(&b["next_fire_at_ms"].as_u64().unwrap_or(0))
                .then_with(|| {
                    a["id"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(b["id"].as_str().unwrap_or(""))
                })
        });

        let text = serde_json::to_string_pretty(&tasks).map_err(|e| e.to_string())?;
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(text),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "cron_list".to_string(),
            display_name: "Cron List".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "List scheduled tasks with target chat and mode, optionally filtering by session-only or durable scope."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["session", "durable", "all"],
                        "default": "all"
                    }
                },
                "required": []
            }),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn parse_scope(args: &HashMap<String, Value>) -> Result<CronListScope, String> {
    match args.get("scope") {
        Some(Value::String(scope)) if scope.trim().is_empty() => Ok(CronListScope::All),
        Some(Value::String(scope)) => match scope.trim() {
            "session" => Ok(CronListScope::Session),
            "durable" => Ok(CronListScope::Durable),
            "all" => Ok(CronListScope::All),
            other => Err(format!(
                "Invalid scope `{other}`. Must be one of: session, durable, all"
            )),
        },
        Some(value) => Err(format!("argument `scope` is not a string: {value:?}")),
        None => Ok(CronListScope::All),
    }
}

fn task_value(task: &Job, now_ms: u64, tz: chrono_tz::Tz) -> Value {
    let cron = task.cron_expr().unwrap_or_default();
    json!({
        "id": task.id,
        "cron": cron,
        "human_schedule": human_schedule(cron),
        "description": task.description,
        "prompt": first_chars(task.prompt().unwrap_or_default(), 200),
        "chat_id": task.chat_id(),
        "mode": task.mode(),
        "recurring": task.recurring,
        "durable": task.durable,
        "next_fire_at_ms": next_run_ms(task, now_ms, tz).unwrap_or(0),
        "fire_count": task.fire_count,
        "created_at_ms": task.created_at_ms,
    })
}

fn first_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::scheduler::{
        InMemoryCronStore, JsonFileCronStore, DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
    };

    fn session_task(id: &str) -> Job {
        let mut task = Job::new_cron_agent_chat(
            "*/5 * * * *".to_string(),
            "session prompt".to_string(),
            format!("{id} desc"),
            true,
            false,
            1000,
        );
        task.id = id.to_string();
        task.set_existing_chat(Some("chat".to_string()));
        task.set_mode(Some("agent".to_string()));
        task.auto_expire_after_ms = DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;
        task
    }

    fn durable_task(id: &str) -> Job {
        let mut task = Job::new_cron_agent_chat(
            "0 9 * * *".to_string(),
            "durable prompt".to_string(),
            format!("{id} desc"),
            true,
            true,
            1000,
        );
        task.id = id.to_string();
        task.set_existing_chat(Some("chat".to_string()));
        task.set_mode(Some("agent".to_string()));
        task.auto_expire_after_ms = DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;
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
            "chat".to_string(),
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
        tool: &mut ToolCronList,
        ccx: Arc<AMutex<AtCommandsContext>>,
        scope: Option<&str>,
    ) -> Vec<Value> {
        let mut args = HashMap::new();
        if let Some(s) = scope {
            args.insert("scope".to_string(), json!(s));
        }
        let (_, messages) = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();
        let message = match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(m) => m,
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        };
        let text = match message.content {
            ChatContent::SimpleText(t) => t,
            _ => panic!("expected text content"),
        };
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn cron_list_scope_session_sees_session_store() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(session_task("cron_list_sess_a"))
            .await
            .unwrap();
        let mut tool = ToolCronList::with_stores(String::new(), session_store, None);
        let ccx = test_ccx().await;

        let session_only = run_tool(&mut tool, ccx.clone(), Some("session")).await;
        assert_eq!(session_only.len(), 1);
        assert_eq!(session_only[0]["id"], json!("cron_list_sess_a"));
        assert_eq!(session_only[0]["durable"], json!(false));

        let durable_only = run_tool(&mut tool, ccx.clone(), Some("durable")).await;
        assert_eq!(durable_only.len(), 0);

        let all = run_tool(&mut tool, ccx, None).await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["id"], json!("cron_list_sess_a"));
    }

    #[tokio::test]
    async fn cron_list_scope_all_sees_session_and_durable() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(session_task("cron_list_all_sess"))
            .await
            .unwrap();

        let temp = tempfile::tempdir().unwrap();
        let durable_store: Arc<dyn CronStore> =
            Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        durable_store
            .add(durable_task("cron_list_all_dur"))
            .await
            .unwrap();

        let mut tool = ToolCronList::with_stores(String::new(), session_store, Some(durable_store));
        let ccx = test_ccx().await;

        let all = run_tool(&mut tool, ccx.clone(), None).await;
        assert_eq!(all.len(), 2);
        let ids: Vec<&str> = all.iter().map(|v| v["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"cron_list_all_sess"));
        assert!(ids.contains(&"cron_list_all_dur"));

        let session_only = run_tool(&mut tool, ccx.clone(), Some("session")).await;
        assert_eq!(session_only.len(), 1);
        assert_eq!(session_only[0]["id"], json!("cron_list_all_sess"));

        let durable_only = run_tool(&mut tool, ccx, Some("durable")).await;
        assert_eq!(durable_only.len(), 1);
        assert_eq!(durable_only[0]["id"], json!("cron_list_all_dur"));
    }

    #[tokio::test]
    async fn cron_list_includes_target_chat_and_mode() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(session_task("cron_list_target_fields"))
            .await
            .unwrap();
        let mut tool = ToolCronList::with_stores(String::new(), session_store, None);
        let ccx = test_ccx().await;

        let items = run_tool(&mut tool, ccx, Some("session")).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["chat_id"], json!("chat"));
        assert_eq!(items[0]["mode"], json!("agent"));
    }

    #[tokio::test]
    async fn cron_list_next_fire_uses_scheduler_timezone() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(session_task("cron_list_tz_check"))
            .await
            .unwrap();
        let mut tool = ToolCronList::with_stores(String::new(), session_store, None);
        let ccx = test_ccx().await;

        let items = run_tool(&mut tool, ccx, Some("session")).await;
        assert_eq!(items.len(), 1);

        let now_ms = Utc::now().timestamp_millis().max(0) as u64;
        let tz = scheduler_timezone();
        let expected = next_run_ms("*/5 * * * *", now_ms, tz).unwrap_or(0);
        let actual = items[0]["next_fire_at_ms"].as_u64().unwrap();
        assert!(
            actual > 0,
            "next_fire_at_ms should be positive, got {actual}"
        );
        assert!(
            actual.abs_diff(expected) < 60_000,
            "next_fire_at_ms {actual} should be close to scheduler_timezone-based {expected}"
        );
    }
}
