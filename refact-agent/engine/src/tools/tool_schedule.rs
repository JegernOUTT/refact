use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::{Tool, ToolDesc};

async fn execute_automation(
    ccx: Arc<AMutex<AtCommandsContext>>,
    call_id: &str,
    args: &HashMap<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    use crate::scheduler::automation_api as api;
    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .ok_or("operation required")?;
    if matches!(operation, "approve" | "approve_endpoint" | "approve_file") {
        let value =
            crate::scheduler::automation_approval::execute_automation_approval(&ccx, call_id, args)
                .await?;
        return Ok((
            false,
            vec![ContextEnum::ChatMessage(
                crate::call_validation::ChatMessage {
                    role: "tool".into(),
                    tool_call_id: call_id.into(),
                    content: crate::call_validation::ChatContent::SimpleText(value.to_string()),
                    ..Default::default()
                },
            )],
        ));
    }
    let mut request = serde_json::to_value(args).map_err(|e| e.to_string())?;
    let map = request.as_object_mut().ok_or("object required")?;
    map.remove("action");
    map.remove("operation");
    let id = map
        .remove("id")
        .map(|v| v.as_str().map(str::to_owned).ok_or("id must be string"))
        .transpose()?;
    let (app, chat_id, model, scope) = {
        let c = ccx.lock().await;
        (
            c.app.clone(),
            c.chat_id.clone(),
            c.current_model.clone(),
            c.execution_scope.clone(),
        )
    };
    let caller = crate::apps::Caller::Tool {
        chat_id: chat_id.clone(),
        model: Some(model.clone()),
    };
    // Reuse the App project's caller/privacy accessor, never process-global active root.
    let project = crate::apps::runtime(&app, &caller).await?;
    if let Some(scope) = scope {
        if tokio::fs::canonicalize(scope.effective_root())
            .await
            .map_err(|e| e.to_string())?
            != project.root
        {
            return Err("automation checkout scope mismatch".into());
        }
    }
    let mut records = crate::apps::preflight_tool_records(&app, &caller).await?;
    crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    records.extend(crate::privacy::records::declared_file_records(
        &app.gcx,
        vec![project.root.join(".refact/automations.json")],
    )?);
    let caller_scope = crate::coordination::scope::model_caller(&app, &chat_id, &model)
        .await
        .map_err(|e| e.to_string())?;
    let mut preflight = crate::call_validation::ChatMessage::default();
    crate::privacy::records::merge_records(&mut preflight, records.clone());
    crate::privacy::destinations::clear_for_model(
        &app.gcx,
        Audit(vec![preflight]),
        caller_scope.model.as_ref().ok_or("model unavailable")?,
    )
    .map_err(|e| e.model_facing())?;
    let rt =
        crate::scheduler::generic_runtime::runtime(app.gcx.clone(), project.root.clone()).await?;
    let required_id = || id.as_deref().ok_or("id required".to_string());
    let value = match operation {
        "list" | "status" => {
            if !map.is_empty() || (operation == "list" && id.is_some()) {
                return Err("unsupported automation read fields".into());
            }
            api::list(
                &rt,
                if operation == "status" {
                    Some(required_id()?)
                } else {
                    None
                },
            )
            .await?
        }
        "create" | "replace" => {
            if operation == "create" && id.is_some() {
                return Err("create identity belongs in definition".into());
            }
            api::save(
                &rt,
                if operation == "replace" {
                    Some(required_id()?)
                } else {
                    None
                },
                serde_json::from_value(request).map_err(|e| e.to_string())?,
            )
            .await?
        }
        "delete" => {
            api::delete(
                &rt,
                required_id()?,
                serde_json::from_value(request).map_err(|e| e.to_string())?,
            )
            .await?
        }
        "run_now" | "revoke" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ExactControl {
                expected_catalog_revision: u64,
                expected_revision: u64,
            }
            let exact: ExactControl = serde_json::from_value(request).map_err(|e| e.to_string())?;
            if operation == "run_now" {
                rt.run_exact(
                    required_id()?,
                    exact.expected_catalog_revision,
                    exact.expected_revision,
                )
                .await?
            } else {
                rt.revoke_exact(
                    required_id()?,
                    exact.expected_catalog_revision,
                    exact.expected_revision,
                )
                .await?;
                json!({"id":required_id()?,"operation":"revoke"})
            }
        }
        "enable" | "disable" => {
            request["operation"] = json!(operation);
            api::control(
                &rt,
                required_id()?,
                serde_json::from_value(request).map_err(|e| e.to_string())?,
            )
            .await?
        }
        _ => {
            return Err(
                "unsupported automation operation; approval is not caller-controlled".into(),
            )
        }
    };
    let mut message = crate::call_validation::ChatMessage {
        role: "tool".into(),
        tool_call_id: call_id.into(),
        content: crate::call_validation::ChatContent::SimpleText(value.to_string()),
        ..Default::default()
    };
    crate::privacy::records::merge_records(&mut message, records);
    struct Audit(Vec<crate::call_validation::ChatMessage>);
    impl refact_privacy::PrivacyAudited for Audit {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            refact_privacy::records_from_messages(&self.0)
        }
    }
    let caller_scope = crate::coordination::scope::model_caller(&app, &chat_id, &model)
        .await
        .map_err(|e| e.to_string())?;
    crate::privacy::destinations::clear_for_model(
        &app.gcx,
        Audit(vec![message.clone()]),
        caller_scope.model.as_ref().ok_or("model unavailable")?,
    )
    .map_err(|e| e.model_facing())?;
    Ok((false, vec![ContextEnum::ChatMessage(message)]))
}

