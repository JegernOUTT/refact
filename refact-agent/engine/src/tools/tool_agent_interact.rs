use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

#[cfg(test)]
use crate::agents::registry::InboxMessage;
use refact_core::chat_types::{PendingDelivery, PushMode};
use crate::agents::types::{BackgroundAgent, BgAgentStatus};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolAgentsOverview {
    pub config_path: String,
}

pub struct ToolAgentMessage {
    pub config_path: String,
}

pub struct ToolAgentPeerGrant {
    pub config_path: String,
}

pub struct ToolProgressReport {
    pub config_path: String,
}

pub struct ToolAgentMailbox {
    pub config_path: String,
}

fn tool_desc(
    config_path: String,
    name: &str,
    display_name: &str,
    description: &str,
    schema: Value,
) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: ToolSource {
            source_type: ToolSourceType::Builtin,
            config_path,
        },
        experimental: false,
        allow_parallel: true,
        description: description.to_string(),
        input_schema: schema,
        output_schema: None,
        annotations: None,
    }
}

fn output(tool_call_id: &str, text: String) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text),
            tool_call_id: tool_call_id.to_string(),
            preserve: Some(true),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        })],
    )
}

fn required_string(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{name}'"))
}

fn optional_bool(args: &HashMap<String, Value>, name: &str) -> Result<bool, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(value) => {
            refact_tool_api::coerce_bool(value).ok_or_else(|| format!("{name} must be a boolean"))
        }
    }
}

fn optional_string(args: &HashMap<String, Value>, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> (AppState, String, String, Option<String>) {
    let ccx = ccx.lock().await;
    (
        ccx.app.clone(),
        ccx.chat_id.clone(),
        ccx.root_chat_id.clone(),
        ccx.background_agent_id.clone(),
    )
}

fn include_overview_record(record: &BackgroundAgent) -> bool {
    !record.status.is_terminal()
        || record
            .finished_at
            .map(|finished_at| finished_at >= Utc::now() - Duration::hours(1))
            .unwrap_or(false)
}

pub async fn records_for_root(app: &AppState, root_chat_id: &str) -> Vec<(BackgroundAgent, usize)> {
    let all = app.agents.list_all().await;
    let mut by_parent: HashMap<&str, Vec<&BackgroundAgent>> = HashMap::new();
    for record in &all {
        by_parent
            .entry(record.parent_chat_id.as_str())
            .or_default()
            .push(record);
    }
    for records in by_parent.values_mut() {
        records.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.agent_id.cmp(&b.agent_id))
        });
    }
    let mut pending = VecDeque::from([(root_chat_id.to_string(), 0usize)]);
    let mut seen_chats = HashSet::new();
    let mut seen_agents = HashSet::new();
    let mut result = Vec::new();
    while let Some((parent_chat_id, depth)) = pending.pop_front() {
        if !seen_chats.insert(parent_chat_id.clone()) {
            continue;
        }
        for record in by_parent.get(parent_chat_id.as_str()).into_iter().flatten() {
            if !seen_agents.insert(record.agent_id.clone()) {
                continue;
            }
            if let Some(child_chat_id) = &record.child_chat_id {
                pending.push_back((child_chat_id.clone(), depth + 1));
            }
            result.push(((*record).clone(), depth));
        }
    }
    result
}

/// One refusal for every cross-root denial. An agent that does not exist, an
/// agent whose project scope is unknown, and an agent that exists but was
/// never granted mail all read identically, so the error cannot be used to
/// discover a peer the caller is not allowed to know about.
fn peer_mail_refusal(agent_id: &str) -> String {
    format!(
        "Background agent '{agent_id}' is not reachable from this chat. Cross-root mail requires an explicit same-project mail grant; discovery grants no messaging or control authority."
    )
}

/// `Ok(_, None)` is an ordinary delivery inside the caller's own agent tree.
/// `Ok(_, Some(stamp))` is consented cross-root peer mail, and the stamp is
/// what the delivery layer re-checks before the message lands.
async fn can_message_agent(
    app: &AppState,
    caller_chat_id: &str,
    root_chat_id: &str,
    caller_agent_id: Option<&str>,
    target_agent_id: &str,
) -> Result<
    (
        BackgroundAgent,
        Option<refact_core::chat_types::PeerMailStamp>,
    ),
    String,
> {
    let Ok(target) = app.agents.get_any(target_agent_id).await else {
        return Err(peer_mail_refusal(target_agent_id));
    };
    if target.parent_chat_id == caller_chat_id {
        return Ok((target, None));
    }
    if let Some(caller_agent_id) = caller_agent_id {
        if app
            .agents
            .list_descendants(caller_agent_id)
            .await
            .iter()
            .any(|record| record.agent_id == target_agent_id)
        {
            return Ok((target, None));
        }
    } else {
        let in_root_tree = app.agents.list_all().await.iter().any(|record| {
            record.agent_id == target_agent_id
                && (record.parent_chat_id == root_chat_id
                    || record.parent_root_chat_id.as_deref() == Some(root_chat_id))
        });
        if in_root_tree {
            return Ok((target, None));
        }
    }
    let target_root = target
        .parent_root_chat_id
        .as_deref()
        .unwrap_or(target.parent_chat_id.as_str())
        .to_string();
    if target_root == root_chat_id {
        return Err("You may only message your direct children or descendants.".to_string());
    }
    let Some(source_root) = app.agent_executions.saved_scope(target_agent_id) else {
        return Err(peer_mail_refusal(target_agent_id));
    };
    let project = crate::coordination::roots::project_identity(&source_root);
    if app.peer_grants.allows(
        &project,
        root_chat_id,
        &target_root,
        crate::coordination::peer_authority::PeerAuthority::Mail,
        delivery_now_ms(),
    ) {
        return Ok((
            target,
            Some(refact_core::chat_types::PeerMailStamp {
                project,
                sender_root: root_chat_id.to_string(),
                recipient_root: target_root,
            }),
        ));
    }
    Err(peer_mail_refusal(target_agent_id))
}

fn delivery_now_ms() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

/// One sentence per outcome, so a receipt never claims a message reached an
/// agent when the delivery layer dropped it. Acceptance for transport only —
/// delivery is at-least-once with logical dedup, never proof of consumption.
fn delivery_outcome_text(to: &str, outcome: refact_chat_api::DeliveryOutcome) -> String {
    use refact_chat_api::DeliveryOutcome;
    match outcome {
        DeliveryOutcome::Queued => format!("Message queued for {to}."),
        DeliveryOutcome::Delivered => format!("Message delivered to {to}."),
        DeliveryOutcome::Duplicate => format!(
            "Message NOT queued for {to}: the delivery layer already accepted this message id."
        ),
        DeliveryOutcome::Expired => {
            format!("Message NOT queued for {to}: its ttl elapsed before it could be accepted.")
        }
        DeliveryOutcome::Superseded => {
            format!("Message NOT queued for {to}: a newer message on the same stream replaced it.")
        }
    }
}

