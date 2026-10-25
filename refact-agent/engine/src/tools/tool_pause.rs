use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolPause {
    pub config_path: String,
}
pub struct ToolResume {
    pub config_path: String,
}

fn description(config_path: &str, resume: bool) -> ToolDesc {
    let mut properties = serde_json::Map::new();
    properties.insert("target".into(), json!({"oneOf":[
        {"type":"object","properties":{"kind":{"const":"goal"}},"required":["kind"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"child"},"agent_id":{"type":"string","minLength":1}},"required":["kind","agent_id"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"card"},"card_id":{"type":"string","minLength":1},"task_id":{"type":"string"}},"required":["kind","card_id"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"schedule"},"id":{"type":"string","minLength":1}},"required":["kind","id"],"additionalProperties":false}
    ]}));
    properties.insert("push".into(), crate::chat::types::PushMode::schema());
    properties.insert(
        if resume { "note" } else { "reason" }.into(),
        json!({"type":"string"}),
    );
    properties.insert("expected_run_generation".into(),json!({"type":"integer","minimum":1,"description":"Required for child controls; current generation from inspect."}));
    if !resume {
        properties.insert("until_ms".into(),json!({"type":"integer","description":"Goal only: absolute Unix milliseconds now + 1 through 1440 minutes; omit for user-resumable indefinite pause."}));
    }
    ToolDesc {
        name: if resume {"resume"} else {"pause"}.into(),
        display_name: if resume {"Resume"} else {"Pause"}.into(),
        source: ToolSource {source_type:ToolSourceType::Builtin,config_path:config_path.into()},
        experimental:false, allow_parallel:false,
        description:"Control a tagged target object. Child controls cooperate within the current generation, never lifecycle recovery. Cards require the bound planner. Goal resume clears only active timed snooze, never manual/safety/terminal holds. Schedule accepts no pause options.".into(),
        input_schema:json!({"type":"object","properties":properties,"required":["target"],"additionalProperties":false}),output_schema:None,annotations:None,
    }
}

async fn execute(
    ccx: Arc<AMutex<AtCommandsContext>>,
    call_id: &String,
    args: &HashMap<String, Value>,
    resume: bool,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let tagged = args
        .get("target")
        .and_then(Value::as_object)
        .ok_or("target must be a tagged object")?;
    let target = tagged
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("target.kind is required")?;
    let target_fields: &[&str] = match target {
        "goal" => &["kind"],
        "child" => &["kind", "agent_id"],
        "card" => &["kind", "card_id", "task_id"],
        "schedule" => &["kind", "id"],
        _ => return Err("target.kind must be goal, child, card, or schedule".into()),
    };
    for (key, value) in tagged {
        if !target_fields.contains(&key.as_str()) {
            return Err(format!("field target.{key} is inapplicable to {target}"));
        }
        if !value.is_string() {
            return Err(format!("target.{key} must be a string"));
        }
    }
    for key in target_fields
        .iter()
        .filter(|key| **key != "kind" && **key != "task_id")
    {
        if tagged
            .get(*key)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!("target.{key} is required"));
        }
    }
    let mut allowed = vec!["target"];
    if target != "schedule" {
        allowed.extend(["push", if resume { "note" } else { "reason" }]);
    }
    if target == "goal" && !resume {
        allowed.push("until_ms");
    }
    if target == "child" {
        allowed.push("expected_run_generation");
    }
    for (key, value) in args {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("field {key} is inapplicable to {target}"));
        }
        if ["reason", "note"].contains(&key.as_str()) && !value.is_string() {
            return Err(format!("{key} must be a string"));
        }
    }
    if target == "child"
        && args
            .get("expected_run_generation")
            .and_then(Value::as_u64)
            .is_none_or(|generation| generation == 0)
    {
        return Err(
            "expected_run_generation is required and must be positive for child controls".into(),
        );
    }
    let mut flattened = args.clone();
    flattened.remove("target");
    flattened.extend(
        tagged
            .iter()
            .filter(|(key, _)| key.as_str() != "kind")
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    let args = &flattened;
    match target {
        "goal" => {
            crate::tools::tool_goal_pursuit_controls::control_goal(ccx, call_id, args, resume).await
        }
        "child" => {
            crate::tools::tool_background_agents::child_pause_resume(ccx, call_id, args, resume)
                .await
        }
        "card" if resume => {
            crate::tools::tool_agent_lifecycle::resume_card(ccx, call_id, args).await
        }
        "card" => crate::tools::tool_agent_lifecycle::pause_card(ccx, call_id, args).await,
        "schedule" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or("id is required")?;
            let value =
                crate::tools::tool_cron_update::set_schedule_enabled(ccx, id, resume).await?;
            Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".into(),
                    content: ChatContent::SimpleText(value.to_string()),
                    tool_call_id: call_id.clone(),
                    ..Default::default()
                })],
            ))
        }
        _ => unreachable!(),
    }
}

#[async_trait]
impl Tool for ToolPause {
    fn tool_description(&self) -> ToolDesc {
        description(&self.config_path, false)
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        execute(ccx, id, args, false).await
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
        control_confirmation(args, false)
    }
}
#[async_trait]
impl Tool for ToolResume {
    fn tool_description(&self) -> ToolDesc {
        description(&self.config_path, true)
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        execute(ccx, id, args, true).await
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
        control_confirmation(args, true)
    }
}

fn control_confirmation(
    args: &HashMap<String, Value>,
    resume: bool,
) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
    use crate::tools::tools_description::{MatchConfirmDeny, MatchConfirmDenyResult};
    let target = args
        .get("target")
        .and_then(Value::as_object)
        .ok_or("target must be a tagged object")?;
    let kind = target
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("target.kind is required")?;
    if !matches!(kind, "goal" | "child" | "card" | "schedule") {
        return Err("target.kind must be goal, child, card, or schedule".into());
    }
    let name = if resume { "resume" } else { "pause" };
    Ok(MatchConfirmDeny {
        result: if kind == "child" {
            MatchConfirmDenyResult::CONFIRMATION
        } else {
            MatchConfirmDenyResult::PASS
        },
        command: format!("{name} {}", Value::Object(target.clone())),
        rule: if kind == "child" { "default" } else { "" }.into(),
    })
}
