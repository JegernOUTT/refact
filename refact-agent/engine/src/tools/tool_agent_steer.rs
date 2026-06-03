use std::collections::HashMap;
use std::sync::Arc;

use refact_chat_history::history_limit::remove_invalid_tool_calls_and_tool_calls_results;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use refact_chat_api::ChatCommand;
use refact_runtime_api::{ChatSessionUpdate, SessionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentSteerPriority {
    Info,
    Steer,
    Urgent,
}

impl AgentSteerPriority {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value.and_then(|v| v.as_str()).unwrap_or("steer") {
            "info" => Ok(Self::Info),
            "steer" => Ok(Self::Steer),
            "urgent" => Ok(Self::Urgent),
            other => Err(format!(
                "Invalid priority '{}', must be one of: info, steer, urgent",
                other
            )),
        }
    }

    fn format_message(self, message: &str) -> String {
        match self {
            Self::Info => format!("[Planner FYI] {}", message),
            Self::Steer => format!(
                "[Planner STEER] {}\n\nPlease adapt based on this and continue.",
                message
            ),
            Self::Urgent => format!(
                "[Planner URGENT] {}\n\nStop current action and address this.",
                message
            ),
        }
    }
}

pub struct ToolAgentSteer;

impl ToolAgentSteer {
    pub fn new() -> Self {
        Self
    }
}

fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

async fn planner_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    let meta = ccx_lock
        .task_meta
        .as_ref()
        .ok_or_else(|| "agent_steer can only be called by the task planner.".to_string())?;
    if meta.role != "planner" {
        return Err("agent_steer can only be called by the task planner.".to_string());
    }
    if let Some(task_id) = args.get("task_id").and_then(|value| value.as_str()) {
        if task_id != meta.task_id {
            return Err(format!(
                "Supplied task_id '{}' does not match bound task_id '{}'",
                task_id, meta.task_id
            ));
        }
    }
    Ok(meta.task_id.clone())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn state_label(state: SessionState) -> String {
    match state {
        SessionState::Idle => "💤 Idle (steer will start a new turn)".to_string(),
        SessionState::Generating => {
            "🔄 Generating response (steer will be processed after current generation)".to_string()
        }
        SessionState::ExecutingTools => {
            "⚙️ Executing tools (steer will be processed after current tool)".to_string()
        }
        SessionState::Paused => "⏸️ Paused (steer is queued)".to_string(),
        SessionState::WaitingIde => "⏳ Waiting for IDE (steer is queued)".to_string(),
        SessionState::WaitingUserInput => "⏳ Waiting for user input (steer is queued)".to_string(),
        SessionState::Completed => "✅ Completed (steer will start a new turn)".to_string(),
        SessionState::Error => "❌ Error (steer is queued)".to_string(),
    }
}

fn tool_output(card_id: &str, message: &str, state: SessionState, compacted: bool) -> String {
    let compaction_note = if compacted {
        "\nAuto-compaction: applied before steering."
    } else {
        ""
    };
    format!(
        "✅ Steered {}\n\nMessage: \"{}\"\nAgent state: {}{}",
        card_id,
        message,
        state_label(state),
        compaction_note
    )
}

fn should_autocompact_before_steer(
    state: SessionState,
    thread: &refact_chat_api::ThreadParams,
) -> bool {
    thread.auto_compact_enabled_effective()
        && matches!(state, SessionState::Completed | SessionState::Error)
}

fn autocompact_messages_for_resume(messages: &mut Vec<ChatMessage>) -> bool {
    let before = serde_json::to_string(messages).ok();
    let summarized = crate::chat::summarization::summarize_oldest_segment_with_static_summary(
        messages,
        "Previous non-user task-agent activity was summarized before planner steering.",
        "agent_steer",
    );
    remove_invalid_tool_calls_and_tool_calls_results(messages);
    summarized || serde_json::to_string(messages).ok() != before
}