#[async_trait]
impl Tool for ToolAgentPeerGrant {
    fn tool_description(&self) -> ToolDesc {
        let settings = refact_work_core::coordination_settings();
        let mut desc = tool_desc(
            self.config_path.clone(),
            "agent_peer_grant",
            "Agent Peer Grant",
            "Grant or revoke time-bounded, directional same-project peer metadata, mail, or control authority. Discovery alone grants none. Control includes mail and metadata; mail includes metadata. User confirmation is required.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "recipient_root": {"type": "string", "minLength": 1},
                    "authority": {"type": "string", "enum": ["metadata", "mail", "control"]},
                    "ttl_secs": {"type": "integer", "minimum": 1, "maximum": settings.peer_grant_max_ttl_secs},
                    "revoke": {"type": "boolean", "default": false}
                },
                "required": ["recipient_root"]
            }),
        );
        desc.allow_parallel = false;
        desc
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let recipient_root = required_string(args, "recipient_root")?;
        let revoke = optional_bool(args, "revoke")?;
        let (app, chat_id, sender_root, caller_agent_id) = context(&ccx).await;
        if caller_agent_id.is_some() {
            return Err("Only a top-level root can change peer authority.".to_string());
        }
        if recipient_root == sender_root {
            return Err("recipient_root must identify a different root".to_string());
        }
        let current_model = ccx.lock().await.current_model.clone();
        let caller = crate::coordination::scope::model_caller(&app, &chat_id, &current_model)
            .await
            .map_err(|error| error.to_string())?;
        let project = crate::coordination::roots::project_identity(&caller.source_root);
        let exists = crate::coordination::catalog::build_catalog(&app, caller.source_root.to_str())
            .await?
            .all_rows()
            .iter()
            .any(|row| row.root_chat_id.as_deref() == Some(recipient_root.as_str()));
        if !exists {
            return Err("recipient root is not visible in this project".to_string());
        }
        if revoke {
            let removed = app
                .peer_grants
                .revoke(&project, &sender_root, &recipient_root);
            return Ok(output(
                tool_call_id,
                format!(
                    "Peer grant {}.",
                    if removed {
                        "revoked"
                    } else {
                        "was not present"
                    }
                ),
            ));
        }
        let authority = match args.get("authority").and_then(Value::as_str) {
            Some("metadata") => crate::coordination::peer_authority::PeerAuthority::Metadata,
            Some("mail") => crate::coordination::peer_authority::PeerAuthority::Mail,
            Some("control") => crate::coordination::peer_authority::PeerAuthority::Control,
            _ => return Err("authority must be metadata, mail or control".to_string()),
        };
        let settings = refact_work_core::coordination_settings();
        let ttl_secs = match args.get("ttl_secs") {
            None | Some(Value::Null) => settings.peer_grant_default_ttl_secs,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| "ttl_secs must be an integer".to_string())?,
        };
        if !(1..=settings.peer_grant_max_ttl_secs).contains(&ttl_secs) {
            return Err(format!(
                "ttl_secs must be between 1 and {}",
                settings.peer_grant_max_ttl_secs
            ));
        }
        app.peer_grants.grant(
            crate::coordination::peer_authority::PeerGrant {
                project,
                sender_root,
                recipient_root,
                authority,
                granted_by: format!("chat:{chat_id}"),
                expires_at_ms: delivery_now_ms().saturating_add(ttl_secs.saturating_mul(1000)),
            },
            delivery_now_ms(),
        )?;
        Ok(output(
            tool_call_id,
            format!("Directional peer {authority:?} authority granted for {ttl_secs}s."),
        ))
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
        Ok(crate::tools::tools_description::MatchConfirmDeny {
            result: crate::tools::tools_description::MatchConfirmDenyResult::CONFIRMATION,
            command: "change peer authority".to_string(),
            rule: "explicit_user_consent".to_string(),
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn short_id(agent_id: &str) -> &str {
    agent_id.get(..agent_id.len().min(16)).unwrap_or(agent_id)
}

fn status_chip(status: BgAgentStatus) -> &'static str {
    match status {
        BgAgentStatus::Queued => "⏳ queued",
        BgAgentStatus::Running => "🟢 running",
        BgAgentStatus::WaitingForApproval => "✋ waiting",
        BgAgentStatus::Completed => "✅ completed",
        BgAgentStatus::Failed => "❌ failed",
        BgAgentStatus::Cancelled => "⏹ cancelled",
        BgAgentStatus::Interrupted => "⚠ interrupted",
        BgAgentStatus::Closed => "🔒 closed",
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!(
        "{}…",
        text.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

fn overview_line(record: &BackgroundAgent, depth: usize) -> String {
    let targets = if record.target_files.is_empty() {
        "-".to_string()
    } else {
        truncate(&record.target_files.join(", "), 80)
    };
    let questions = record
        .questions
        .iter()
        .filter(|question| question.answer.is_none())
        .count();
    let model = record
        .model_type
        .as_deref()
        .map(|model_type| format!("{model_type} ({})", record.model))
        .unwrap_or_else(|| record.model.clone());
    let cost = record
        .cost_usd
        .map(|cost| format!(", ${cost:.4}"))
        .unwrap_or_default();
    format!(
        "{}- {} — {} — {} — step {} — tool: {} — targets: {} — branch: {} — merge: {} — model: {} — tokens: {}{} — questions: {}",
        "  ".repeat(depth),
        short_id(&record.agent_id),
        record.title,
        status_chip(record.status),
        record.step_count,
        record.current_tool.as_deref().unwrap_or("-"),
        targets,
        record.worktree_branch.as_deref().unwrap_or("-"),
        record.merge_status.as_deref().unwrap_or("-"),
        model,
        record.tokens_used,
        cost,
        questions,
    )
}

#[async_trait]
impl Tool for ToolAgentsOverview {
    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            self.config_path.clone(),
            "agents_overview",
            "Agents Overview",
            "Show the active background-agent tree rooted at this chat, including recent finished agents, their work, and pending questions.",
            json!({"type": "object", "properties": {}, "required": []}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (app, _chat_id, root_chat_id, agent_id) = context(&ccx).await;
        let records = records_for_root(&app, &root_chat_id)
            .await
            .into_iter()
            .filter(|(record, _)| include_overview_record(record))
            .collect::<Vec<_>>();
        let mut text = String::from("# Agents Overview\n\n");
        if let Some(agent_id) = agent_id {
            text.push_str(&format!("You are agent {agent_id}.\n\n"));
        }
        if records.is_empty() {
            text.push_str("No active or recently finished agents in this tree.");
        } else {
            for (record, depth) in records {
                text.push_str(&overview_line(&record, depth));
                text.push('\n');
            }
        }
        Ok(output(tool_call_id, text))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolAgentMessage {
    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            self.config_path.clone(),
            "agent_message",
            "Agent Message",
            "Send a message to a child or descendant agent, or use to=parent to ask or notify the parent agent.",
            json!({
                "type": "object",
                "properties": {
                    "to": {"type": "string", "description": "A child/descendant agent id, or parent."},
                    "text": {"type": "string", "minLength": 1, "maxLength": refact_work_core::coordination_settings().agent_message_max_chars, "description": "Message text."},
                    "expects_reply": {"type": "boolean", "description": "When messaging parent, create a tracked question."},
                    "push": PushMode::schema(),
                    "reply_to": {"type": "string", "description": "Question id being answered when messaging a child."},
                    "ttl_secs": {"type": "integer", "minimum": 1, "maximum": refact_work_core::coordination_settings().delivery_max_ttl_secs, "description": "Drop the message as expired if the recipient reaches no boundary within this many seconds; omitted uses the configured default, and expired messages never land late."}
                },
                "required": ["to", "text"]
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let to = required_string(args, "to")?;
        let text = required_string(args, "text")?;
        let expects_reply = optional_bool(args, "expects_reply")?;
        let push = PushMode::from_args(args)?;
        let reply_to = optional_string(args, "reply_to");
        let (app, chat_id, root_chat_id, caller_agent_id) = context(&ccx).await;
        let settings = refact_work_core::coordination_settings();
        if text.chars().count() > settings.agent_message_max_chars {
            return Err(format!(
                "argument `text` supports at most {} characters",
                settings.agent_message_max_chars
            ));
        }
        let ttl_secs = match args.get("ttl_secs") {
            None | Some(Value::Null) => settings.delivery_default_ttl_secs,
            Some(value) => value
                .as_u64()
                .filter(|secs| (1..=settings.delivery_max_ttl_secs).contains(secs))
                .ok_or_else(|| {
                    format!(
                        "argument `ttl_secs` must be an integer between 1 and {}",
                        settings.delivery_max_ttl_secs
                    )
                })?,
        };

        if to == "parent" {
            let Some(agent_id) = caller_agent_id else {
                return Err(
                    "You are not a subagent; use an agent id to message a child.".to_string(),
                );
            };
            let caller = app.agents.get_any(&agent_id).await?;
            let parent_notice_chat_id = caller.parent_chat_id.clone();
            if expects_reply {
                let (updated, question_id) =
                    app.agents.add_question(&agent_id, text.clone()).await?;
                crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
                {
                    crate::agents::push::push_notice_to_chat_with_mode(
                        app,
                        &parent_notice_chat_id,
                        format!(
                            "[subagent question] agent {agent_id} asks: {text} — answer with agent_message(to=\"{agent_id}\", reply_to=\"{question_id}\", text=...)"
                        ),
                        push,
                    )
                    .await?;
                }
                return Ok(output(
                    tool_call_id,
                    format!("Question {question_id} sent to parent."),
                ));
            }
            crate::agents::push::push_notice_to_chat_with_mode(
                app.clone(),
                &parent_notice_chat_id,
                format!("[subagent note] agent {agent_id}: {text}"),
                push,
            )
            .await?;
            return Ok(output(tool_call_id, "Note sent to parent.".to_string()));
        }

        let (target, peer_mail) = can_message_agent(
            &app,
            &chat_id,
            &root_chat_id,
            caller_agent_id.as_deref(),
            &to,
        )
        .await?;
        if peer_mail.is_some() {
            if push == PushMode::Preempt {
                return Err("Peer mail is informational: a mail grant carries no authority to interrupt another root. Use push=append or push=when_idle.".to_string());
            }
            if reply_to.is_some() {
                return Err("Peer mail cannot answer another root's question: a mail grant carries no control over that agent.".to_string());
            }
        }
        if let Some(question_id) = reply_to.as_deref() {
            let updated = app
                .agents
                .answer_question(&to, &question_id, text.clone())
                .await?;
            crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
            if target.status.is_terminal() {
                return Ok(output(
                    tool_call_id,
                    format!("Answer recorded for finished agent {to}."),
                ));
            }
        }
        let message_text = reply_to
            .as_deref()
            .map(|question_id| format!("Answer to {question_id}: {text}"))
            .unwrap_or(text);
        let from = if peer_mail.is_some() {
            "peer"
        } else {
            "parent"
        };
        let mut delivery = PendingDelivery::new(
            vec![crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::SystemNotice,
                "agents.message",
                json!({"from": from}),
                format!("[message from {from}]\n{message_text}"),
            )],
            push,
            "agents.message".to_string(),
            // Peer mail is informational: it never wakes another root's agent.
            peer_mail.is_none(),
        );
        delivery.peer_mail = peer_mail;
        delivery.set_ttl_secs(ttl_secs);
        let delivery_id = delivery.id.clone();
        let outcome = crate::agents::delivery::deliver_to_agent(app, &to, delivery).await?;
        let text = delivery_outcome_text(&to, outcome);
        let mut result = output(tool_call_id, text);
        if let Some(ContextEnum::ChatMessage(message)) = result.1.first_mut() {
            message.extra.insert(
                "delivery_receipt".into(),
                json!({
                    "kind": "agent_message",
                    "delivery_id": delivery_id,
                    "outcome": outcome,
                    "accepted": matches!(
                        outcome,
                        refact_chat_api::DeliveryOutcome::Queued
                            | refact_chat_api::DeliveryOutcome::Delivered
                    ),
                    "ttl_secs": ttl_secs,
                    "push": push,
                }),
            );
        }
        Ok(result)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolProgressReport {
    fn tool_description(&self) -> ToolDesc {
        tool_desc(
            self.config_path.clone(),
            "progress_report",
            "Progress Report",
            "Publish a concise progress update for this background agent.",
            json!({
                "type": "object",
                "properties": {"status_line": {"type": "string"}},
                "required": ["status_line"]
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let status_line = required_string(args, "status_line")?;
        let (app, _chat_id, _root_chat_id, agent_id) = context(&ccx).await;
        let Some(agent_id) = agent_id else {
            return Err(
                "You are not a subagent; progress_report is only available to background agents."
                    .to_string(),
            );
        };
        let updated = app
            .agents
            .update_activity(&agent_id, Some(status_line.clone()), None, None)
            .await?;
        crate::agents::spawn::emit_background_agent_update(app, &updated).await;
        Ok(output(
            tool_call_id,
            format!("Progress updated: {status_line}"),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

struct MailboxAudit(Vec<(usize, refact_privacy::FileRecord)>);

impl refact_privacy::PrivacyAudited for MailboxAudit {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        Ok(self.0.clone())
    }
}

struct MailboxRow {
    id: String,
    status: refact_chat_api::DeliveryStatus,
    delivery: Option<PendingDelivery>,
}

impl MailboxRow {
    fn value(&self) -> Value {
        let mut row = json!({"delivery_id": self.id, "status": self.status});
        if let Some(delivery) = &self.delivery {
            row["source"] = json!(delivery.source);
            row["push"] = json!(delivery.push);
            row["from_peer"] = json!(delivery.peer_mail.is_some());
            row["enqueued_at_ms"] = json!(delivery.enqueued_at_ms);
            row["expires_at_ms"] = json!(delivery.expires_at_ms);
            row["preview"] = json!(truncate(&delivery.preview_text(), 200));
        }
        row
    }

    fn line(&self) -> String {
        match &self.delivery {
            Some(delivery) => format!(
                "- {} — {:?} — source: {} — push: {} — from: {} — {}",
                short_id(&self.id),
                self.status,
                delivery.source,
                delivery.push.as_str(),
                if delivery.peer_mail.is_some() {
                    "peer"
                } else {
                    "own tree"
                },
                truncate(&delivery.preview_text(), 120),
            ),
            None => format!("- {} — {:?}", short_id(&self.id), self.status),
        }
    }
}

/// A row is shown only when every privacy record its payload carries is
/// allowed to reach the model that is reading the mailbox. A refused row is
/// omitted outright, and totals are counted after filtering, so neither the
/// row nor its existence is disclosed.
fn mailbox_row_permitted(
    gcx: &std::sync::Arc<crate::global_context::GlobalContext>,
    model: &crate::caps::BaseModelRecord,
    delivery: &PendingDelivery,
) -> bool {
    let Ok(records) = refact_privacy::records_from_messages(&delivery.messages) else {
        return false;
    };
    crate::privacy::destinations::clear_for_model(gcx, MailboxAudit(records), model).is_ok()
}

async fn mailbox_rows(app: &AppState, chat_id: &str, agent_id: Option<&str>) -> Vec<MailboxRow> {
    let (pending, dropped) = match agent_id {
        Some(agent_id) => match app.agents.get_any(agent_id).await {
            Ok(record) => (
                record.pending_deliveries,
                record.dropped_deliveries.into_iter().collect::<Vec<_>>(),
            ),
            Err(_) => (Vec::new(), Vec::new()),
        },
        None => {
            let session = app.chat.sessions.read().await.get(chat_id).cloned();
            match session {
                Some(session) => {
                    let session = session.lock().await;
                    (
                        session.pending_deliveries.iter().cloned().collect(),
                        session.dropped_deliveries.iter().cloned().collect(),
                    )
                }
                None => (Vec::new(), Vec::new()),
            }
        }
    };
    let mut rows: Vec<MailboxRow> = pending
        .into_iter()
        .map(|delivery| MailboxRow {
            id: delivery.id.clone(),
            status: refact_chat_api::DeliveryStatus::Pending,
            delivery: Some(delivery),
        })
        .collect();
    rows.extend(dropped.into_iter().map(|record| MailboxRow {
        id: record.id,
        status: record.status,
        delivery: None,
    }));
    rows
}

#[async_trait]
impl Tool for ToolAgentMailbox {
    fn tool_description(&self) -> ToolDesc {
        let settings = refact_work_core::coordination_settings();
        tool_desc(
            self.config_path.clone(),
            "agent_mailbox",
            "Agent Mailbox",
            "List your own pending and recently finished message deliveries with their truthful status. Shows only your mailbox; it never lists another root's messages.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": settings.mailbox_max_rows, "description": "Rows per page."},
                    "offset": {"type": "integer", "minimum": 0, "description": "Rows to skip; use it with the reported total to page."}
                },
                "required": []
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let settings = refact_work_core::coordination_settings();
        let limit = match args.get("limit") {
            None | Some(Value::Null) => settings.mailbox_default_rows,
            Some(value) => value
                .as_u64()
                .map(|limit| limit as usize)
                .filter(|limit| (1..=settings.mailbox_max_rows).contains(limit))
                .ok_or_else(|| {
                    format!(
                        "argument `limit` must be an integer between 1 and {}",
                        settings.mailbox_max_rows
                    )
                })?,
        };
        let offset = match args.get("offset") {
            None | Some(Value::Null) => 0usize,
            Some(value) => value
                .as_u64()
                .map(|offset| offset as usize)
                .ok_or_else(|| "argument `offset` must be a non-negative integer".to_string())?,
        };
        let (app, chat_id, _root_chat_id, agent_id) = context(&ccx).await;
        // The destination is the model that will read this listing; an unknown
        // or uncompilable policy is rejected by `policy_key` rather than
        // defaulting to disclosure.
        let model = crate::caps::BaseModelRecord {
            id: ccx.lock().await.current_model.clone(),
            ..Default::default()
        };
        let before = crate::coordination::privacy_guard::policy_key(&app.gcx, &model)
            .map_err(|_| "mailbox privacy policy is unavailable".to_string())?;
        let rows = mailbox_rows(&app, &chat_id, agent_id.as_deref())
            .await
            .into_iter()
            .filter(|row| match &row.delivery {
                Some(delivery) => mailbox_row_permitted(&app.gcx, &model, delivery),
                None => true,
            })
            .collect::<Vec<_>>();
        if crate::coordination::privacy_guard::policy_key(&app.gcx, &model)
            .map_err(|_| "mailbox privacy policy is unavailable".to_string())?
            != before
        {
            return Err("mailbox privacy policy is unavailable".to_string());
        }
        let total = rows.len();
        let page = rows.iter().skip(offset).take(limit).collect::<Vec<_>>();
        let mut text = format!(
            "# Mailbox\n\nshowing {} of {total} (offset {offset})\n\n",
            page.len()
        );
        if page.is_empty() {
            text.push_str("No deliveries.");
        } else {
            for row in &page {
                text.push_str(&row.line());
                text.push('\n');
            }
        }
        let mut result = output(tool_call_id, text);
        if let Some(ContextEnum::ChatMessage(message)) = result.1.first_mut() {
            message.extra.insert(
                "mailbox".into(),
                json!({
                    "total": total,
                    "offset": offset,
                    "limit": limit,
                    "next_offset": (offset + page.len() < total).then_some(offset + page.len()),
                    "rows": page.iter().map(|row| row.value()).collect::<Vec<_>>(),
                }),
            );
        }
        Ok(result)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::Ordering;

    use crate::agents::types::{BgAgentKind, CreateAgentRequest};
    use crate::chat::types::ChatSession;
    use crate::tools::tools_description::Tool;

    fn request(parent_chat_id: &str, title: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: Some(parent_chat_id.to_string()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "subagent".to_string(),
            title: title.to_string(),
            prompt: title.to_string(),
            target_files: vec![format!("src/{title}.rs")],
            model: "test/model".to_string(),
            model_type: Some("light".to_string()),
            goal_summary: Some("ship it".to_string()),
            plan_present: true,
            worktree_id: None,
            worktree_branch: Some(format!("branch/{title}")),
        }
    }

    async fn test_context(
        chat_id: &str,
        root_chat_id: &str,
        background_agent_id: Option<String>,
    ) -> (AppState, Arc<AMutex<AtCommandsContext>>) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                Some(root_chat_id.to_string()),
                "test/model".to_string(),
                None,
                None,
            )
            .await,
        ));
        ccx.lock().await.background_agent_id = background_agent_id;
        (app, ccx)
    }

    async fn create(app: &AppState, parent_chat_id: &str, title: &str) -> BackgroundAgent {
        app.agents
            .create(request(parent_chat_id, title))
            .await
            .unwrap()
            .0
    }

    async fn parent_delivery_fixture(
        app: &AppState,
        chat_id: &str,
    ) -> (tempfile::TempDir, Arc<AMutex<ChatSession>>) {
        let workspace = tempfile::tempdir().unwrap();
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let session = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        session
            .lock()
            .await
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session.clone());
        (workspace, session)
    }

    async fn assert_parent_notice_persisted(app: &AppState, chat_id: &str, text: &str) {
        let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
            .await
            .expect("parent notice trajectory persisted");
        assert!(saved
            .messages
            .iter()
            .any(|message| message.content.content_text_only().contains(text)));
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn receipt_of(result: &(bool, Vec<ContextEnum>)) -> Value {
        match &result.1[0] {
            ContextEnum::ChatMessage(message) => message.extra["delivery_receipt"].clone(),
            _ => panic!("expected tool message"),
        }
    }

    fn text(result: (bool, Vec<ContextEnum>)) -> String {
        let ContextEnum::ChatMessage(message) = result.1.into_iter().next().unwrap() else {
            panic!("expected tool message")
        };
        let ChatContent::SimpleText(text) = message.content else {
            panic!("expected text")
        };
        text
    }

    #[tokio::test]
    async fn overview_renders_recursive_tree_and_caller_identity() {
        let (app, root_ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        app.agents
            .mark_running(&child.agent_id, "child-chat".to_string())
            .await
            .unwrap();
        let grandchild = create(&app, "child-chat", "grandchild").await;
        app.agents
            .update_activity(
                &grandchild.agent_id,
                Some("checking files".to_string()),
                Some(2),
                Some(Some("cat".to_string())),
            )
            .await
            .unwrap();
        let child_ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "child-chat".to_string(),
                Some("root".to_string()),
                "test/model".to_string(),
                None,
                None,
            )
            .await,
        ));
        child_ccx.lock().await.background_agent_id = Some(child.agent_id.clone());

        let mut tool = ToolAgentsOverview {
            config_path: String::new(),
        };
        let root_output = text(
            tool.tool_execute(root_ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );
        assert!(root_output.contains(short_id(&child.agent_id)));
        assert!(root_output.contains(short_id(&grandchild.agent_id)));
        assert!(root_output.contains("tool: cat"));

        let child_output = text(
            tool.tool_execute(child_ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );
        assert!(child_output.contains(&format!("You are agent {}", child.agent_id)));
    }

    #[tokio::test]
    async fn child_messages_preserve_all_push_modes() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        for mode in [PushMode::Append, PushMode::Preempt, PushMode::WhenIdle] {
            tool.tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(&[
                    ("to", json!(child.agent_id)),
                    ("text", json!("hello")),
                    ("push", json!(mode)),
                ]),
            )
            .await
            .unwrap();
        }
        let record = app.agents.get_any(&child.agent_id).await.unwrap();
        assert_eq!(
            record
                .pending_deliveries
                .iter()
                .map(|d| d.push)
                .collect::<Vec<_>>(),
            vec![PushMode::Append, PushMode::Preempt, PushMode::WhenIdle]
        );
    }

    #[tokio::test]
    async fn agent_message_validates_ttl_against_settings_bounds() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let settings = refact_work_core::coordination_settings();
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        for bad in [
            json!(0),
            json!(settings.delivery_max_ttl_secs + 1),
            json!(-1),
            json!("forever"),
        ] {
            let error = tool
                .tool_execute(
                    ccx.clone(),
                    &"bad".to_string(),
                    &args(&[
                        ("to", json!(child.agent_id)),
                        ("text", json!("hello")),
                        ("ttl_secs", bad.clone()),
                    ]),
                )
                .await
                .unwrap_err();
            assert!(error.contains("ttl_secs"), "{bad}: {error}");
        }
        assert!(app
            .agents
            .get_any(&child.agent_id)
            .await
            .unwrap()
            .pending_deliveries
            .is_empty());

        tool.tool_execute(
            ccx,
            &"ok".to_string(),
            &args(&[
                ("to", json!(child.agent_id)),
                ("text", json!("hello")),
                ("ttl_secs", json!(settings.delivery_max_ttl_secs)),
            ]),
        )
        .await
        .unwrap();
        let pending = app
            .agents
            .get_any(&child.agent_id)
            .await
            .unwrap()
            .pending_deliveries;
        assert_eq!(
            pending[0].expires_at_ms,
            Some(PendingDelivery::deadline_ms(
                pending[0].enqueued_at_ms,
                settings.delivery_max_ttl_secs
            ))
        );
    }

    #[tokio::test]
    async fn agent_message_receipt_never_claims_a_dropped_message_was_queued() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        let request = args(&[("to", json!(child.agent_id)), ("text", json!("hello"))]);

        let accepted = tool
            .tool_execute(ccx.clone(), &"first".to_string(), &request)
            .await
            .unwrap();
        let receipt = receipt_of(&accepted);
        assert_eq!(receipt["outcome"], "queued");
        assert_eq!(receipt["accepted"], true);
        assert!(text(accepted).contains("Message queued"));

        let delivery_id = receipt["delivery_id"].as_str().unwrap().to_string();
        app.agents
            .record_delivery_status(
                &child.agent_id,
                &delivery_id,
                refact_chat_api::DeliveryStatus::Expired,
            )
            .await
            .unwrap();
        assert_eq!(
            app.agents
                .delivery_status(&child.agent_id, &delivery_id)
                .await,
            refact_chat_api::DeliveryStatus::Expired
        );

        let mut replay = PendingDelivery::with_id(
            delivery_id.clone(),
            vec![crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::SystemNotice,
                "agents.message",
                json!({}),
                "replay".to_string(),
            )],
            PushMode::Append,
            "agents.message".to_string(),
            true,
        );
        replay.set_ttl_secs(60);
        assert_eq!(
            crate::agents::delivery::deliver_to_agent(app.clone(), &child.agent_id, replay)
                .await
                .unwrap(),
            refact_chat_api::DeliveryOutcome::Duplicate,
            "a terminal delivery id must never be silently reused"
        );

        for (outcome, expected) in [
            (refact_chat_api::DeliveryOutcome::Duplicate, "NOT queued"),
            (refact_chat_api::DeliveryOutcome::Expired, "NOT queued"),
            (refact_chat_api::DeliveryOutcome::Superseded, "NOT queued"),
            (refact_chat_api::DeliveryOutcome::Queued, "queued for"),
            (refact_chat_api::DeliveryOutcome::Delivered, "delivered to"),
        ] {
            assert!(
                delivery_outcome_text("child", outcome).contains(expected),
                "{outcome:?}"
            );
        }
    }

    #[tokio::test]
    async fn agent_message_rejects_oversized_text_before_queueing_anything() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let settings = refact_work_core::coordination_settings();
        let error = ToolAgentMessage {
            config_path: String::new(),
        }
        .tool_execute(
            ccx,
            &"oversized".to_string(),
            &args(&[
                ("to", json!(child.agent_id)),
                (
                    "text",
                    json!("x".repeat(settings.agent_message_max_chars + 1)),
                ),
            ]),
        )
        .await
        .unwrap_err();
        assert!(error.contains("characters"), "{error}");
        assert!(app
            .agents
            .get_any(&child.agent_id)
            .await
            .unwrap()
            .pending_deliveries
            .is_empty());
    }

    async fn agent_ccx(
        app: &AppState,
        chat_id: &str,
        model: &str,
        agent_id: Option<String>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                Some("root".to_string()),
                model.to_string(),
                None,
                None,
            )
            .await,
        ));
        ccx.lock().await.background_agent_id = agent_id;
        ccx
    }

    fn mailbox_of(result: &(bool, Vec<ContextEnum>)) -> Value {
        match &result.1[0] {
            ContextEnum::ChatMessage(message) => message.extra["mailbox"].clone(),
            _ => panic!("expected tool message"),
        }
    }

    #[tokio::test]
    async fn mailbox_lists_the_callers_own_deliveries_bounded_and_paged() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let mut message = ToolAgentMessage {
            config_path: String::new(),
        };
        for index in 0..5 {
            message
                .tool_execute(
                    ccx.clone(),
                    &format!("m{index}"),
                    &args(&[
                        ("to", json!(child.agent_id)),
                        ("text", json!(format!("note {index}"))),
                    ]),
                )
                .await
                .unwrap();
        }
        let child_ccx = agent_ccx(
            &app,
            "child-chat",
            "test/model",
            Some(child.agent_id.clone()),
        )
        .await;
        let mut mailbox = ToolAgentMailbox {
            config_path: String::new(),
        };

        let settings = refact_work_core::coordination_settings();
        for bad in [json!(0), json!(settings.mailbox_max_rows + 1), json!("all")] {
            assert!(mailbox
                .tool_execute(
                    child_ccx.clone(),
                    &"bad".to_string(),
                    &args(&[("limit", bad.clone())]),
                )
                .await
                .unwrap_err()
                .contains("limit"));
        }

        let first = mailbox
            .tool_execute(
                child_ccx.clone(),
                &"first".to_string(),
                &args(&[("limit", json!(2))]),
            )
            .await
            .unwrap();
        let page = mailbox_of(&first);
        assert_eq!(page["total"], 5);
        assert_eq!(page["rows"].as_array().unwrap().len(), 2);
        assert_eq!(page["next_offset"], 2);
        assert_eq!(page["rows"][0]["status"], "pending");
        assert_eq!(page["rows"][0]["source"], "agents.message");
        assert_eq!(page["rows"][0]["from_peer"], false);
        assert!(page["rows"][0]["preview"]
            .as_str()
            .unwrap()
            .contains("note 0"));

        let last = mailbox_of(
            &mailbox
                .tool_execute(
                    child_ccx.clone(),
                    &"last".to_string(),
                    &args(&[("limit", json!(2)), ("offset", json!(4))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(last["rows"].as_array().unwrap().len(), 1);
        assert_eq!(last["next_offset"], Value::Null);

        let other = agent_ccx(&app, "other-chat", "test/model", None).await;
        let empty = mailbox_of(
            &mailbox
                .tool_execute(other, &"other".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );
        assert_eq!(
            empty["total"], 0,
            "a mailbox lists only the caller's own deliveries"
        );
    }

    /// A row the current model may not receive is omitted outright, and the
    /// reported total counts only what was disclosed, so the hidden row leaks
    /// neither content nor its own existence. Switching models re-evaluates it.
    #[tokio::test]
    async fn mailbox_rows_are_filtered_per_model_without_leaking_a_count() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        app.gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "guarded".into(),
                    patterns: vec!["**/secret/**".into()],
                    send_to: vec!["trusted".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        ToolAgentMessage {
            config_path: String::new(),
        }
        .tool_execute(
            ccx,
            &"plain".to_string(),
            &args(&[("to", json!(child.agent_id)), ("text", json!("plain note"))]),
        )
        .await
        .unwrap();

        let mut guarded_message = ChatMessage::new("user".into(), "guarded note".into());
        crate::privacy::records::merge_records(
            &mut guarded_message,
            [refact_privacy::FileRecord {
                path: "/secret/a".into(),
                zone: "guarded".into(),
                attribution: refact_privacy::Attribution::Declared,
            }],
        );
        crate::agents::delivery::deliver_to_agent(
            app.clone(),
            &child.agent_id,
            PendingDelivery::with_id(
                "guarded-delivery",
                vec![guarded_message],
                PushMode::Append,
                "agents.message".to_string(),
                false,
            ),
        )
        .await
        .unwrap();

        let mut mailbox = ToolAgentMailbox {
            config_path: String::new(),
        };
        for (model, expected_total, expect_guarded) in
            [("trusted/model", 2, true), ("other/model", 1, false)]
        {
            let caller = agent_ccx(&app, "child-chat", model, Some(child.agent_id.clone())).await;
            let result = mailbox
                .tool_execute(caller, &model.to_string(), &HashMap::new())
                .await
                .unwrap();
            let page = mailbox_of(&result);
            assert_eq!(page["total"], expected_total, "{model}");
            assert_eq!(
                page["rows"].as_array().unwrap().len(),
                expected_total,
                "{model}"
            );
            let rendered = text(result);
            assert_eq!(
                rendered.contains("guarded-delivery") || rendered.contains("guarded note"),
                expect_guarded,
                "{model}"
            );
            assert!(!rendered.contains("hidden"), "{model}: {rendered}");
        }

        app.gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "broken".into(),
                    patterns: vec!["[".into()],
                    send_to: vec!["*".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        let caller = agent_ccx(
            &app,
            "child-chat",
            "trusted/model",
            Some(child.agent_id.clone()),
        )
        .await;
        assert!(mailbox
            .tool_execute(caller, &"broken".to_string(), &HashMap::new())
            .await
            .unwrap_err()
            .contains("privacy policy is unavailable"));
    }

    #[tokio::test]
    async fn parent_message_rejects_agent_outside_its_tree() {
        let (app, ccx) = test_context("root", "root", None).await;
        let outsider = create(&app, "other", "outsider").await;
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        let error = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("to", json!(outsider.agent_id)), ("text", json!("hello"))]),
            )
            .await
            .unwrap_err();
        assert_eq!(error, peer_mail_refusal(&outsider.agent_id));
    }

    /// Every denial reads the same, so the error cannot be used to learn that
    /// a peer exists, what it is called, or which root it belongs to.
    #[tokio::test]
    async fn a_refused_peer_message_reveals_nothing_about_the_peer() {
        let (app, ccx) = test_context("root-a", "root-a", None).await;
        let hidden = create(&app, "root-b", "secret-peer-title").await;
        app.agent_executions
            .remember_scope(&hidden.agent_id, std::env::temp_dir());
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        let mut errors = Vec::new();
        for target in [
            hidden.agent_id.clone(),
            "00000000-0000-0000-0000-000000000000".to_string(),
        ] {
            let error = tool
                .tool_execute(
                    ccx.clone(),
                    &"denied".to_string(),
                    &args(&[("to", json!(target)), ("text", json!("hello"))]),
                )
                .await
                .unwrap_err();
            assert_eq!(error, peer_mail_refusal(&target));
            errors.push(error);
        }
        let without_id = errors
            .iter()
            .map(|error| {
                error
                    .replace(&hidden.agent_id, "")
                    .replace("00000000-0000-0000-0000-000000000000", "")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            without_id[0], without_id[1],
            "an existing peer and an unknown id must be indistinguishable"
        );
        for error in &errors {
            assert!(!error.contains("secret-peer-title"), "{error}");
            assert!(!error.contains("root-b"), "{error}");
        }
    }

    /// Mail is informational: it carries no authority to interrupt the other
    /// root, to answer its questions, or to wake it.
    #[tokio::test]
    async fn a_mail_grant_conveys_no_control_over_the_peer() {
        let (app, ccx) = test_context("root-a", "root-a", None).await;
        let peer = create(&app, "root-b", "peer").await;
        app.agent_executions
            .remember_scope(&peer.agent_id, std::env::temp_dir());
        let project = crate::coordination::roots::project_identity(&std::env::temp_dir());
        app.peer_grants
            .grant(
                crate::coordination::peer_authority::PeerGrant {
                    project: project.clone(),
                    sender_root: "root-a".to_string(),
                    recipient_root: "root-b".to_string(),
                    authority: crate::coordination::peer_authority::PeerAuthority::Mail,
                    granted_by: "user".to_string(),
                    expires_at_ms: delivery_now_ms() + 10_000,
                },
                delivery_now_ms(),
            )
            .unwrap();
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        let preempt = tool
            .tool_execute(
                ccx.clone(),
                &"preempt".to_string(),
                &args(&[
                    ("to", json!(peer.agent_id)),
                    ("text", json!("stop")),
                    ("push", json!("preempt")),
                ]),
            )
            .await
            .unwrap_err();
        assert!(preempt.contains("no authority to interrupt"), "{preempt}");
        let (_, question_id) = app
            .agents
            .add_question(&peer.agent_id, "May I?".to_string())
            .await
            .unwrap();
        let answer = tool
            .tool_execute(
                ccx.clone(),
                &"answer".to_string(),
                &args(&[
                    ("to", json!(peer.agent_id)),
                    ("text", json!("yes")),
                    ("reply_to", json!(question_id)),
                ]),
            )
            .await
            .unwrap_err();
        assert!(answer.contains("no control over that agent"), "{answer}");
        assert!(
            app.agents.get_any(&peer.agent_id).await.unwrap().questions[0]
                .answer
                .is_none(),
            "a refused peer answer must not mutate the peer's question"
        );
        assert!(app
            .agents
            .get_any(&peer.agent_id)
            .await
            .unwrap()
            .pending_deliveries
            .is_empty());

        tool.tool_execute(
            ccx,
            &"informational".to_string(),
            &args(&[("to", json!(peer.agent_id)), ("text", json!("fyi"))]),
        )
        .await
        .unwrap();
        let queued = app
            .agents
            .get_any(&peer.agent_id)
            .await
            .unwrap()
            .pending_deliveries;
        assert!(!queued[0].wake, "peer mail must never wake another root");
        assert_eq!(
            queued[0]
                .peer_mail
                .as_ref()
                .map(|stamp| (stamp.sender_root.as_str(), stamp.recipient_root.as_str())),
            Some(("root-a", "root-b"))
        );
    }

    /// Consent is re-checked at delivery, not only at send time.
    #[tokio::test]
    async fn a_grant_revoked_between_enqueue_and_drain_stops_the_delivery() {
        for revoke in [true, false] {
            let (app, ccx) = test_context("root-a", "root-a", None).await;
            let peer = create(&app, "root-b", "peer").await;
            app.agent_executions
                .remember_scope(&peer.agent_id, std::env::temp_dir());
            let project = crate::coordination::roots::project_identity(&std::env::temp_dir());
            app.peer_grants
                .grant(
                    crate::coordination::peer_authority::PeerGrant {
                        project: project.clone(),
                        sender_root: "root-a".to_string(),
                        recipient_root: "root-b".to_string(),
                        authority: crate::coordination::peer_authority::PeerAuthority::Mail,
                        granted_by: "user".to_string(),
                        expires_at_ms: delivery_now_ms() + 60_000,
                    },
                    delivery_now_ms(),
                )
                .unwrap();
            let queued = ToolAgentMessage {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"peer".to_string(),
                &args(&[("to", json!(peer.agent_id)), ("text", json!("fyi"))]),
            )
            .await
            .unwrap();
            let delivery_id = receipt_of(&queued)["delivery_id"]
                .as_str()
                .unwrap()
                .to_string();

            if revoke {
                assert!(app.peer_grants.revoke(&project, "root-a", "root-b"));
            }
            let drained = crate::agents::delivery::drain_authorized(&app, &peer.agent_id, false)
                .await
                .unwrap();
            assert_eq!(drained.len(), usize::from(!revoke), "revoke={revoke}");
            assert_eq!(
                app.agents
                    .delivery_status(&peer.agent_id, &delivery_id)
                    .await,
                if revoke {
                    refact_chat_api::DeliveryStatus::Failed
                } else {
                    refact_chat_api::DeliveryStatus::Pending
                }
            );
        }
    }

    #[tokio::test]
    async fn unrelated_root_requires_directional_same_project_mail_grant() {
        let (app, ccx) = test_context("root-a", "root-a", None).await;
        let peer = create(&app, "root-b", "peer").await;
        app.agent_executions
            .remember_scope(&peer.agent_id, std::env::temp_dir());
        let project = crate::coordination::roots::project_identity(&std::env::temp_dir());
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        let request = args(&[
            ("to", json!(peer.agent_id.clone())),
            ("text", json!("hello")),
        ]);
        assert!(tool
            .tool_execute(ccx.clone(), &"denied".to_string(), &request)
            .await
            .unwrap_err()
            .contains("explicit same-project mail grant"));
        app.peer_grants
            .grant(
                crate::coordination::peer_authority::PeerGrant {
                    project: project.clone(),
                    sender_root: "root-a".to_string(),
                    recipient_root: "root-b".to_string(),
                    authority: crate::coordination::peer_authority::PeerAuthority::Metadata,
                    granted_by: "user".to_string(),
                    expires_at_ms: delivery_now_ms() + 10_000,
                },
                delivery_now_ms(),
            )
            .unwrap();
        assert!(tool
            .tool_execute(ccx.clone(), &"metadata".to_string(), &request)
            .await
            .unwrap_err()
            .contains("explicit same-project mail grant"));
        app.peer_grants
            .grant(
                crate::coordination::peer_authority::PeerGrant {
                    project: project.clone(),
                    sender_root: "root-a".to_string(),
                    recipient_root: "root-b".to_string(),
                    authority: crate::coordination::peer_authority::PeerAuthority::Mail,
                    granted_by: "user".to_string(),
                    expires_at_ms: delivery_now_ms() + 10_000,
                },
                delivery_now_ms(),
            )
            .unwrap();
        let response = tool
            .tool_execute(ccx, &"allowed".to_string(), &request)
            .await
            .unwrap();
        assert!(text(response).contains("Message queued"));
        assert!(!app.peer_grants.allows(
            &project,
            "root-b",
            "root-a",
            crate::coordination::peer_authority::PeerAuthority::Mail,
            delivery_now_ms(),
        ));
    }

    #[tokio::test]
    async fn child_question_records_question_and_queues_parent_notice() {
        let (app, ccx) = test_context("child-chat", "root", None).await;
        let child = create(&app, "root", "child").await;
        ccx.lock().await.background_agent_id = Some(child.agent_id.clone());
        let (_workspace, session) = parent_delivery_fixture(&app, "root").await;

        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        let output = text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("to", json!("parent")),
                    ("text", json!("May I edit frogs?")),
                    ("expects_reply", json!(true)),
                ]),
            )
            .await
            .unwrap(),
        );
        let after = app.agents.get_any(&child.agent_id).await.unwrap();
        assert_eq!(after.questions.len(), 1);
        assert!(output.contains(&after.questions[0].id));
        assert!(session.lock().await.messages.iter().any(|message| {
            message
                .content
                .content_text_only()
                .contains("subagent question")
        }));
        assert_parent_notice_persisted(&app, "root", "May I edit frogs?").await;
    }

    #[tokio::test]
    async fn reply_answers_question_and_delivers_inbox() {
        let (app, ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let (_, question_id) = app
            .agents
            .add_question(&child.agent_id, "Can I edit frogs?".to_string())
            .await
            .unwrap();
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        tool.tool_execute(
            ccx,
            &"call".to_string(),
            &args(&[
                ("to", json!(child.agent_id)),
                ("reply_to", json!(question_id)),
                ("text", json!("Yes")),
            ]),
        )
        .await
        .unwrap();
        let after = app.agents.get_any(&child.agent_id).await.unwrap();
        assert_eq!(after.questions[0].answer.as_deref(), Some("Yes"));
        let inbox = app
            .agents
            .get_any(&child.agent_id)
            .await
            .unwrap()
            .pending_deliveries;
        assert_eq!(inbox[0].push, PushMode::Append);
        assert!(inbox[0].messages[0]
            .content
            .content_text_only()
            .contains(&format!("Answer to {question_id}: Yes")));
    }

    #[tokio::test]
    async fn child_note_delivers_to_parent_without_changing_progress() {
        let (app, ccx) = test_context("child-chat", "root", None).await;
        let (_workspace, _session) = parent_delivery_fixture(&app, "root").await;
        let child = create(&app, "root", "child").await;
        ccx.lock().await.background_agent_id = Some(child.agent_id.clone());
        let mut tool = ToolAgentMessage {
            config_path: String::new(),
        };
        tool.tool_execute(
            ccx,
            &"call".to_string(),
            &args(&[("to", json!("parent")), ("text", json!("I found a clue"))]),
        )
        .await
        .unwrap();
        let after = app.agents.get_any(&child.agent_id).await.unwrap();
        assert_eq!(after.progress, None);
        assert_parent_notice_persisted(&app, "root", "I found a clue").await;
    }

    #[tokio::test]
    async fn progress_report_requires_child_identity() {
        let (app, parent_ccx) = test_context("root", "root", None).await;
        let child = create(&app, "root", "child").await;
        let child_ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "child-chat".to_string(),
                Some("root".to_string()),
                "test/model".to_string(),
                None,
                None,
            )
            .await,
        ));
        child_ccx.lock().await.background_agent_id = Some(child.agent_id.clone());
        let mut tool = ToolProgressReport {
            config_path: String::new(),
        };
        assert!(tool
            .tool_execute(
                parent_ccx,
                &"call".to_string(),
                &args(&[("status_line", json!("working"))]),
            )
            .await
            .unwrap_err()
            .contains("not a subagent"));
        tool.tool_execute(
            child_ccx,
            &"call".to_string(),
            &args(&[("status_line", json!("working"))]),
        )
        .await
        .unwrap();
        assert_eq!(
            app.agents
                .get_any(&child.agent_id)
                .await
                .unwrap()
                .progress
                .as_deref(),
            Some("working")
        );
    }

    #[tokio::test]
    async fn inbox_drain_appends_system_notice() {
        let (app, ccx) = test_context("child-chat", "root", None).await;
        let child = create(&app, "root", "child").await;
        ccx.lock().await.background_agent_id = Some(child.agent_id.clone());
        app.agents
            .push_inbox(
                &child.agent_id,
                InboxMessage {
                    from: "user".to_string(),
                    text: "Please verify tests".to_string(),
                    queued_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let mut messages = Vec::new();
        crate::subchat::drain_background_agent_inbox(&ccx, &mut messages).await;
        assert_eq!(messages.len(), 1);
        assert!(messages[0]
            .content
            .content_text_only()
            .contains("[message from user]\nPlease verify tests"));
    }

    #[tokio::test]
    async fn unloaded_subchat_context_restores_background_agent_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let child = create(&app, "parent", "child").await;
        app.agents
            .mark_running(&child.agent_id, "subchat-unloaded-child".to_string())
            .await
            .unwrap();

        let ccx = AtCommandsContext::new_from_app(
            app,
            4096,
            20,
            false,
            vec![],
            "subchat-unloaded-child".to_string(),
            Some("parent".to_string()),
            "test/model".to_string(),
            None,
            None,
        )
        .await;

        assert_eq!(
            ccx.background_agent_id.as_deref(),
            Some(child.agent_id.as_str())
        );
    }

    #[tokio::test]
    async fn stateful_child_session_restores_identity_for_parent_tools() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let (_workspace, _parent_session) = parent_delivery_fixture(&app, "parent").await;
        let child = create(&app, "parent", "child").await;
        app.agents
            .mark_running(&child.agent_id, "subchat-child-chat".to_string())
            .await
            .unwrap();
        let config = crate::subchat::SubchatConfig {
            history_source_model: None,
            tool_name: "subagent".to_string(),
            stateful: true,
            autonomous_no_confirm: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            chat_id: Some("subchat-child-chat".to_string()),
            title: Some("child".to_string()),
            parent_id: Some("parent".to_string()),
            link_type: Some("subagent".to_string()),
            root_chat_id: Some("parent".to_string()),
            tools: crate::subchat::ToolsPolicy::All,
            max_steps: 1,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: None,
            worktree: None,
            model: "test/model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            soft_abort: false,
            background_agent_id: Some(child.agent_id.clone()),
            subchat_depth: 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: crate::subchat::TraceParent::rooted("parent", "parent"),
        };
        crate::subchat::install_stateful_subchat_session(&app, "subchat-child-chat", &config, &[])
            .await;
        let thread = app
            .chat
            .sessions
            .read()
            .await
            .get("subchat-child-chat")
            .cloned()
            .unwrap()
            .lock()
            .await
            .thread
            .clone();
        crate::chat::trajectories::save_trajectory_as_with_intent(
            gcx.clone(),
            &thread,
            &[],
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if crate::chat::trajectories::load_trajectory_for_chat(
                    gcx.clone(),
                    "subchat-child-chat",
                )
                .await
                .is_some()
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("child trajectory should persist");
        app.chat.sessions.write().await.remove("subchat-child-chat");

        crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            "subchat-child-chat",
        )
        .await;
        app.chat
            .sessions
            .read()
            .await
            .get("subchat-child-chat")
            .cloned()
            .unwrap()
            .lock()
            .await
            .thread
            .parent_id = None;

        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "subchat-child-chat".to_string(),
                Some("parent".to_string()),
                "test/model".to_string(),
                None,
                None,
            )
            .await,
        ));
        assert_eq!(
            ccx.lock().await.background_agent_id.as_deref(),
            Some(child.agent_id.as_str())
        );
        let mut progress = ToolProgressReport {
            config_path: String::new(),
        };
        progress
            .tool_execute(
                ccx.clone(),
                &"progress".to_string(),
                &args(&[("status_line", json!("stateful child running"))]),
            )
            .await
            .unwrap();
        let mut message = ToolAgentMessage {
            config_path: String::new(),
        };
        message
            .tool_execute(
                ccx,
                &"message".to_string(),
                &args(&[("to", json!("parent")), ("text", json!("hello parent"))]),
            )
            .await
            .unwrap();
        let record = app.agents.get_any(&child.agent_id).await.unwrap();
        assert_eq!(
            record.progress.as_deref(),
            Some("stateful child running"),
            "agent_message delivery is independent of progress activity"
        );
        assert_parent_notice_persisted(&app, "parent", "hello parent").await;
    }
}
