use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::agents::registry::InboxMessage;
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

pub struct ToolProgressReport {
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

async fn can_message_agent(
    app: &AppState,
    caller_chat_id: &str,
    root_chat_id: &str,
    caller_agent_id: Option<&str>,
    target_agent_id: &str,
) -> Result<BackgroundAgent, String> {
    let target = app.agents.get_any(target_agent_id).await?;
    if target.parent_chat_id == caller_chat_id {
        return Ok(target);
    }
    if let Some(caller_agent_id) = caller_agent_id {
        if app
            .agents
            .list_descendants(caller_agent_id)
            .await
            .iter()
            .any(|record| record.agent_id == target_agent_id)
        {
            return Ok(target);
        }
    } else {
        let in_root_tree = app.agents.list_all().await.iter().any(|record| {
            record.agent_id == target_agent_id
                && (record.parent_chat_id == root_chat_id
                    || record.parent_root_chat_id.as_deref() == Some(root_chat_id))
        });
        if in_root_tree {
            return Ok(target);
        }
    }
    Err("You may only message your direct children or descendants.".to_string())
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
                    "text": {"type": "string", "description": "Message text."},
                    "expects_reply": {"type": "boolean", "description": "When messaging parent, create a tracked question."},
                    "reply_to": {"type": "string", "description": "Question id being answered when messaging a child."}
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
        let reply_to = optional_string(args, "reply_to");
        let (app, chat_id, root_chat_id, caller_agent_id) = context(&ccx).await;

        if to == "parent" {
            let Some(agent_id) = caller_agent_id else {
                return Err(
                    "You are not a subagent; use an agent id to message a child.".to_string(),
                );
            };
            let caller = app.agents.get_any(&agent_id).await?;
            let parent_notice_chat_id = parent_notice_chat_id(&app, &caller).await;
            if expects_reply {
                let (updated, question_id) =
                    app.agents.add_question(&agent_id, text.clone()).await?;
                crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
                if let Some(parent_notice_chat_id) = parent_notice_chat_id {
                    crate::agents::push::push_notice_to_chat(
                        app,
                        &parent_notice_chat_id,
                        format!(
                            "[subagent question] agent {agent_id} asks: {text} — answer with agent_message(to=\"{agent_id}\", reply_to=\"{question_id}\", text=...)"
                        ),
                    )
                    .await?;
                }
                return Ok(output(
                    tool_call_id,
                    format!("Question {question_id} sent to parent."),
                ));
            }
            if let Some(parent_notice_chat_id) = parent_notice_chat_id {
                crate::agents::push::push_notice_to_chat(
                    app.clone(),
                    &parent_notice_chat_id,
                    format!("[subagent note] agent {agent_id}: {text}"),
                )
                .await?;
            } else {
                let updated = app
                    .agents
                    .update_activity(
                        &agent_id,
                        Some(format!("note to parent: {text}")),
                        None,
                        None,
                    )
                    .await?;
                crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
            }
            return Ok(output(tool_call_id, "Note sent to parent.".to_string()));
        }

        let target = can_message_agent(
            &app,
            &chat_id,
            &root_chat_id,
            caller_agent_id.as_deref(),
            &to,
        )
        .await?;
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
        app.agents
            .push_inbox(
                &to,
                InboxMessage {
                    from: "parent".to_string(),
                    text: reply_to
                        .as_deref()
                        .map(|question_id| format!("Answer to {question_id}: {text}"))
                        .unwrap_or(text),
                    queued_at: Utc::now(),
                },
            )
            .await?;
        Ok(output(tool_call_id, format!("Message queued for {to}.")))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

async fn parent_notice_chat_id(app: &AppState, caller: &BackgroundAgent) -> Option<String> {
    let sessions = app.chat.sessions.read().await;
    if sessions.contains_key(&caller.parent_chat_id) {
        return Some(caller.parent_chat_id.clone());
    }
    caller
        .parent_root_chat_id
        .as_ref()
        .filter(|chat_id| sessions.contains_key(chat_id.as_str()))
        .cloned()
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

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
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
        assert!(error.contains("direct children or descendants"));
    }

    #[tokio::test]
    async fn child_question_records_question_and_queues_parent_notice() {
        let (app, ccx) = test_context("child-chat", "root", None).await;
        let child = create(&app, "root", "child").await;
        ccx.lock().await.background_agent_id = Some(child.agent_id.clone());
        let session = Arc::new(AMutex::new(ChatSession::new("root".to_string())));
        session
            .lock()
            .await
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        app.chat
            .sessions
            .write()
            .await
            .insert("root".to_string(), session.clone());

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
        assert!(session.lock().await.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("subagent question")));
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
        let inbox = app.agents.drain_inbox(&child.agent_id).await;
        assert_eq!(inbox[0].text, format!("Answer to {question_id}: Yes"));
    }

    #[tokio::test]
    async fn child_note_tolerates_missing_parent_session() {
        let (app, ccx) = test_context("child-chat", "root", None).await;
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
        assert!(after.progress.unwrap().contains("I found a clue"));
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
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let child = create(&app, "parent", "child").await;
        app.agents
            .mark_running(&child.agent_id, "subchat-child-chat".to_string())
            .await
            .unwrap();
        let config = crate::subchat::SubchatConfig {
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
            Some("note to parent: hello parent"),
            "agent_message records the latest activity line (last write wins)"
        );
    }
}
