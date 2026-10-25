use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::teams::{live_chat_ids, RoleAssignment, TeamRefusal};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolTeamSeat {
    pub config_path: String,
}

/// Delegates the assigner's existing peer authority toward a recipient to a seated team member.
pub struct ToolTeamRoleGrant {
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
        let (catalog, _) = crate::coordination::service::visible_projection(&app, &caller)
            .await
            .map_err(|error| error.to_string())?;
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

fn peer_authority(
    value: Option<&str>,
) -> Result<crate::coordination::peer_authority::PeerAuthority, String> {
    match value {
        Some("metadata") => Ok(crate::coordination::peer_authority::PeerAuthority::Metadata),
        Some("mail") => Ok(crate::coordination::peer_authority::PeerAuthority::Mail),
        Some("control") => Ok(crate::coordination::peer_authority::PeerAuthority::Control),
        _ => Err("authority must be metadata, mail or control".to_string()),
    }
}

fn address(
    args: &HashMap<String, Value>,
    agent_key: &str,
    chat_key: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let has_agent_id = args.contains_key(agent_key);
    let has_chat_id = args.contains_key(chat_key);
    if has_agent_id == has_chat_id {
        return Err(format!(
            "exactly one of '{agent_key}' or '{chat_key}' is required"
        ));
    }
    let agent_id = has_agent_id
        .then(|| required_string(args, agent_key))
        .transpose()?;
    let chat_id = has_chat_id
        .then(|| required_string(args, chat_key))
        .transpose()?;
    Ok((agent_id, chat_id))
}

pub(super) fn root_for_address(
    rows: &[crate::coordination::types::WorkRow],
    agent_id: Option<&str>,
    chat_id: Option<&str>,
    label: &str,
) -> Result<(String, Option<String>), String> {
    let row = match (agent_id, chat_id) {
        (Some(agent_id), None) => rows.iter().find(|row| {
            row.key.runtime == refact_work_core::RuntimeKind::BackgroundAgent
                && row.key.id == agent_id
        }),
        (None, Some(chat_id)) => rows
            .iter()
            .find(|row| row.chat_id.as_deref() == Some(chat_id)),
        _ => None,
    }
    .ok_or_else(|| format!("{label} is not visible in this project"))?;
    let root = match row.key.runtime {
        refact_work_core::RuntimeKind::BackgroundAgent => row
            .root_chat_id
            .as_deref()
            .or(row.parent_chat_id.as_deref()),
        refact_work_core::RuntimeKind::Trajectory => {
            row.root_chat_id.as_deref().or(row.chat_id.as_deref())
        }
    }
    .ok_or_else(|| format!("{label} is not visible in this project"))?
    .to_string();
    let chat = chat_id.map(str::to_string).or_else(|| row.chat_id.clone());
    Ok((root, chat))
}

pub(super) fn require_live_seat(
    roster: &crate::coordination::teams::TeamRoster,
    role: &str,
    member_chat: &str,
    live_chat_ids: &std::collections::BTreeSet<String>,
    now_ms: u64,
) -> Result<(), String> {
    if roster.has_live_seat(role, member_chat, live_chat_ids, now_ms) {
        Ok(())
    } else {
        Err(format!("member does not hold {role} in {}", roster.team()))
    }
}

#[async_trait]
impl Tool for ToolTeamRoleGrant {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "team_role_grant".into(),
            display_name: "Team Role Grant".into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Grant a seated team member time-bounded peer authority toward a visible same-project recipient. The role ceiling and the assigner's existing authority both limit the grant; user confirmation is required.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "team": {"type": "string", "minLength": 1},
                    "role": {"type": "string", "minLength": 1},
                    "member_agent_id": {"type": "string", "minLength": 1, "description": "Provide exactly one of member_agent_id or member_chat_id."},
                    "member_chat_id": {"type": "string", "minLength": 1, "description": "Provide exactly one of member_agent_id or member_chat_id."},
                    "recipient_agent_id": {"type": "string", "minLength": 1, "description": "Provide exactly one of recipient_agent_id or recipient_chat_id."},
                    "recipient_chat_id": {"type": "string", "minLength": 1, "description": "Provide exactly one of recipient_agent_id or recipient_chat_id."},
                    "authority": {"type": "string", "enum": ["metadata", "mail", "control"]}
                },
                "required": ["team", "role", "authority"]
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
        let role = required_string(args, "role")?;
        let authority = peer_authority(args.get("authority").and_then(Value::as_str))?;
        let (member_agent_id, member_chat_id) = address(args, "member_agent_id", "member_chat_id")?;
        let (recipient_agent_id, recipient_chat_id) =
            address(args, "recipient_agent_id", "recipient_chat_id")?;
        let (app, chat_id, model) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.clone(),
                ccx.chat_id.clone(),
                ccx.current_model.clone(),
            )
        };
        let assigner_root =
            crate::tools::tool_agent_interact::require_top_level_root(&*ccx.lock().await)?;
        let caller = crate::coordination::scope::model_caller(&app, &chat_id, &model)
            .await
            .map_err(|error| error.to_string())?;
        let project = crate::coordination::roots::project_identity(&caller.source_root);
        let team_spec = crate::coordination::teams::load_team(app.gcx.clone(), &team).await?;
        app.teams
            .register(team_spec)
            .map_err(|refusal: TeamRefusal| refusal.to_string())?;
        let (catalog, _) = crate::coordination::service::visible_projection(&app, &caller)
            .await
            .map_err(|_| "member or recipient is not reachable for this grant".to_string())?;
        let rows = catalog.all_rows();
        let live_ids = live_chat_ids(&rows);
        let now = now_ms();
        let (member_root, member_chat) = root_for_address(
            &rows,
            member_agent_id.as_deref(),
            member_chat_id.as_deref(),
            "member",
        )
        .map_err(|_| "member or recipient is not reachable for this grant".to_string())?;
        let member_chat = member_chat
            .ok_or_else(|| "member or recipient is not reachable for this grant".to_string())?;
        let (recipient_root, _) = root_for_address(
            &rows,
            recipient_agent_id.as_deref(),
            recipient_chat_id.as_deref(),
            "recipient",
        )
        .map_err(|_| "member or recipient is not reachable for this grant".to_string())?;
        let role_spec = app
            .teams
            .with_roster(&project, &team, |spec, roster| {
                let role_spec = spec
                    .role(&role)
                    .ok_or_else(|| format!("team `{team}` does not define role `{role}`"))?;
                require_live_seat(roster, &role, &member_chat, &live_ids, now)?;
                Ok::<_, String>(role_spec.clone())
            })
            .map_err(|refusal| refusal.to_string())??;
        // The assigner delegates an existing grant toward the chosen recipient to the seated member.
        let assignment = RoleAssignment {
            team: &team,
            project: &project,
            assigner_root: &assigner_root,
            member_root: &member_root,
            recipient_root: &recipient_root,
        };
        let proposal = crate::coordination::teams::propose_role_grant(
            &app.peer_grants,
            &role_spec,
            &assignment,
            authority,
            now,
        )
        .map_err(|refusal| match refusal {
            crate::coordination::teams::RoleRefusal::AssignerLacksAuthority { .. } => {
                "member or recipient is not reachable for this grant".to_string()
            }
            _ => refusal.to_string(),
        })?;
        crate::coordination::teams::apply_role_grant(&app.peer_grants, proposal, now)?;
        let response = json!({
            "team": team,
            "role": role,
            "member_agent_id": member_agent_id,
            "member_chat_id": member_chat_id,
            "recipient_agent_id": recipient_agent_id,
            "recipient_chat_id": recipient_chat_id,
            "authority": authority,
        });
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: tool_call_id.clone(),
            content: ChatContent::SimpleText(format!(
                "Granted {authority:?} authority through `{team}` role `{role}`."
            )),
            preserve: Some(true),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        };
        message.extra.insert("team_role_grant".into(), response);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
        Ok(crate::tools::tools_description::MatchConfirmDeny {
            result: crate::tools::tools_description::MatchConfirmDenyResult::CONFIRMATION,
            command: "grant team role peer authority".to_string(),
            rule: "explicit_user_consent".to_string(),
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        Vec::new()
    }
}