fn validate_agent_snapshot(
    snapshot: &refact_runtime_api::ChatSessionSnapshot,
    task_id: &str,
    card_id: &str,
    assignee: Option<&str>,
) -> Result<(), String> {
    let Some(task_meta) = &snapshot.thread.task_meta else {
        return Err(format!(
            "Agent session for card {card_id} is missing task metadata; it may be stale or deleted."
        ));
    };
    if task_meta.role != "agents" || task_meta.task_id != task_id {
        return Err(format!(
            "Agent session for card {card_id} is bound to a different task/role."
        ));
    }
    if task_meta.card_id.as_deref() != Some(card_id) {
        return Err(format!(
            "Agent session metadata does not match card {card_id}."
        ));
    }
    if let Some(assignee) = assignee {
        if task_meta.agent_id.as_deref() != Some(assignee) {
            return Err(format!(
                "Agent session for card {card_id} is assigned to a different agent."
            ));
        }
    }
    Ok(())
}

#[async_trait]
impl Tool for ToolAgentSteer {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "agent_steer".to_string(),
            display_name: "Agent Steer".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool that injects a steering user message into a running task agent chat through the chat command queue.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "card_id": {
                        "type": "string",
                        "description": "Doing card whose running agent should receive the steering message"
                    },
                    "message": {
                        "type": "string",
                        "description": "What to tell the agent"
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["info", "steer", "urgent"],
                        "description": "Message priority. Default: steer"
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (optional if chat is bound to a task)"
                    }
                },
                "required": ["card_id", "message"]
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
        let task_id = planner_task_id(&ccx, args).await?;
        let card_id = required_string(args, "card_id")?;
        let message = required_string(args, "message")?;
        let priority = AgentSteerPriority::parse(args.get("priority"))?;
        let steer_message = priority.format_message(&message);

        let (gcx, chat_facade) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.app.gcx.clone(), ccx_lock.app.chat.facade.clone())
        };

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        if card.column != "doing" {
            return Err(format!(
                "Card {} must be in 'doing' column to steer its agent; current column is '{}'.",
                card_id, card.column
            ));
        }
        let agent_chat_id = card.agent_chat_id.clone().ok_or_else(|| {
            format!(
                "Card {} has no agent_chat_id; no running agent to steer.",
                card_id
            )
        })?;

        let snapshot = chat_facade.session_snapshot(&agent_chat_id).await?;
        validate_agent_snapshot(&snapshot, &task_id, &card_id, card.assignee.as_deref())?;
        let session_state = snapshot.session_state;
        let compacted = if should_autocompact_before_steer(session_state, &snapshot.thread) {
            let mut messages = snapshot.messages.clone();
            let compacted = autocompact_messages_for_resume(&mut messages);
            if compacted {
                chat_facade
                    .update_session(
                        &agent_chat_id,
                        ChatSessionUpdate {
                            messages,
                            previous_response_id: None,
                        },
                    )
                    .await?;
            }
            compacted
        } else {
            false
        };

        let card_id_for_update = card_id.clone();
        let agent_chat_id_for_update = agent_chat_id.clone();
        let status_message = format!("Planner steered: {}", truncate_chars(&message, 80));
        let heartbeat = Utc::now().to_rfc3339();
        storage::update_board_atomic(gcx, &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            if card.column != "doing" {
                return Err(format!(
                    "Card {} must be in 'doing' column to steer its agent; current column is '{}'.",
                    card_id_for_update, card.column
                ));
            }
            if card.agent_chat_id.as_deref() != Some(agent_chat_id_for_update.as_str()) {
                return Err(format!(
                    "Card {} agent_chat_id changed before steering could be recorded.",
                    card_id_for_update
                ));
            }
            card.last_heartbeat_at = Some(heartbeat.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: heartbeat.clone(),
                message: status_message.clone(),
            });
            Ok(())
        })
        .await?;

        chat_facade
            .push_priority_command(
                &agent_chat_id,
                ChatCommand::UserMessage {
                    content: Value::String(steer_message.clone()),
                    attachments: vec![],
                    context_files: vec![],
                    origin: None,
                    suppress_auto_enrichment: false,
                },
            )
            .await?;

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(tool_output(
                    &card_id,
                    &message,
                    session_state,
                    compacted,
                )),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
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
    use refact_runtime_api::{
        ChatSessionFacade, ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest,
        RuntimeTrajectorySnapshot,
    };
    use std::sync::Mutex as StdMutex;

    struct MockChatFacade {
        state: StdMutex<SessionState>,
        pushed: StdMutex<Vec<(String, ChatCommand)>>,
        messages: StdMutex<Vec<ChatMessage>>,
        thread: StdMutex<refact_chat_api::ThreadParams>,
        updates: StdMutex<Vec<ChatSessionUpdate>>,
    }

    fn test_agent_thread() -> refact_chat_api::ThreadParams {
        refact_chat_api::ThreadParams {
            task_meta: Some(ThreadTaskMeta {
                task_id: "task-1".to_string(),
                role: "agents".to_string(),
                agent_id: Some("agent-1".to_string()),
                card_id: Some("T-29".to_string()),
                planner_chat_id: Some("planner".to_string()),
            }),
            ..Default::default()
        }
    }

    impl MockChatFacade {
        fn new(state: SessionState) -> Self {
            Self {
                state: StdMutex::new(state),
                pushed: StdMutex::new(vec![]),
                messages: StdMutex::new(vec![]),
                thread: StdMutex::new(refact_chat_api::ThreadParams {
                    auto_compact_enabled: Some(true),
                    ..test_agent_thread()
                }),
                updates: StdMutex::new(vec![]),
            }
        }

        fn with_messages(state: SessionState, messages: Vec<ChatMessage>) -> Self {
            Self {
                state: StdMutex::new(state),
                pushed: StdMutex::new(vec![]),
                messages: StdMutex::new(messages),
                thread: StdMutex::new(refact_chat_api::ThreadParams {
                    auto_compact_enabled: Some(true),
                    ..test_agent_thread()
                }),
                updates: StdMutex::new(vec![]),
            }
        }

        fn pushed_commands(&self) -> Vec<(String, ChatCommand)> {
            self.pushed.lock().unwrap().clone()
        }

        fn updates(&self) -> Vec<ChatSessionUpdate> {
            self.updates.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            Ok(ChatSessionSnapshot {
                messages: self.messages.lock().unwrap().clone(),
                thread: self.thread.lock().unwrap().clone(),
                session_state: *self.state.lock().unwrap(),
                pause_reasons: vec![],
            })
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            update: ChatSessionUpdate,
        ) -> Result<(), String> {
            *self.messages.lock().unwrap() = update.messages.clone();
            self.updates.lock().unwrap().push(update);
            Ok(())
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String> {
            self.pushed
                .lock()
                .unwrap()
                .push((chat_id.to_string(), command));
            Ok(())
        }

        async fn session_state(&self, _chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(Some(*self.state.lock().unwrap()))
        }

        async fn maybe_save_session(&self, _chat_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn save_trajectory_snapshot(
            &self,
            _snapshot: RuntimeTrajectorySnapshot,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    fn test_card(column: &str, agent_chat_id: Option<String>) -> BoardCard {
        BoardCard {
            id: "T-29".to_string(),
            title: "Steerable card".to_string(),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id,
            status_updates: vec![],
            comments: vec![],
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
            ab_variants: None,
            team_members: vec![],
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
            conductor: None,
        }
    }

    async fn write_task(
        root: &std::path::Path,
        card: BoardCard,
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
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        gcx
    }

    async fn planner_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        facade: Arc<dyn ChatSessionFacade>,
        role: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let mut app = AppState::from_gcx(gcx).await;
        app.chat.facade = facade;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                "planner-chat".to_string(),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: role.to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: None,
                }),
                None,
            )
            .await,
        ))
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn tool_output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[test]
    fn tool_agent_steer_formats_all_priorities() {
        assert_eq!(
            AgentSteerPriority::Info.format_message("heads up"),
            "[Planner FYI] heads up"
        );
        assert_eq!(
            AgentSteerPriority::Steer.format_message("add tests"),
            "[Planner STEER] add tests\n\nPlease adapt based on this and continue."
        );
        assert_eq!(
            AgentSteerPriority::Urgent.format_message("stop"),
            "[Planner URGENT] stop\n\nStop current action and address this."
        );
    }

    #[tokio::test]
    async fn tool_agent_steer_rejects_non_planner_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string())),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(SessionState::Idle));
        let ccx = planner_ccx(gcx, mock.clone(), "agents").await;
        let mut tool = ToolAgentSteer::new();

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("card_id", json!("T-29")), ("message", json!("add tests"))]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
        assert!(mock.pushed_commands().is_empty());
    }

    #[tokio::test]
    async fn tool_agent_steer_rejects_card_not_doing() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("planned", Some("agent-chat-1".to_string())),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(SessionState::Idle));
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;
        let mut tool = ToolAgentSteer::new();

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("card_id", json!("T-29")), ("message", json!("add tests"))]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("must be in 'doing' column"));
        assert!(mock.pushed_commands().is_empty());
    }

    #[tokio::test]
    async fn tool_agent_steer_rejects_missing_agent_chat_id() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), test_card("doing", None)).await;
        let mock = Arc::new(MockChatFacade::new(SessionState::Idle));
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;
        let mut tool = ToolAgentSteer::new();

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("card_id", json!("T-29")), ("message", json!("add tests"))]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("has no agent_chat_id"));
        assert!(mock.pushed_commands().is_empty());
    }

    #[tokio::test]
    async fn tool_agent_steer_pushes_user_message_and_updates_card() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string())),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(SessionState::ExecutingTools));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;
        let mut tool = ToolAgentSteer::new();

        let output = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-29")),
                    ("message", json!("Please add a test for empty filter list")),
                ]),
            )
            .await
            .unwrap(),
        );

        let pushed = mock.pushed_commands();
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].0, "agent-chat-1");
        match &pushed[0].1 {
            ChatCommand::UserMessage {
                content,
                attachments,
                context_files,
                origin,
                suppress_auto_enrichment,
            } => {
                assert_eq!(
                    content.as_str(),
                    Some(
                        "[Planner STEER] Please add a test for empty filter list\n\nPlease adapt based on this and continue."
                    )
                );
                assert!(attachments.is_empty());
                assert!(context_files.is_empty());
                assert_eq!(*origin, None);
                assert!(!suppress_auto_enrichment);
            }
            _ => panic!("expected user message"),
        }

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-29").unwrap();
        assert!(card.last_heartbeat_at.is_some());
        assert_eq!(card.status_updates.len(), 1);
        assert_eq!(
            card.status_updates[0].message,
            "Planner steered: Please add a test for empty filter list"
        );
        assert!(output.contains("✅ Steered T-29"));
        assert!(output.contains("⚙️ Executing tools"));
    }

    #[tokio::test]
    async fn tool_agent_steer_autocompacts_completed_agent_before_resume() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string())),
        )
        .await;
        let mut tool_result = ChatMessage::new("tool".to_string(), "x".repeat(80_000));
        tool_result.tool_call_id = "call-large".to_string();
        let messages = vec![
            ChatMessage::new("user".to_string(), "hi".to_string()),
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![refact_core::chat_types::ContextFile {
                    file_name: "foo.rs".to_string(),
                    file_content: "old".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![refact_core::chat_types::ContextFile {
                    file_name: "foo.rs".to_string(),
                    file_content: "new".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![refact_core::chat_types::ChatToolCall {
                    id: "call-large".to_string(),
                    tool_type: "function".to_string(),
                    function: refact_core::chat_types::ChatToolFunction {
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: Some(0),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            tool_result,
            ChatMessage::new("user".to_string(), "closed follow-up".to_string()),
        ];
        let mock = Arc::new(MockChatFacade::with_messages(
            SessionState::Completed,
            messages,
        ));
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;
        let mut tool = ToolAgentSteer::new();

        let output = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-29")),
                    ("message", json!("Continue now")),
                ]),
            )
            .await
            .unwrap(),
        );

        let updates = mock.updates();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].previous_response_id, None);
        assert!(updates[0]
            .messages
            .iter()
            .any(crate::chat::summarization::is_segment_summary));
        assert!(output.contains("Auto-compaction: applied before steering."));
        assert_eq!(mock.pushed_commands().len(), 1);
    }

    #[tokio::test]
    async fn tool_agent_steer_autocompacts_when_auto_compact_is_unset() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string())),
        )
        .await;
        let mut tool_result = ChatMessage::new("tool".to_string(), "x".repeat(40_000));
        tool_result.tool_call_id = "call-large".to_string();
        let messages = vec![
            ChatMessage::new("user".to_string(), "hi".to_string()),
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![refact_core::chat_types::ContextFile {
                    file_name: "foo.rs".to_string(),
                    file_content: "old".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![refact_core::chat_types::ContextFile {
                    file_name: "foo.rs".to_string(),
                    file_content: "new".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![refact_core::chat_types::ChatToolCall {
                    id: "call-large".to_string(),
                    tool_type: "function".to_string(),
                    function: refact_core::chat_types::ChatToolFunction {
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: Some(0),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            tool_result,
            ChatMessage::new("user".to_string(), "closed follow-up".to_string()),
        ];
        let mock = Arc::new(MockChatFacade::with_messages(
            SessionState::Completed,
            messages,
        ));
        *mock.thread.lock().unwrap() = refact_chat_api::ThreadParams {
            auto_compact_enabled: None,
            ..test_agent_thread()
        };
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;
        let mut tool = ToolAgentSteer::new();

        let output = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-29")),
                    ("message", json!("Continue now")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert_eq!(mock.updates().len(), 1);
        assert!(mock.updates()[0]
            .messages
            .iter()
            .any(crate::chat::summarization::is_segment_summary));
        assert!(output.contains("Auto-compaction: applied before steering."));
        assert_eq!(mock.pushed_commands().len(), 1);
    }

    #[tokio::test]
    async fn tool_agent_steer_skips_autocompaction_when_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string())),
        )
        .await;
        let mut tool_result = ChatMessage::new("tool".to_string(), "x".repeat(40_000));
        tool_result.tool_call_id = "call-large".to_string();
        let messages = vec![
            ChatMessage::new("user".to_string(), "hi".to_string()),
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![refact_core::chat_types::ContextFile {
                    file_name: "foo.rs".to_string(),
                    file_content: "old".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![refact_core::chat_types::ContextFile {
                    file_name: "foo.rs".to_string(),
                    file_content: "new".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![refact_core::chat_types::ChatToolCall {
                    id: "call-large".to_string(),
                    tool_type: "function".to_string(),
                    function: refact_core::chat_types::ChatToolFunction {
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: Some(0),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            tool_result,
        ];
        let mock = Arc::new(MockChatFacade::with_messages(
            SessionState::Completed,
            messages,
        ));
        *mock.thread.lock().unwrap() = refact_chat_api::ThreadParams {
            auto_compact_enabled: Some(false),
            ..test_agent_thread()
        };
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;
        let mut tool = ToolAgentSteer::new();

        let output = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-29")),
                    ("message", json!("Continue now")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(mock.updates().is_empty());
        assert!(!output.contains("Auto-compaction: applied before steering."));
        assert_eq!(mock.pushed_commands().len(), 1);
    }
}
