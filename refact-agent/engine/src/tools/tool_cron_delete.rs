use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::scheduler::{active_durable_cron_store, session_cron_store, CronStore};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolCronDelete {
    pub config_path: String,
    #[cfg(test)]
    test_session_store: Option<Arc<dyn CronStore>>,
    #[cfg(test)]
    test_durable_store: Option<Arc<dyn CronStore>>,
}

impl ToolCronDelete {
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
    fn with_stores(
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

#[async_trait]
impl Tool for ToolCronDelete {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "cron_delete".to_string(),
            display_name: "Delete Scheduled Task".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Cancel a scheduled task by ID.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
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
        let id = match args.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(value) => return Err(format!("argument `id` is not a string: {value:?}")),
            None => return Err("argument `id` is missing".to_string()),
        };

        let mut removed = self.session_store().remove(&id).await?;

        if !removed {
            if let Some(durable) = self.durable_store(ccx).await? {
                removed = durable.remove(&id).await?;
            }
        }

        if removed {
            crate::scheduler::runner::notify_runner_change();
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(json!({ "removed": removed }).to_string()),
                tool_call_id: tool_call_id.clone(),
                tool_failed: Some(false),
                ..Default::default()
            })],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::scheduler::{
        InMemoryCronStore, JsonFileCronStore, ScheduledTask, DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
    };

    fn test_task(id: &str, durable: bool) -> ScheduledTask {
        ScheduledTask {
            id: id.to_string(),
            cron: "*/5 * * * *".to_string(),
            prompt: "Check the build".to_string(),
            description: "Check build".to_string(),
            recurring: true,
            durable,
            created_at_ms: 1000,
            chat_id: Some("chat".to_string()),
            mode: Some("agent".to_string()),
            last_fired_at_ms: None,
            fire_count: 0,
            auto_expire_after_ms: DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
        }
    }

    async fn test_ccx() -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            Vec::new(),
            "cron-delete-test".to_string(),
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
        tool: &mut ToolCronDelete,
        ccx: Arc<AMutex<AtCommandsContext>>,
        id: &str,
    ) -> Value {
        let mut args = HashMap::new();
        args.insert("id".to_string(), json!(id));
        let (_, contexts) = tool
            .tool_execute(ccx, &"call".to_string(), &args)
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
    async fn cron_delete_removes_session_task() {
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        session_store
            .add(test_task("cron_del_sess", false))
            .await
            .unwrap();
        let mut tool = ToolCronDelete::with_stores(String::new(), session_store.clone(), None);
        let ccx = test_ccx().await;

        let result = run_tool(&mut tool, ccx, "cron_del_sess").await;

        assert_eq!(result, json!({ "removed": true }));
        assert!(session_store.list().await.is_empty());
    }

    #[tokio::test]
    async fn cron_delete_removes_durable_task() {
        let temp = tempfile::tempdir().unwrap();
        let durable_store: Arc<dyn CronStore> =
            Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        durable_store
            .add(test_task("cron_del_dur", true))
            .await
            .unwrap();
        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut tool =
            ToolCronDelete::with_stores(String::new(), session_store, Some(durable_store.clone()));
        let ccx = test_ccx().await;

        let result = run_tool(&mut tool, ccx, "cron_del_dur").await;

        assert_eq!(result, json!({ "removed": true }));
        assert!(durable_store.list().await.is_empty());
    }

    #[tokio::test]
    async fn cron_delete_uses_same_durable_path_as_list() {
        use crate::tools::tool_cron_list::ToolCronList;

        let temp = tempfile::tempdir().unwrap();
        let durable_store: Arc<dyn CronStore> =
            Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        durable_store
            .add(test_task("cron_shared_path", true))
            .await
            .unwrap();

        let session_store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut delete_tool = ToolCronDelete::with_stores(
            String::new(),
            session_store.clone(),
            Some(durable_store.clone()),
        );
        let mut list_tool =
            ToolCronList::with_stores(String::new(), session_store, Some(durable_store));

        let ccx = test_ccx().await;

        let before = {
            let args = HashMap::new();
            let (_, msgs) = list_tool
                .tool_execute(ccx.clone(), &"call".to_string(), &args)
                .await
                .unwrap();
            let ContextEnum::ChatMessage(m) = msgs.into_iter().next().unwrap() else {
                panic!()
            };
            let ChatContent::SimpleText(t) = m.content else {
                panic!()
            };
            serde_json::from_str::<Vec<Value>>(&t).unwrap()
        };
        assert_eq!(before.len(), 1);
        assert_eq!(before[0]["id"], json!("cron_shared_path"));

        let delete_result = run_tool(&mut delete_tool, ccx.clone(), "cron_shared_path").await;
        assert_eq!(delete_result, json!({ "removed": true }));

        let after = {
            let args = HashMap::new();
            let (_, msgs) = list_tool
                .tool_execute(ccx, &"call".to_string(), &args)
                .await
                .unwrap();
            let ContextEnum::ChatMessage(m) = msgs.into_iter().next().unwrap() else {
                panic!()
            };
            let ChatContent::SimpleText(t) = m.content else {
                panic!()
            };
            serde_json::from_str::<Vec<Value>>(&t).unwrap()
        };
        assert!(after.is_empty());
    }
}
