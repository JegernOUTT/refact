use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::{Tool, ToolDesc};

pub struct ToolSchedule {
    pub config_path: String,
}

impl ToolSchedule {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

#[async_trait]
impl Tool for ToolSchedule {
    fn tool_description(&self) -> ToolDesc {
        let mut desc = super::tool_cron_create::create_description(self.config_path.clone());
        desc.name = "schedule".into();
        desc.display_name = "Schedule".into();
        desc.description = format!("Create, list, update or delete scheduled jobs using action. Create requires description and exactly one schedule and execution action. Update/delete require id. Update supports cron/every/at/tz, prompt, description, push and run_now. List supports scope. Use pause/resume to enable or disable jobs. {}", desc.description);
        let props = desc.input_schema["properties"].as_object_mut().unwrap();
        props.insert(
            "action".into(),
            json!({"type":"string","enum":["create","list","update","delete"]}),
        );
        props.insert("id".into(), json!({"type":"string"}));
        props.insert(
            "scope".into(),
            json!({"type":"string","enum":["session","durable","all"],"default":"all"}),
        );
        props.insert("run_now".into(), json!({"type":"boolean"}));
        desc.input_schema["required"] = json!(["action"]);
        desc.input_schema["additionalProperties"] = json!(false);
        desc
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action must be create, list, update, or delete")?;
        let allowed: &[&str] = match action {
            "create" => &[
                "action",
                "cron",
                "every",
                "at",
                "tz",
                "trigger",
                "hook_id",
                "prompt",
                "command",
                "command_argv",
                "cwd",
                "timeout_secs",
                "delivery",
                "push",
                "recurring",
                "durable",
                "isolated",
                "description",
            ],
            "list" => &["action", "scope"],
            "update" => &[
                "action",
                "id",
                "cron",
                "every",
                "at",
                "tz",
                "prompt",
                "description",
                "run_now",
                "push",
            ],
            "delete" => &["action", "id"],
            _ => return Err("action must be create, list, update, or delete".into()),
        };
        for key in args.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(format!(
                    "argument `{key}` is not supported for schedule {action}"
                ));
            }
        }
        match action {
            "create" => super::tool_cron_create::execute_create(ccx, tool_call_id, args).await,
            "list" => super::tool_cron_list::execute_list(ccx, tool_call_id, args).await,
            "update" => super::tool_cron_update::execute_update(ccx, tool_call_id, args).await,
            "delete" => super::tool_cron_delete::execute_delete(ccx, tool_call_id, args).await,
            _ => unreachable!(),
        }
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}
