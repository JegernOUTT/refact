use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::teams::{live_chat_ids, TeamRefusal};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolTeamSeat {
    pub config_path: String,
}

fn required_string(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{name}'"))
}

fn optional_string(args: &HashMap<String, Value>, name: &str) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .map(Some)
            .ok_or_else(|| format!("'{name}' must be a nonempty string")),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn claim_outcome_text(outcome: crate::tasks::types::ClaimOutcome) -> &'static str {
    match outcome {
        crate::tasks::types::ClaimOutcome::Claimed => "claimed",
        crate::tasks::types::ClaimOutcome::AlreadyHeld => "already held",
    }
}

#[async_trait]
impl Tool for ToolTeamSeat {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "team_seat".into(),
            display_name: "Team Seat".into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Claim and manage your own seat in a configured team. The member identity is always this chat; claim/reclaim require a role and record_outcome requires an outcome.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "team": {"type": "string", "minLength": 1},
                    "action": {"type": "string", "enum": ["claim", "heartbeat", "release", "reclaim", "record_outcome"]},
                    "role": {"type": "string", "minLength": 1},
                    "idempotency_key": {"type": "string", "minLength": 1},
                    "outcome": {"type": "string", "minLength": 1}
                },
                "required": ["team", "action"]
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
        let team = required_string(args, "team")?;
        let action = required_string(args, "action")?;
        if !matches!(
            action.as_str(),
            "claim" | "heartbeat" | "release" | "reclaim" | "record_outcome"
        ) {
            return Err(
                "action must be claim, heartbeat, release, reclaim or record_outcome".into(),
            );
        }
        let role = required_string(args, "role")?;
        let idempotency_key = if matches!(action.as_str(), "claim" | "reclaim") {
            optional_string(args, "idempotency_key")?
        } else {
            None
        };
        let outcome = if action == "record_outcome" {
            Some(required_string(args, "outcome")?)
        } else {
            None
        };

        let (app, chat_id, model) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.clone(),
                ccx.chat_id.clone(),
                ccx.current_model.clone(),
            )
        };
        let spec = crate::coordination::teams::load_team(app.gcx.clone(), &team).await?;
        app.teams
            .register(spec)
            .map_err(|refusal: TeamRefusal| refusal.to_string())?;
        let caller = crate::coordination::scope::model_caller(&app, &chat_id, &model)
            .await
            .map_err(|error| error.to_string())?;
        let project = crate::coordination::roots::project_identity(&caller.source_root);
        let catalog =
            crate::coordination::catalog::build_catalog(&app, caller.source_root.to_str()).await?;
        let live_ids = live_chat_ids(&catalog.all_rows());
        let now = now_ms();
        let idempotency_key = idempotency_key.unwrap_or_else(|| chat_id.clone());

        let result = app
            .teams
            .with_roster(&project, &team, |_, roster| {
                let action_result = match action.as_str() {
                    "claim" => claim_outcome_text(roster.claim(
                        &role,
                        &chat_id,
                        &idempotency_key,
                        None,
                        now,
                    )?)
                    .to_string(),
                    "heartbeat" => {
                        roster.heartbeat(&role, &chat_id, now)?;
                        "heartbeat recorded".to_string()
                    }
                    "release" => {
                        roster.release(&role, &chat_id, now)?;
                        "released".to_string()
                    }
                    "reclaim" => claim_outcome_text(roster.reclaim(
                        &role,
                        &chat_id,
                        &idempotency_key,
                        None,
                        now,
                    )?)
                    .to_string(),
                    "record_outcome" => {
                        roster.record_outcome(
                            &role,
                            &chat_id,
                            outcome.as_deref().expect("validated outcome"),
                            now,
                        )?;
                        "outcome recorded".to_string()
                    }
                    _ => unreachable!(),
                };
                Ok::<_, crate::coordination::teams::RosterRefusal>((
                    action_result,
                    roster.live_team_members(&live_ids),
                ))
            })
            .map_err(|refusal| refusal.to_string())?
            .map_err(|refusal| refusal.to_string())?;
        let response = json!({
            "team": team,
            "action": action,
            "role": role,
            "member_chat_id": chat_id,
            "result": result.0,
            "outcome": outcome,
            "members": result.1
        });
        let live_members = result.1.len();
        let summary = format!(
            "Team `{team}` role `{role}` {result} for this chat. {live_members} live team member(s).",
            result = result.0
        );
        let serialized = serde_json::to_string(&response).map_err(|error| error.to_string())?;
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: tool_call_id.clone(),
            content: ChatContent::SimpleText(format!("{summary}\n\n{serialized}")),
            preserve: Some(true),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        };
        message.extra.insert("team_seat".into(), response);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        Vec::new()
    }
}
