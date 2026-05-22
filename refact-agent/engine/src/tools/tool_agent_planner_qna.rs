use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::tools::task_tool_helpers::require_bound_planner_task;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const ASK_PREFIX: &str = "[ASK:";
const REPLY_PREFIX: &str = "[REPLY:";
const QUESTION_LIMIT: usize = 800;
const ANSWER_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuestionUrgency {
    Info,
    Block,
}

impl QuestionUrgency {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value.and_then(|value| value.as_str()).unwrap_or("info") {
            "info" => Ok(Self::Info),
            "block" => Ok(Self::Block),
            other => Err(format!(
                "Invalid urgency '{}', must be one of: info, block",
                other
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnansweredQuestion {
    card_id: String,
    card_title: String,
    question_id: String,
    urgency: String,
    question: String,
}

pub struct ToolAgentAskPlanner;
pub struct ToolPlannerReply;
pub struct ToolTaskQuestionsList;

impl ToolAgentAskPlanner {
    pub fn new() -> Self {
        Self
    }
}

impl ToolPlannerReply {
    pub fn new() -> Self {
        Self
    }
}

impl ToolTaskQuestionsList {
    pub fn new() -> Self {
        Self
    }
}

fn make_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

fn tool_message(tool_call_id: &str, content: String) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    })
}

fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn make_question_id() -> String {
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

fn is_question_id(value: &str) -> bool {
    value.len() == 8
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn required_question_id(args: &HashMap<String, Value>) -> Result<String, String> {
    let question_id = required_string(args, "question_id")?;
    if !is_question_id(&question_id) {
        return Err("question_id must be 8 lowercase hex characters".to_string());
    }
    Ok(question_id)
}

async fn agent_scope(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> Result<(Arc<GlobalContext>, String, String), String> {
    let ccx_lock = ccx.lock().await;
    let meta = ccx_lock
        .task_meta
        .as_ref()
        .ok_or_else(|| "agent_ask_planner can only be called by task agents.".to_string())?;
    if meta.role != "agents" {
        return Err("agent_ask_planner can only be called by task agents.".to_string());
    }
    let card_id = meta
        .card_id
        .clone()
        .ok_or_else(|| "agent_ask_planner requires a bound card_id.".to_string())?;
    Ok((ccx_lock.app.gcx.clone(), meta.task_id.clone(), card_id))
}

async fn require_planner_role(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    tool_name: &str,
) -> Result<(), String> {
    let ccx_lock = ccx.lock().await;
    let meta = ccx_lock
        .task_meta
        .as_ref()
        .ok_or_else(|| format!("{} can only be called by the task planner.", tool_name))?;
    if meta.role != "planner" {
        return Err(format!(
            "{} can only be called by the task planner.",
            tool_name
        ));
    }
    Ok(())
}

async fn planner_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<String, String> {
    require_planner_role(ccx, tool_name).await?;
    require_bound_planner_task(ccx, args).await
}

fn parse_status_id<'a>(message: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let rest = message.strip_prefix(prefix)?;
    let end = rest.find(']')?;
    let id = &rest[..end];
    if !is_question_id(id) {
        return None;
    }
    Some((id, rest[end + 1..].trim_start()))
}

fn parse_ask(message: &str) -> Option<(&str, &str)> {
    parse_status_id(message, ASK_PREFIX)
}

fn parse_reply(message: &str) -> Option<(&str, &str)> {
    parse_status_id(message, REPLY_PREFIX)
}

fn parse_block_marker(message: &str) -> Option<&str> {
    let rest = message.strip_prefix(ASK_PREFIX)?;
    let id = rest.strip_suffix(":block] agent flagged for planner attention")?;
    if !is_question_id(id) {
        return None;
    }
    Some(id)
}

fn card_has_ask(card: &BoardCard, question_id: &str) -> bool {
    card.status_updates.iter().any(|update| {
        parse_ask(&update.message)
            .map(|(id, _)| id == question_id)
            .unwrap_or(false)
    })
}

fn split_question_and_urgency(text: &str) -> (String, String) {
    let trimmed = text.trim();
    for urgency in ["block", "info"] {
        let suffix = format!(" (urgency={})", urgency);
        if let Some(question) = trimmed.strip_suffix(&suffix) {
            return (question.to_string(), urgency.to_string());
        }
    }
    (trimmed.to_string(), "info".to_string())
}

fn collect_unanswered_questions(cards: &[BoardCard]) -> Vec<UnansweredQuestion> {
    let mut replied = HashSet::new();
    let mut block_markers = HashSet::new();
    for card in cards {
        for update in &card.status_updates {
            if let Some((id, _)) = parse_reply(&update.message) {
                replied.insert((card.id.clone(), id.to_string()));
            }
            if let Some(id) = parse_block_marker(&update.message) {
                block_markers.insert((card.id.clone(), id.to_string()));
            }
        }
    }

    let mut seen = HashSet::new();
    let mut questions = Vec::new();
    for card in cards {
        for update in &card.status_updates {
            let Some((id, text)) = parse_ask(&update.message) else {
                continue;
            };
            let key = (card.id.clone(), id.to_string());
            if replied.contains(&key) || !seen.insert(key.clone()) {
                continue;
            }
            let (question, mut urgency) = split_question_and_urgency(text);
            if block_markers.contains(&key) {
                urgency = "block".to_string();
            }
            questions.push(UnansweredQuestion {
                card_id: card.id.clone(),
                card_title: card.title.clone(),
                question_id: id.to_string(),
                urgency,
                question,
            });
        }
    }
    questions.sort_by(|a, b| {
        a.card_id
            .cmp(&b.card_id)
            .then_with(|| a.question_id.cmp(&b.question_id))
    });
    questions
}

fn markdown_table_cell(value: &str) -> String {
    value
        .replace('\r', "")
        .replace('\n', "<br>")
        .replace('|', "\\|")
}

fn format_unanswered_questions(questions: &[UnansweredQuestion]) -> String {
    if questions.is_empty() {
        return "# Unanswered Planner Questions\n\nNo unanswered planner questions.".to_string();
    }

    let mut lines = vec![
        "# Unanswered Planner Questions".to_string(),
        String::new(),
        "| Card | Title | Question ID | Urgency | Question |".to_string(),
        "|---|---|---|---|---|".to_string(),
    ];
    for question in questions {
        lines.push(format!(
            "| `{}` | {} | `{}` | {} | {} |",
            markdown_table_cell(&question.card_id),
            markdown_table_cell(&question.card_title),
            question.question_id,
            question.urgency,
            markdown_table_cell(&question.question)
        ));
    }
    lines.join("\n")
}

#[async_trait]
impl Tool for ToolAgentAskPlanner {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "agent_ask_planner".to_string(),
            display_name: "Agent Ask Planner".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Task-agent-only tool for recording an asynchronous question for the task planner on the current card.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "Question for the planner"
                    },
                    "urgency": {
                        "type": "string",
                        "enum": ["info", "block"],
                        "description": "Question urgency. block only flags planner attention; it does not pause the agent. Default: info"
                    }
                },
                "required": ["question", "urgency"]
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
        let (gcx, task_id, card_id) = agent_scope(&ccx).await?;
        let question = truncate_chars(&required_string(args, "question")?, QUESTION_LIMIT);
        let urgency = QuestionUrgency::parse(args.get("urgency"))?;
        let question_id = make_question_id();
        let message = format!(
            "[ASK:{}] {} (urgency={})",
            question_id,
            question,
            urgency.as_str()
        );
        let block_message = if urgency == QuestionUrgency::Block {
            Some(format!(
                "[ASK:{}:block] agent flagged for planner attention",
                question_id
            ))
        } else {
            None
        };
        let card_id_for_update = card_id.clone();
        let timestamp = Utc::now().to_rfc3339();
        storage::update_board_atomic(gcx, &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            card.status_updates.push(StatusUpdate {
                timestamp: timestamp.clone(),
                message: message.clone(),
            });
            if let Some(block_message) = &block_message {
                card.status_updates.push(StatusUpdate {
                    timestamp: timestamp.clone(),
                    message: block_message.clone(),
                });
            }
            Ok(())
        })
        .await?;

