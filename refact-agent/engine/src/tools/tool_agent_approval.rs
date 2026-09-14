use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, MatchConfirmDeny, MatchConfirmDenyResult,
};

pub struct ToolAgentApproval {
    pub config_path: String,
}

fn requested<'a>(args: &'a HashMap<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing {name}"))
}

#[async_trait]
impl Tool for ToolAgentApproval {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "agent_approval".into(), display_name: "Agent command approval".into(),
            description: "Inspect or decide a child's exact pending command approval. Actions: list, approve, reject. Approval is scoped to the request, checked against parent permissions, and cannot override hard denials.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{"action":{"type":"string","enum":["list","approve","reject"]},"request_id":{"type":"string"}},"required":["action"]}),
            output_schema: None, annotations: None, experimental: false, allow_parallel: false,
            source: ToolSource { source_type: ToolSourceType::Builtin, config_path: self.config_path.clone() },
        }
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let action = requested(args, "action")?;
        if action != "approve" {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: action.into(),
                rule: String::new(),
            });
        }
        let (app, chat_id) = {
            let context = ccx.lock().await;
            (context.app.clone(), context.chat_id.clone())
        };
        let pending = app
            .agent_approvals
            .get(
                &chat_id,
                requested(args, "request_id")?,
                crate::agents::approval::now_ms(),
            )
            .map_err(|e| format!("approval request: {e:?}"))?;
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or("parent session unavailable")?;
        let thread = session.lock().await.thread.clone();
        let (asks, denials) =
            crate::agents::approval::parent_command_policy(&app, &thread, &pending).await?;
        Ok(MatchConfirmDeny {
            result: if !denials.is_empty() {
                MatchConfirmDenyResult::DENY
            } else if !asks.is_empty() {
                MatchConfirmDenyResult::CONFIRMATION
            } else {
                MatchConfirmDenyResult::PASS
            },
            command: format!(
                "Approve child {} command: {} {}",
                pending.child_work_id, pending.tool_name, pending.args
            ),
            rule: denials
                .first()
                .or(asks.first())
                .map(|r| r.rule.clone())
                .unwrap_or_default(),
        })
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (app, chat_id) = {
            let context = ccx.lock().await;
            (context.app.clone(), context.chat_id.clone())
        };
        let action = requested(args, "action")?;
        let now = crate::agents::approval::now_ms();
        let value = if action == "list" {
            let requests: Vec<_> = app
                .agent_approvals
                .list_for_parent(&chat_id, now)
                .into_iter()
                .map(|request| {
                    serde_json::json!({
                        "request_id": request.id,
                        "agent_id": request.child_work_id,
                        "run_generation": request.run_generation,
                        "created_at": request.created_at,
                        "expires_at": request.expires_at,
                    })
                })
                .collect();
            serde_json::json!({"requests": requests, "details": "Exact command details remain in the privacy-checked approval notice."})
        } else {
            if action != "approve" && action != "reject" {
                return Err("action must be list, approve or reject".into());
            }
            let id = requested(args, "request_id")?;
            let pending = app
                .agent_approvals
                .get(&chat_id, id, now)
                .map_err(|e| format!("approval request: {e:?}"))?;
            let target = crate::agents::approval_target::ApprovalTarget::resolve(
                &app,
                &pending.child_work_id,
            )
            .await?;
            target.check(&pending)?;
            let generation = target.generation;
            if let Ok(decision) = app.agent_approvals.wait_now(id, now) {
                let wanted = if action == "approve" {
                    refact_work_core::ApprovalDecision::Approved
                } else {
                    refact_work_core::ApprovalDecision::Rejected
                };
                if decision == wanted
                    && generation == pending.run_generation
                    && target.parent_id == chat_id
                {
                    return Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".into(), tool_call_id: tool_call_id.clone(),
                        content: ChatContent::SimpleText(serde_json::json!({"request_id":id,"decision":action,"already_decided":true,"scope":"once"}).to_string()),
                        ..Default::default()
                    })]));
                }
            }
            if target.parent_id != chat_id || generation != pending.run_generation {
                return Err("approval request belongs to a stale or unauthorized child run".into());
            }
            if action == "approve" {
                let session = app
                    .chat
                    .sessions
                    .read()
                    .await
                    .get(&chat_id)
                    .cloned()
                    .ok_or("parent session unavailable")?;
                let thread = session.lock().await.thread.clone();
                let (asks, denials) =
                    crate::agents::approval::parent_command_policy(&app, &thread, &pending).await?;
                if !denials.is_empty() {
                    return Err("parent policy denies this command".into());
                }
                if !asks.is_empty() {
                    app.agent_approvals.consume_user_once(
                        &chat_id, id, pending.run_generation, tool_call_id, now,
                    ).map_err(|_| "This command requires explicit user approval; the parent cannot approve beyond its delegated command authority.".to_string())?;
                }
            }
            let decision = app
                .agent_approvals
                .decide(
                    &chat_id,
                    pending.run_generation,
                    id,
                    action == "approve",
                    now,
                )
                .map_err(|e| format!("approval decision: {e:?}"))?;
            serde_json::json!({"request_id":id,"agent_id":pending.child_work_id,"decision":format!("{decision:?}").to_lowercase(),"scope":"once"})
        };
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".into(),
                tool_call_id: tool_call_id.clone(),
                content: ChatContent::SimpleText(value.to_string()),
                ..Default::default()
            })],
        ))
    }
}
