use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::trajectories::chat_id_is_planner_for_task;
use crate::tasks::storage;
use crate::tasks::types::BoardReviewPolicy;
use crate::tools::task_tool_helpers::{require_bound_planner_task, required_string};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
};

pub struct ToolTaskBoardAuthority;

impl ToolTaskBoardAuthority {
    pub fn new() -> Self {
        Self
    }
}

fn parse_policy(args: &HashMap<String, Value>) -> Result<BoardReviewPolicy, String> {
    match required_string(args, "review_policy")?.as_str() {
        "owner_allowed" => Ok(BoardReviewPolicy::OwnerAllowed),
        "independent_required" => Ok(BoardReviewPolicy::IndependentRequired),
        other => Err(format!(
            "Invalid review_policy '{other}'; expected owner_allowed or independent_required"
        )),
    }
}

async fn planner_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<(Arc<crate::global_context::GlobalContext>, String, String), String> {
    let task_id = require_bound_planner_task(ccx, args).await?;
    let ccx = ccx.lock().await;
    Ok((ccx.app.gcx.clone(), task_id, ccx.chat_id.clone()))
}

async fn verified_target(
    gcx: Arc<crate::global_context::GlobalContext>,
    task_id: &str,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    let target = required_string(args, "target_chat_id")?;
    if !chat_id_is_planner_for_task(gcx, &target, task_id).await {
        return Err(
            "target_chat_id is not an authenticated planner chat for this task".to_string(),
        );
    }
    Ok(target)
}

#[async_trait]
impl Tool for ToolTaskBoardAuthority {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let action = required_string(args, "action")?;
        let (gcx, task_id, actor) = planner_context(&ccx, args).await?;
        let target = match action.as_str() {
            "add_reviewer" | "remove_reviewer" | "transfer_owner" => {
                Some(verified_target(gcx.clone(), &task_id, args).await?)
            }
            "claim_owner" | "set_review_policy" | "get" | "override_dependencies" => None,
            other => return Err(format!("Unknown board authority action '{other}'")),
        };
        let policy = (action == "set_review_policy")
            .then(|| parse_policy(args))
            .transpose()?;
        if action == "get" {
            let board = storage::load_board(gcx, &task_id).await?;
            let receipt = json!({
                "type": "task_board_authority",
                "action": action,
                "changed": false,
                "task_id": task_id,
                "owner_chat_id": board.owner_chat_id,
                "reviewer_chat_ids": board.reviewer_chat_ids,
                "review_policy": board.review_policy,
                "target_chat_id": target,
                "rev": board.rev,
            });
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(receipt.to_string()),
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }
        if action == "override_dependencies" {
            let card_id = required_string(args, "card_id")?;
            let reason = required_string(args, "reason")?;
            let (board, unsatisfied) =
                storage::override_card_dependencies(gcx, &task_id, &card_id, &actor, &reason)
                    .await?;
            let receipt = json!({
                "type": "task_board_authority",
                "action": action,
                "changed": true,
                "task_id": task_id,
                "card_id": card_id,
                "overridden_by": actor,
                "reason": reason,
                "unsatisfied_dependencies": unsatisfied,
                "rev": board.rev,
            });
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(receipt.to_string()),
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }
        let actor_for_update = actor.clone();
        let target_for_update = target.clone();
        let action_for_update = action.clone();
        let (board, changed) = storage::update_board_atomic(gcx, &task_id, move |board| {
            match action_for_update.as_str() {
                "claim_owner" => board.authorize_owner(&actor_for_update)?,
                "add_reviewer" => {
                    board.require_owner(&actor_for_update)?;
                    let target = target_for_update.as_ref().expect("target validated");
                    if board.owner_chat_id.as_deref() == Some(target.as_str()) {
                        return Err(
                            "task board owner cannot also be an assigned reviewer".to_string()
                        );
                    }
                    if !board.reviewer_chat_ids.contains(target) {
                        board.reviewer_chat_ids.push(target.clone());
                    }
                }
                "remove_reviewer" => {
                    board.require_owner(&actor_for_update)?;
                    board.remove_reviewer(target_for_update.as_deref().expect("target validated"));
                }
                "transfer_owner" => {
                    board.require_owner(&actor_for_update)?;
                    let target = target_for_update.as_ref().expect("target validated");
                    board.remove_reviewer(target);
                    board.owner_chat_id = Some(target.clone());
                    board.clear_verifier_reports();
                }
                "set_review_policy" => {
                    board.require_owner(&actor_for_update)?;
                    board.review_policy = policy.expect("policy validated");
                    board.clear_verifier_reports();
                }
                _ => unreachable!(),
            }
            Ok(true)
        })
        .await?;
        let receipt = json!({
            "type": "task_board_authority",
            "action": action,
            "changed": changed,
            "task_id": task_id,
            "owner_chat_id": board.owner_chat_id,
            "reviewer_chat_ids": board.reviewer_chat_ids,
            "review_policy": board.review_policy,
            "target_chat_id": target,
            "rev": board.rev,
        });
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(receipt.to_string()),
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        Vec::new()
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_board_authority".to_string(),
            display_name: "Task Board Authority".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Claim or transfer task-board ownership, manage authenticated reviewers, set the explicit self-review policy, record an explicit dependency override for one card, or inspect current board authority.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["get", "claim_owner", "add_reviewer", "remove_reviewer", "transfer_owner", "set_review_policy", "override_dependencies"]},
                    "task_id": {"type": "string"},
                    "target_chat_id": {"type": "string"},
                    "review_policy": {"type": "string", "enum": ["owner_allowed", "independent_required"]},
                    "card_id": {"type": "string", "description": "Card whose unsatisfied dependencies are overridden by override_dependencies"},
                    "reason": {"type": "string", "description": "Recorded justification for override_dependencies"}
                },
                "required": ["action"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        if args.get("action").and_then(Value::as_str) == Some("get") {
            Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "inspect task board authority".to_string(),
                rule: "read_only".to_string(),
            })
        } else {
            Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::CONFIRMATION,
                command: "change task board authority".to_string(),
                rule: "explicit_user_consent".to_string(),
            })
        }
    }
}