        let output = format!(
            "Question recorded for planner.\n\n- card_id: `{}`\n- question_id: `{}`\n- urgency: `{}`\n\nPlanner reply instruction: call `planner_reply(card_id=\"{}\", question_id=\"{}\", answer=\"...\")`.",
            card_id,
            question_id,
            urgency.as_str(),
            card_id,
            question_id
        );
        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolPlannerReply {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "planner_reply".to_string(),
            display_name: "Planner Reply".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool for answering an agent_ask_planner question recorded on a task card.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "card_id": {
                        "type": "string",
                        "description": "Card ID containing the question"
                    },
                    "question_id": {
                        "type": "string",
                        "description": "8 lowercase hex question ID returned by agent_ask_planner"
                    },
                    "answer": {
                        "type": "string",
                        "description": "Planner answer to record"
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (optional if planner chat is bound to a task)"
                    }
                },
                "required": ["card_id", "question_id", "answer"]
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
        let task_id = planner_task_id(&ccx, args, "planner_reply").await?;
        let card_id = required_string(args, "card_id")?;
        let question_id = required_question_id(args)?;
        let answer = truncate_chars(&required_string(args, "answer")?, ANSWER_LIMIT);
        let gcx = ccx.lock().await.app.gcx.clone();

        let card_id_for_update = card_id.clone();
        let question_id_for_update = question_id.clone();
        let answer_for_update = answer.clone();
        storage::update_board_atomic(gcx, &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            if !card_has_ask(card, &question_id_for_update) {
                return Err(format!(
                    "Question {} not found on card {}",
                    question_id_for_update, card_id_for_update
                ));
            }
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: format!("[REPLY:{}] {}", question_id_for_update, answer_for_update),
            });
            Ok(())
        })
        .await?;

        let output = format!(
            "Reply delivered to card `{}` for question `{}`.\n\nAnswer: {}",
            card_id, question_id, answer
        );
        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskQuestionsList {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_questions_list".to_string(),
            display_name: "Task Questions List".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "Planner-only tool that lists unanswered agent questions recorded in card status updates.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (optional if planner chat is bound to a task)"
                    }
                },
                "required": []
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
        let task_id = planner_task_id(&ccx, args, "task_questions_list").await?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let board = storage::load_board(gcx, &task_id).await?;
        let questions = collect_unanswered_questions(&board.cards);
        Ok((
            false,
            vec![tool_message(
                tool_call_id,
                format_unanswered_questions(&questions),
            )],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::types::TaskMeta as ThreadTaskMeta;
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus};
    use crate::tools::tools_description::Tool;

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn test_card(id: &str, title: &str, status_updates: Vec<StatusUpdate>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: title.to_string(),
            column: "doing".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: Some(format!("agent-chat-{}", id)),
            status_updates,
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn task_meta() -> TaskMeta {
        let now = Utc::now().to_rfc3339();
        TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    async fn write_task(
        root: &std::path::Path,
        cards: Vec<BoardCard>,
    ) -> Arc<crate::global_context::GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        storage::save_task_meta(gcx.clone(), "task-1", &task_meta())
            .await
            .unwrap();
        storage::save_board(
            gcx.clone(),
            "task-1",
            &TaskBoard {
                cards,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        gcx
    }

    async fn task_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        role: &str,
        card_id: Option<&str>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                format!("{}-chat", role),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: role.to_string(),
                    agent_id: Some("agent-1".to_string()),
                    card_id: card_id.map(str::to_string),
                    planner_chat_id: Some("planner-chat".to_string()),
                }),
                None,
            )
            .await,
        ))
    }

    fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[tokio::test]
    async fn agent_ask_planner_rejects_non_agent_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), vec![test_card("T-40", "QnA", vec![])]).await;
        let ccx = task_ccx(gcx, "planner", None).await;

        let err = ToolAgentAskPlanner::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("question", json!("What should I do?")),
                    ("urgency", json!("info")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by task agents"));
    }

    #[test]
    fn question_id_format_8_hex() {
        let question_id = make_question_id();

        assert_eq!(question_id.len(), 8);
        assert!(question_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }

    #[tokio::test]
    async fn planner_reply_rejects_non_planner_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card(
                "T-40",
                "QnA",
                vec![StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: "[ASK:1234abcd] Need direction (urgency=info)".to_string(),
                }],
            )],
        )
        .await;
        let ccx = task_ccx(gcx, "agents", Some("T-40")).await;

        let err = ToolPlannerReply::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-40")),
                    ("question_id", json!("1234abcd")),
                    ("answer", json!("Use the smaller fix.")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("planner_reply can only be called by the task planner"));
    }

    #[tokio::test]
    async fn planner_reply_rejects_unknown_question_id() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), vec![test_card("T-40", "QnA", vec![])]).await;
        let ccx = task_ccx(gcx, "planner", None).await;

        let err = ToolPlannerReply::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-40")),
                    ("question_id", json!("1234abcd")),
                    ("answer", json!("Use the smaller fix.")),
                ]),
            )
            .await
            .unwrap_err();

        assert_eq!(err, "Question 1234abcd not found on card T-40");
    }

    #[test]
    fn task_questions_list_filters_unanswered_correctly() {
        let cards = vec![
            test_card(
                "T-1",
                "first card",
                vec![
                    StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: "[ASK:aaaaaaaa] Answered question (urgency=info)".to_string(),
                    },
                    StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: "[REPLY:aaaaaaaa] Answer".to_string(),
                    },
                    StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: "[ASK:bbbbbbbb] Blocking question (urgency=info)".to_string(),
                    },
                    StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: "[ASK:bbbbbbbb:block] agent flagged for planner attention"
                            .to_string(),
                    },
                ],
            ),
            test_card(
                "T-2",
                "second card",
                vec![StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: "[ASK:cccccccc] Open question (urgency=info)".to_string(),
                }],
            ),
        ];

        let questions = collect_unanswered_questions(&cards);
        let output = format_unanswered_questions(&questions);

        assert_eq!(questions.len(), 2);
        assert!(!output.contains("aaaaaaaa"));
        assert!(output.contains("bbbbbbbb"));
        assert!(output.contains("cccccccc"));
        assert!(output.contains("| block | Blocking question |"));
        assert!(output.contains("| info | Open question |"));
    }

    #[tokio::test]
    async fn agent_ask_planner_records_question_and_block_marker() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), vec![test_card("T-40", "QnA", vec![])]).await;
        let ccx = task_ccx(gcx.clone(), "agents", Some("T-40")).await;

        let output = output_text(
            ToolAgentAskPlanner::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[
                        ("question", json!("Should I choose implementation A or B?")),
                        ("urgency", json!("block")),
                    ]),
                )
                .await
                .unwrap(),
        );

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-40").unwrap();
        assert_eq!(card.status_updates.len(), 2);
        assert!(card.status_updates[0].message.starts_with("[ASK:"));
        assert!(card.status_updates[0]
            .message
            .contains("Should I choose implementation A or B? (urgency=block)"));
        assert!(card.status_updates[1]
            .message
            .ends_with(":block] agent flagged for planner attention"));
        assert!(output.contains("question_id"));
    }
}