pub struct ToolSchedule {
    pub config_path: String,
}

impl ToolSchedule {
    pub fn new(config_path: String) -> Self {
        Self {
            config_path,
        }
    }
}

#[async_trait]
impl Tool for ToolSchedule {
    fn tool_description(&self) -> ToolDesc {
        let mut desc = super::tool_cron_create::create_description(self.config_path.clone());
        desc.name = "schedule".into();
        desc.display_name = "Schedule".into();
        desc.description = format!("Create, list, update or delete scheduled jobs using action. Create requires description and exactly one schedule and execution action. Update/delete require id. Update supports cron/every/at/tz, prompt, description, push and run_now. List supports scope. For unified project definitions use action=automation; legacy job operations remain unchanged. {}", desc.description);
        let props = desc.input_schema["properties"].as_object_mut().unwrap();
        props.insert(
            "action".into(),
            json!({"type":"string","enum":["create","list","update","delete","automation"]}),
        );
        props.insert("id".into(), json!({"type":"string"}));
        props.insert(
            "scope".into(),
            json!({"type":"string","enum":["session","durable","all"],"default":"all"}),
        );
        props.insert("run_now".into(), json!({"type":"boolean"}));
        props.insert("operation".into(), json!({"type":"string","enum":["list","status","create","replace","delete","enable","disable","run_now","revoke","approve","approve_endpoint","approve_file"]}));
        props.insert("file_grant".into(), json!({
            "type":"object","additionalProperties":false,"required":["path","rotation"],
            "properties":{"path":{"type":"string","minLength":1,"maxLength":4096},"rotation":{"type":"string","enum":["stop","follow"]}},
            "description":"Exact project-relative file and rotation policy; required only for approve_file and bound to actual human confirmation."
        }));
        props.insert("endpoint_grant".into(), json!({
            "type":"object", "additionalProperties":false,
            "required":["origin","allowed_private_ips"],
            "properties":{
                "origin":{"type":"string","minLength":1,"maxLength":2048,"description":"Exact canonical collector origin, e.g. http://127.0.0.1:9000. No path, credentials, or wildcard."},
                "allowed_private_ips":{"type":"array","minItems":1,"maxItems":64,"uniqueItems":true,"items":{"type":"string","minLength":2,"maxLength":45},"description":"Exact IP addresses authorized by the human; no CIDR or wildcard."}
            },
            "description":"Required only for approve_endpoint; always requires separate actual human confirmation of all exact arguments. First approve the definition digest. JSON alone never grants private network access."
        }));
        props.insert(
            "expected_catalog_revision".into(),
            json!({"type":"integer","minimum":0}),
        );
        props.insert(
            "expected_revision".into(),
            json!({"type":"integer","minimum":1}),
        );
        props.insert("security_digest".into(), json!({"type":"string","description":"Exact digest from status; required for operation=approve or approve_endpoint, both always require actual human confirmation."}));
        props.insert("definition".into(), json!({"type":"object","description":"Canonical schema_version=1 project Definition. Never include approved or secrets. App-owned definitions must use app_source."}));
        desc.description.push_str(" action=automation uses operation=list|status|create|replace|delete|enable|disable|run_now|revoke|approve|approve_endpoint|approve_file against canonical project definitions. Writes require catalog CAS and replacement/delete/control require definition CAS. New security digests require separate trusted human approval; desired enabled is not approval.");
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
        if action == "automation" {
            return execute_automation(ccx, tool_call_id, args).await;
        }
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

#[cfg(test)]
mod automation_schema_tests {
    use super::*;
    #[test]
    fn unified_schedule_remains_strict_root_object_and_legacy_actions_survive() {
        let schema = ToolSchedule::new(String::new())
            .tool_description()
            .input_schema;
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema.get("oneOf").is_none());
        for action in ["create", "list", "update", "delete", "automation"] {
            assert!(schema["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(action)));
        }
        assert!(schema["properties"].get("approved").is_none());
        assert!(schema["properties"].get("secret").is_none());
        assert!(schema["properties"].get("security_digest").is_some());
        for operation in ["approve", "approve_endpoint", "run_now", "revoke"] {
            assert!(schema["properties"]["operation"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(operation)));
        }
        let grant = &schema["properties"]["endpoint_grant"];
        assert_eq!(grant["type"], "object");
        assert_eq!(grant["additionalProperties"], false);
        assert_eq!(grant["required"], json!(["origin", "allowed_private_ips"]));
        assert_eq!(grant["properties"]["origin"]["type"], "string");
        assert_eq!(grant["properties"]["allowed_private_ips"]["type"], "array");
        assert_eq!(grant["properties"]["allowed_private_ips"]["minItems"], 1);
        assert_eq!(grant["properties"]["allowed_private_ips"]["maxItems"], 64);
        assert_eq!(
            grant["properties"]["allowed_private_ips"]["items"]["type"],
            "string"
        );
    }
}
